#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;
use std::time::Duration;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, RunEvent, WindowEvent,
};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

#[cfg(target_os = "macos")]
use tauri::ActivationPolicy;

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

                match app_handle.shell().sidecar("linggen") {
                    Ok(sidecar_cmd) => {
                        // Run in server mode
                        let sidecar_with_args = sidecar_cmd.args(&["serve"]);
                        match sidecar_with_args.spawn() {
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
                        }
                    }
                    Err(e) => {
                        eprintln!("[Tauri] Warning: Failed to setup backend sidecar: {}", e);
                        eprintln!("[Tauri] Assuming backend is running separately...");
                    }
                }
            }

            // Create system tray icon with menu
            let show_item = MenuItem::with_id(app, "show", "Show Linggen", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Linggen", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            // Load tray icon (use the 32x32 icon for menu bar)
            let icon = Image::from_path("icons/32x32.png")
                .or_else(|_| Image::from_path("../icons/32x32.png"))
                .unwrap_or_else(|_| {
                    // Fallback: use embedded icon bytes
                    Image::from_bytes(include_bytes!("../icons/32x32.png"))
                        .expect("Failed to load embedded icon")
                });

            let app_handle_for_tray = app_handle.clone();
            TrayIconBuilder::new()
                .icon(icon)
                .menu(&tray_menu)
                .tooltip("Linggen - Running")
                .on_tray_icon_event(move |_tray, event| {
                    // Left click on tray icon shows the window
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(window) = app_handle_for_tray.get_webview_window("main") {
                            // Show in Dock again on macOS
                            #[cfg(target_os = "macos")]
                            let _ = app_handle_for_tray.set_activation_policy(ActivationPolicy::Regular);
                            
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            #[cfg(target_os = "macos")]
                            let _ = app.set_activation_policy(ActivationPolicy::Regular);
                            
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        // This will trigger RunEvent::ExitRequested
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            println!("[Tauri] System tray icon created");

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
                // macOS: clicking Dock icon when app is running but window is hidden
                RunEvent::Reopen { .. } => {
                    println!("[Tauri] Reopen event - showing main window");
                    
                    // Show in Dock again
                    #[cfg(target_os = "macos")]
                    let _ = app_handle.set_activation_policy(ActivationPolicy::Regular);
                    
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
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
                    label,
                    event: WindowEvent::CloseRequested { api, .. },
                    ..
                } => {
                    // Docker Desktop–style behavior:
                    // - Prevent the window from actually closing
                    // - Just hide it so the app (and backend) keep running
                    // - Hide from Dock, show only in menu bar
                    api.prevent_close();

                    if let Some(window) = app_handle.get_webview_window(&label) {
                        let _ = window.hide();
                        
                        // Hide from Dock on macOS (app becomes menu bar only)
                        #[cfg(target_os = "macos")]
                        let _ = app_handle.set_activation_policy(ActivationPolicy::Accessory);
                        
                        println!("[Tauri] Window hidden - click menu bar icon to reopen");
                    }
                }
                _ => {}
            }
        });
}
