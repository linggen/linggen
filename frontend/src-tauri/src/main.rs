#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;
use std::time::Duration;
use tauri::{Manager, RunEvent, WindowEvent};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

/// Global state to hold the backend child process and whether we started it
struct BackendProcess {
    child: Mutex<Option<CommandChild>>,
    /// If true, we started the backend and should kill it on exit
    /// If false, backend was already running (e.g., from IDE) and we should leave it alone
    managed_by_us: Mutex<bool>,
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(BackendProcess {
            child: Mutex::new(None),
            managed_by_us: Mutex::new(false),
        })
        .setup(|app| {
            let app_handle = app.handle().clone();

            // First, check if backend is already running (e.g., from Cursor IDE)
            let backend_already_running = tauri::async_runtime::block_on(async {
                let client = reqwest::Client::builder()
                    .timeout(Duration::from_millis(500))
                    .build()
                    .unwrap();

                match client.get("http://127.0.0.1:8787/api/status").send().await {
                    Ok(response) if response.status().is_success() => {
                        println!("[Tauri] Backend already running (e.g., from IDE), will use it");
                        true
                    }
                    _ => false,
                }
            });

            if !backend_already_running {
                // Backend not running, spawn the sidecar
                println!("[Tauri] Starting backend sidecar...");

                match app_handle.shell().sidecar("linggen-backend") {
                    Ok(sidecar_cmd) => match sidecar_cmd.spawn() {
                        Ok((mut rx, child)) => {
                            // Store the child process handle for cleanup
                            let backend_state = app_handle.state::<BackendProcess>();
                            *backend_state.child.lock().unwrap() = Some(child);
                            *backend_state.managed_by_us.lock().unwrap() = true;

                            // Handle sidecar events in a separate task to log output
                            tauri::async_runtime::spawn(async move {
                                while let Some(event) = rx.recv().await {
                                    match event {
                                        CommandEvent::Stdout(line) => {
                                            println!("[Backend] {}", String::from_utf8_lossy(&line))
                                        }
                                        CommandEvent::Stderr(line) => {
                                            eprintln!("[Backend] {}", String::from_utf8_lossy(&line))
                                        }
                                        CommandEvent::Error(err) => {
                                            eprintln!("[Backend Error] {}", err)
                                        }
                                        CommandEvent::Terminated(payload) => {
                                            println!(
                                                "[Backend] Process terminated with code: {:?}, signal: {:?}",
                                                payload.code, payload.signal
                                            );
                                        }
                                        _ => {}
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            eprintln!("[Tauri] Warning: Failed to spawn backend sidecar: {}", e);
                            eprintln!("[Tauri] Assuming backend is running separately...");
                        }
                    },
                    Err(e) => {
                        eprintln!("[Tauri] Warning: Failed to setup backend sidecar: {}", e);
                        eprintln!("[Tauri] Assuming backend is running separately...");
                    }
                }
            }

            // Wait for backend to be fully ready before showing window
            let app_handle_clone = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                // Poll the backend health endpoint until status is "ready"
                let client = reqwest::Client::new();
                let mut attempts = 0;
                let max_attempts = 60; // 60 seconds max wait (embedding model can take time)

                while attempts < max_attempts {
                    match client.get("http://127.0.0.1:8787/api/status").send().await {
                        Ok(response) if response.status().is_success() => {
                            // Parse the response to check actual status
                            if let Ok(text) = response.text().await {
                                if text.contains("\"status\":\"ready\"") {
                                    println!("[Tauri] Backend is fully ready!");
                                    // Small delay to ensure all services are stable
                                    tokio::time::sleep(Duration::from_millis(500)).await;
                                    // Show the main window
                                    if let Some(window) = app_handle_clone.get_webview_window("main") {
                                        let _ = window.show();
                                    }
                                    return;
                                } else {
                                    println!("[Tauri] Backend responded but not ready yet: {}", text);
                                }
                            }
                        }
                        Err(e) => {
                            println!("[Tauri] Waiting for backend... ({})", e);
                        }
                        _ => {}
                    }
                    attempts += 1;
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }

                eprintln!(
                    "[Tauri] Backend failed to become ready after {} seconds",
                    max_attempts / 2
                );
                // Still show the window even if backend didn't respond
                if let Some(window) = app_handle_clone.get_webview_window("main") {
                    let _ = window.show();
                }
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match event {
                RunEvent::ExitRequested { .. } | RunEvent::Exit => {
                    // Only terminate backend if we started it
                    let backend_state = app_handle.state::<BackendProcess>();
                    let managed_by_us = *backend_state.managed_by_us.lock().unwrap();

                    if managed_by_us {
                        if let Some(child) = backend_state.child.lock().unwrap().take() {
                            println!("[Tauri] Terminating backend process (we started it)...");
                            let _ = child.kill();
                        };
                    } else {
                        println!("[Tauri] Leaving backend running (started externally)");
                    }
                }
                RunEvent::WindowEvent {
                    event: WindowEvent::CloseRequested { .. },
                    ..
                } => {
                    // Only terminate backend if we started it
                    let backend_state = app_handle.state::<BackendProcess>();
                    let managed_by_us = *backend_state.managed_by_us.lock().unwrap();

                    if managed_by_us {
                        if let Some(child) = backend_state.child.lock().unwrap().take() {
                            println!("[Tauri] Terminating backend on window close (we started it)...");
                            let _ = child.kill();
                        };
                    }
                }
                _ => {}
            }
        });
}
