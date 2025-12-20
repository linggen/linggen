#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;
use std::time::Duration;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, RunEvent, WindowEvent,
};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

#[cfg(target_os = "macos")]
use tauri::ActivationPolicy;

#[cfg(target_os = "macos")]
fn kill_linggen_server_by_bundle_path() {
    use std::process::{Command, Stdio};
    
    // 1. Kill any process sitting on our port (8787)
    // Using absolute path for lsof as it might not be in PATH at startup
    let _ = Command::new("/usr/sbin/lsof")
        .args(["-t", "-i", "tcp:8787"])
        .output()
        .map(|output| {
            let pids = String::from_utf8_lossy(&output.stdout);
            for pid in pids.lines() {
                let _ = Command::new("kill").args(["-9", pid]).status();
            }
        });

    // 2. Kill any process that has the metadata file open
    if let Some(home) = dirs::home_dir() {
        let db_path = home.join("Library/Application Support/Linggen/metadata.redb");
        if db_path.exists() {
            if let Some(path_str) = db_path.to_str() {
                let _ = Command::new("/usr/sbin/lsof")
                    .args(["-t", path_str])
                    .output()
                    .map(|output| {
                        let pids = String::from_utf8_lossy(&output.stdout);
                        for pid in pids.lines() {
                            let _ = Command::new("kill").args(["-9", pid]).status();
                        }
                    });
            }
        }
    }

    // 3. Kill by name with various patterns
    let patterns = ["linggen-server", "linggen-server-aarch64", "linggen-server-x86_64"];
    for pattern in patterns {
        let _ = Command::new("/usr/bin/pkill")
            .args(["-9", "-f", pattern])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Global state to hold the backend child process and whether we started it
struct BackendProcess {
    child: Mutex<Option<CommandChild>>,
    /// If true, we started the backend and should kill it on exit
    /// If false, backend was already running (e.g., from IDE) and we should leave it alone
    managed_by_us: Mutex<bool>,
}

async fn ensure_backend_running(app_handle: AppHandle, force_restart: bool) {
    let backend_state = app_handle.state::<BackendProcess>();

    // If force restart, kill existing child first
    if force_restart {
        println!("[Tauri] Force restarting backend...");
        {
            let mut child_lock = backend_state.child.lock().unwrap();
            if let Some(child) = child_lock.take() {
                let _ = child.kill();
            }
        }
        #[cfg(target_os = "macos")]
        kill_linggen_server_by_bundle_path();
        // Give it a moment to release the port
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // First, check if backend is already running (e.g., from Cursor IDE or already started)
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .unwrap();

    let backend_already_running = if force_restart {
        false
    } else {
        match client.get("http://127.0.0.1:8787/api/status").send().await {
            Ok(response) if response.status().is_success() => {
                println!("[Tauri] Backend already running, will use it");
                true
            }
            _ => false,
        }
    };

    if !backend_already_running {
        // Backend not running, spawn the sidecar
        println!("[Tauri] Starting backend sidecar...");

        #[cfg(target_os = "macos")]
        {
            let is_dev = std::env::var("LINGGEN_DEV").is_ok() || cfg!(debug_assertions);
            if !is_dev {
                // If not in dev mode and not already running, do a quick cleanup of any
                // lingering/stuck processes before spawning.
                kill_linggen_server_by_bundle_path();
            }
        }

        let mut spawn_attempts = 0;
        let max_spawn_attempts = 2;

        while spawn_attempts < max_spawn_attempts {
            match app_handle.shell().sidecar("linggen-server") {
                Ok(sidecar_cmd) => {
                    let parent_pid = std::process::id().to_string();
                    let resource_dir = app_handle
                        .path()
                        .resource_dir()
                        .ok()
                        .and_then(|p| p.to_str().map(|s| s.to_string()));
                    let frontend_dir = resource_dir
                        .as_deref()
                        .map(|rd| format!("{}/resources/frontend", rd))
                        .filter(|p| std::path::Path::new(p).exists());

                    let mut sidecar_with_args =
                        sidecar_cmd.args(&["--port", "8787", "--parent-pid", &parent_pid]);
                    if let Some(fd) = frontend_dir.as_deref() {
                        sidecar_with_args =
                            sidecar_with_args.env("LINGGEN_FRONTEND_DIR", fd.to_string());
                    } else if let Some(rd) = resource_dir.as_deref() {
                        sidecar_with_args =
                            sidecar_with_args.env("TAURI_RESOURCE_DIR", rd.to_string());
                    }

                    match sidecar_with_args.spawn() {
                        Ok((mut rx, child)) => {
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
                                            eprintln!("[Backend Stderr] {}", String::from_utf8_lossy(&line))
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
                            // Successfully spawned
                            break;
                        }
                        Err(e) => {
                            spawn_attempts += 1;
                            eprintln!("[Tauri] Error: Failed to spawn backend sidecar (attempt {}/{}): {}", spawn_attempts, max_spawn_attempts, e);
                            if spawn_attempts < max_spawn_attempts {
                                tokio::time::sleep(Duration::from_secs(2)).await;
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[Tauri] Error: Failed to setup backend sidecar: {}", e);
                    break;
                }
            }
        }
    }

    // Wait for backend to be fully ready before showing window
    let app_handle_clone = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::new();
        let mut attempts = 0;
        let max_attempts = 120; // 60 seconds max wait (checking every 500ms)

        while attempts < max_attempts {
            match client.get("http://127.0.0.1:8787/api/status").send().await {
                Ok(response) if response.status().is_success() => {
                    if let Ok(text) = response.text().await {
                        if text.contains("\"status\":\"ready\"") {
                            println!("[Tauri] Backend is fully ready!");
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            if let Some(window) = app_handle_clone.get_webview_window("main") {
                                let _ = window.show();
                            }
                            return;
                        }
                    }
                }
                _ => {}
            }
            attempts += 1;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        eprintln!("[Tauri] Backend failed to become ready after 60 seconds");
        if let Some(window) = app_handle_clone.get_webview_window("main") {
            let _ = window.show();
        }
    });
}

fn main() {
    let is_dev = std::env::var("LINGGEN_DEV").is_ok() || cfg!(debug_assertions);

    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(BackendProcess {
            child: Mutex::new(None),
            managed_by_us: Mutex::new(false),
        })
        .setup(move |app| {
            let app_handle = app.handle().clone();

            // Spawn backend sidecar in the background
            let app_handle_for_setup = app_handle.clone();
            let is_dev_for_setup = is_dev;
            tauri::async_runtime::spawn(async move {
                // Add a small delay on macOS auto-launch to ensure system services are ready
                // Only do this in production to avoid slowing down development
                #[cfg(target_os = "macos")]
                if !is_dev_for_setup {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                
                ensure_backend_running(app_handle_for_setup, false).await;
            });

            // Create system tray icon with menu
            let show_item = MenuItem::with_id(app, "show", "Show Linggen", true, None::<&str>)?;
            let restart_backend_item = MenuItem::with_id(app, "restart_backend", "Restart Backend Server", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Linggen", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&show_item, &restart_backend_item, &quit_item])?;

            // Load tray icon (use the 32x32 icon for menu bar)
            let icon = Image::from_path("icons/32x32.png")
                .or_else(|_| Image::from_path("../icons/32x32.png"))
                .unwrap_or_else(|_| {
                    Image::from_bytes(include_bytes!("../icons/32x32.png"))
                        .expect("Failed to load embedded icon")
                });

            let app_handle_for_tray = app_handle.clone();
            TrayIconBuilder::new()
                .icon(icon)
                .menu(&tray_menu)
                .tooltip("Linggen - Running")
                .on_tray_icon_event(move |_tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(window) = app_handle_for_tray.get_webview_window("main") {
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
                    "restart_backend" => {
                        let app_handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            ensure_backend_running(app_handle, true).await;
                        });
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            println!("[Tauri] System tray icon created");

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match event {
                RunEvent::Reopen { .. } => {
                    #[cfg(target_os = "macos")]
                    let _ = app_handle.set_activation_policy(ActivationPolicy::Regular);
                    
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                RunEvent::ExitRequested { .. } | RunEvent::Exit => {
                    let backend_state = app_handle.state::<BackendProcess>();
                    let managed_by_us = *backend_state.managed_by_us.lock().unwrap();

                    if managed_by_us {
                        if let Some(child) = backend_state.child.lock().unwrap().take() {
                            println!("[Tauri] Terminating backend process...");
                            let _ = child.kill();
                        };
                    }

                    // In production, we also do a best-effort cleanup of any lingering 
                    // linggen-server processes to ensure the backend doesn't stay orphaned.
                    #[cfg(target_os = "macos")]
                    {
                        let is_dev = std::env::var("LINGGEN_DEV").is_ok() || cfg!(debug_assertions);
                        if !is_dev {
                            kill_linggen_server_by_bundle_path();
                        }
                    }
                }
                RunEvent::WindowEvent {
                    label,
                    event: WindowEvent::CloseRequested { api, .. },
                    ..
                } => {
                    api.prevent_close();
                    if let Some(window) = app_handle.get_webview_window(&label) {
                        let _ = window.hide();
                        #[cfg(target_os = "macos")]
                        let _ = app_handle.set_activation_policy(ActivationPolicy::Accessory);
                    }
                }
                _ => {}
            }
        });
}
