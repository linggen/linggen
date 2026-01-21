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

enum BackendChild {
    Shell(CommandChild),
    Std(std::process::Child),
}

impl BackendChild {
    fn kill(self) {
        match self {
            BackendChild::Shell(child) => {
                let _ = child.kill();
            }
            BackendChild::Std(mut child) => {
                let _ = child.kill();
            }
        }
    }
}

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
                let _ = Command::new("/bin/kill").args(["-9", pid]).status();
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
                            let _ = Command::new("/bin/kill").args(["-9", pid]).status();
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

/// macOS GUI apps launched from Finder have a very restricted PATH.
/// This helper ensures common binary locations are included so that sidecars
/// and their dependencies (like curl) can be found.
#[cfg(target_os = "macos")]
fn fix_macos_gui_path() {
    let current_path = std::env::var("PATH").unwrap_or_default();
    let common_paths = "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin";
    
    if !current_path.contains("/usr/local/bin") || !current_path.contains("/opt/homebrew/bin") {
        let new_path = if current_path.is_empty() {
            common_paths.to_string()
        } else {
            format!("{}:{}", common_paths, current_path)
        };
        std::env::set_var("PATH", new_path);
    }
}

/// Helper to log to a file on disk when running as a GUI app.
/// Useful for diagnosing startup issues.
fn log_to_file(msg: &str) {
    if let Some(mut log_dir) = dirs::data_dir() {
        log_dir.push("Linggen");
        log_dir.push("logs");
        let _ = std::fs::create_dir_all(&log_dir);
        log_dir.push("tauri.log");
        
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir) 
        {
            use std::io::Write;
            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(file, "[{}] {}", timestamp, msg);
        }
    }
}

/// Global state to hold the backend child process and whether we started it
struct BackendProcess {
    child: Mutex<Option<BackendChild>>,
    /// If true, we started the backend and should kill it on exit
    /// If false, backend was already running (e.g., from IDE) and we should leave it alone
    managed_by_us: Mutex<bool>,
}

#[cfg(target_os = "macos")]
fn bundled_sidecar_path(app_handle: &AppHandle) -> Option<std::path::PathBuf> {
    // In Tauri 2.0, sidecars are located in the Resources/bin directory
    // and they have the target triple appended.
    let resource_dir = app_handle.path().resource_dir().ok()?;
    
    // We need the target triple. Since this is a build-time thing for the binary,
    // we can use a compile-time detection or just look for the file.
    // However, Tauri's sidecar name is "linggen-server".
    // For macOS, we usually have aarch64-apple-darwin or x86_64-apple-darwin.
    
    let patterns = [
        "linggen-server-aarch64-apple-darwin",
        "linggen-server-x86_64-apple-darwin",
        "linggen-server"
    ];

    for pattern in patterns {
        let p = resource_dir.join("bin").join(pattern);
        if p.exists() {
            return Some(p);
        }
        // Also check directly in bin/ (some older Tauri versions/configs)
        let p = resource_dir.join(pattern);
        if p.exists() {
            return Some(p);
        }
    }

    // Fallback to alongside executable (standard for non-Tauri sidecars)
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let p = dir.join("linggen-server");
    if p.exists() {
        return Some(p);
    }
    
    None
}

#[cfg(target_os = "macos")]
fn extracted_sidecar_path() -> Option<std::path::PathBuf> {
    let mut path = dirs::data_dir()?;
    path.push("Linggen");
    path.push("bin");
    let _ = std::fs::create_dir_all(&path);
    path.push("linggen-server");
    Some(path)
}

#[cfg(target_os = "macos")]
fn copy_sidecar_out_of_bundle_if_needed(app_handle: &AppHandle) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let src = bundled_sidecar_path(app_handle)?;
    let dest = extracted_sidecar_path()?;

    // Best-effort: if the extracted file exists and is newer or same size, reuse it.
    let needs_copy = match (std::fs::metadata(&src), std::fs::metadata(&dest)) {
        (Ok(src_meta), Ok(dest_meta)) => {
            src_meta.len() != dest_meta.len()
                || src_meta
                    .modified()
                    .ok()
                    .and_then(|sm| dest_meta.modified().ok().map(|dm| sm > dm))
                    .unwrap_or(true)
        }
        (Ok(_), Err(_)) => true,
        _ => true,
    };

    if needs_copy {
        let _ = std::fs::copy(&src, &dest);
        if let Ok(meta) = std::fs::metadata(&dest) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&dest, perms);
        }
    }

    Some(dest)
}

#[cfg(target_os = "macos")]
fn spawn_backend_via_std_process(
    program: &std::path::Path,
    parent_pid: &str,
    envs: Vec<(&str, String)>,
) -> std::io::Result<std::process::Child> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(program);
    cmd.arg("--port")
        .arg("8787")
        .arg("--parent-pid")
        .arg(parent_pid)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.spawn()
}

async fn ensure_backend_running(app_handle: AppHandle, force_restart: bool) {
    let backend_state = app_handle.state::<BackendProcess>();

    // If force restart, kill existing child first
    if force_restart {
        log_to_file("[Tauri] Force restarting backend...");
        {
            let mut child_lock = backend_state.child.lock().unwrap();
            if let Some(child) = child_lock.take() {
                child.kill();
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
                let app_version = app_handle.package_info().version.to_string();
                if let Ok(status) = response.json::<serde_json::Value>().await {
                    let backend_version = status["version"].as_str().unwrap_or("");
                    if backend_version == app_version {
                        log_to_file(&format!(
                            "[Tauri] Backend already running with correct version ({}), will use it",
                            backend_version
                        ));
                        true
                    } else {
                        log_to_file(&format!(
                            "[Tauri] Backend running but version mismatch (backend: {}, app: {}). Will force restart.",
                            backend_version, app_version
                        ));
                        false
                    }
                } else {
                    log_to_file("[Tauri] Backend already running but could not parse status. Will force restart.");
                    false
                }
            }
            _ => false,
        }
    };

    if !backend_already_running {
        // If we found an old version, kill it before spawning new one
        #[cfg(target_os = "macos")]
        {
            log_to_file("[Tauri] Killing any legacy linggen-server processes...");
            kill_linggen_server_by_bundle_path();
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        // Useful diagnostics for Finder-launched apps.
        #[cfg(target_os = "macos")]
        {
            let exe = std::env::current_exe()
                .ok()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "<unknown>".to_string());
            let resource_dir = app_handle
                .path()
                .resource_dir()
                .ok()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "<unknown>".to_string());
            let path_env = std::env::var("PATH").unwrap_or_else(|_| "<unset>".to_string());
            log_to_file(&format!("[Tauri] Diagnostics: exe={}", exe));
            log_to_file(&format!("[Tauri] Diagnostics: resource_dir={}", resource_dir));
            log_to_file(&format!("[Tauri] Diagnostics: PATH={}", path_env));
            if let Some(p) = bundled_sidecar_path(&app_handle) {
                log_to_file(&format!(
                    "[Tauri] Diagnostics: bundled_sidecar={} exists={}",
                    p.display(),
                    p.exists()
                ));
            }
        }

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
            // Try matching the sidecar name exactly as defined in tauri.conf.json
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
                    let mut envs_for_fallback: Vec<(&str, String)> = Vec::new();
                    if let Some(fd) = frontend_dir.as_deref() {
                        sidecar_with_args =
                            sidecar_with_args.env("LINGGEN_FRONTEND_DIR", fd.to_string());
                        envs_for_fallback.push(("LINGGEN_FRONTEND_DIR", fd.to_string()));
                    } else if let Some(rd) = resource_dir.as_deref() {
                        sidecar_with_args =
                            sidecar_with_args.env("TAURI_RESOURCE_DIR", rd.to_string());
                        envs_for_fallback.push(("TAURI_RESOURCE_DIR", rd.to_string()));
                    }

                    log_to_file(&format!("[Tauri] Spawning sidecar with PID {}", parent_pid));

                    match sidecar_with_args.spawn() {
                        Ok((mut rx, child)) => {
                            log_to_file("[Tauri] Sidecar spawned successfully");
                            *backend_state.child.lock().unwrap() = Some(BackendChild::Shell(child));
                            *backend_state.managed_by_us.lock().unwrap() = true;

                            // Handle sidecar events in a separate task to log output
                            tauri::async_runtime::spawn(async move {
                                while let Some(event) = rx.recv().await {
                                    match event {
                                        CommandEvent::Stdout(line) => {
                                            let msg = format!("[Backend] {}", String::from_utf8_lossy(&line));
                                            log_to_file(&msg);
                                        }
                                        CommandEvent::Stderr(line) => {
                                            let msg = format!("[Backend Stderr] {}", String::from_utf8_lossy(&line));
                                            log_to_file(&msg);
                                        }
                                        CommandEvent::Error(err) => {
                                            let msg = format!("[Backend Error] {}", err);
                                            log_to_file(&msg);
                                        }
                                        CommandEvent::Terminated(payload) => {
                                            let msg = format!(
                                                "[Backend] Process terminated with code: {:?}, signal: {:?}",
                                                payload.code, payload.signal
                                            );
                                            log_to_file(&msg);
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
                            let err_msg = format!("[Tauri] Error: Failed to spawn backend sidecar (attempt {}/{}): {}", spawn_attempts, max_spawn_attempts, e);
                            log_to_file(&err_msg);

                            // macOS: Finder-launched apps can hit permission/quarantine issues for embedded binaries.
                            // Fallback: copy the sidecar out of the app bundle and launch it from Application Support.
                            #[cfg(target_os = "macos")]
                            {
                                let is_dev = std::env::var("LINGGEN_DEV").is_ok() || cfg!(debug_assertions);
                                if !is_dev {
                                    if let Some(extracted) = copy_sidecar_out_of_bundle_if_needed(&app_handle) {
                                        log_to_file(&format!(
                                            "[Tauri] Attempting fallback spawn from extracted sidecar: {}",
                                            extracted.display()
                                        ));
                                        match spawn_backend_via_std_process(&extracted, &parent_pid, envs_for_fallback.clone()) {
                                            Ok(mut child) => {
                                                log_to_file("[Tauri] Fallback sidecar spawned successfully");

                                                // Pipe stdout/stderr to our log file.
                                                if let Some(mut out) = child.stdout.take() {
                                                    tauri::async_runtime::spawn(async move {
                                                        use std::io::Read;
                                                        let mut buf = [0u8; 4096];
                                                        loop {
                                                            match out.read(&mut buf) {
                                                                Ok(0) => break,
                                                                Ok(n) => {
                                                                    let msg = format!("[Backend] {}", String::from_utf8_lossy(&buf[..n]));
                                                                    log_to_file(&msg);
                                                                }
                                                                Err(_) => break,
                                                            }
                                                        }
                                                    });
                                                }
                                                if let Some(mut errp) = child.stderr.take() {
                                                    tauri::async_runtime::spawn(async move {
                                                        use std::io::Read;
                                                        let mut buf = [0u8; 4096];
                                                        loop {
                                                            match errp.read(&mut buf) {
                                                                Ok(0) => break,
                                                                Ok(n) => {
                                                                    let msg = format!("[Backend Stderr] {}", String::from_utf8_lossy(&buf[..n]));
                                                                    log_to_file(&msg);
                                                                }
                                                                Err(_) => break,
                                                            }
                                                        }
                                                    });
                                                }

                                                *backend_state.child.lock().unwrap() = Some(BackendChild::Std(child));
                                                *backend_state.managed_by_us.lock().unwrap() = true;
                                                break;
                                            }
                                            Err(e2) => {
                                                log_to_file(&format!(
                                                    "[Tauri] Error: fallback spawn failed: {}",
                                                    e2
                                                ));
                                            }
                                        }
                                    } else {
                                        log_to_file("[Tauri] Fallback spawn skipped: could not locate/copy sidecar");
                                    }
                                }
                            }

                            if spawn_attempts < max_spawn_attempts {
                                tokio::time::sleep(Duration::from_secs(2)).await;
                            }
                        }
                    }
                }
                Err(e) => {
                    let err_msg = format!("[Tauri] Error: Failed to setup backend sidecar: {}", e);
                    log_to_file(&err_msg);
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
                            log_to_file("[Tauri] Backend is fully ready!");
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            if let Some(window) = app_handle_clone.get_webview_window("main") {
                                #[cfg(target_os = "macos")]
                                let _ = app_handle_clone.set_activation_policy(tauri::ActivationPolicy::Regular);
                                
                                let _ = window.show();
                                let _ = window.set_focus();
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

        log_to_file("[Tauri] Backend failed to become ready after 60 seconds");
        if let Some(window) = app_handle_clone.get_webview_window("main") {
            let _ = window.show();
        }
    });
}

fn main() {
    #[cfg(target_os = "macos")]
    fix_macos_gui_path();

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
                            child.kill();
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
