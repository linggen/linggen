use anyhow::{Context, Result};
use std::process::Command;

pub fn run_cmd(cmd: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(cmd)
        .args(args)
        .status()
        .with_context(|| format!("Failed to run {} {:?}", cmd, args))?;
    if !status.success() {
        anyhow::bail!("Command failed: {} {:?}", cmd, args);
    }
    Ok(())
}

pub fn run_and_capture_version(cmd: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(cmd).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}

pub fn command_exists(cmd: &str) -> bool {
    // 1. Check if it's in PATH
    if is_in_path(cmd) {
        return true;
    }

    // 2. On macOS, check the standard app bundle location for linggen-server
    #[cfg(target_os = "macos")]
    if cmd == "linggen-server" {
        if std::path::Path::new("/Applications/Linggen.app/Contents/MacOS/linggen-server").exists()
        {
            return true;
        }
    }

    false
}

/// Helper to find the linggen-server binary in common locations
pub fn find_server_binary() -> String {
    let base_name = "linggen-server";

    // 1. On macOS, prioritize the standard app bundle location.
    // This ensures we use the version the user likely just installed/updated,
    // rather than potentially stale binaries in /usr/local/bin or elsewhere.
    #[cfg(target_os = "macos")]
    {
        let mut bundle_paths = Vec::new();

        // 1. Prioritize Tauri sidecar paths (most likely to be the correct, fresh version in prod)
        let target_triple = match std::env::consts::ARCH {
            "aarch64" => "aarch64-apple-darwin",
            "x86_64" => "x86_64-apple-darwin",
            _ => "",
        };

        if !target_triple.is_empty() {
            // Check Resources/bin first (Tauri standard sidecar location)
            bundle_paths.push(format!(
                "/Applications/Linggen.app/Contents/Resources/bin/linggen-server-{}",
                target_triple
            ));
        }

        // 2. Fallback to standard names in bundle folders
        bundle_paths.push("/Applications/Linggen.app/Contents/MacOS/linggen-server".to_string());

        if std::env::consts::ARCH == "aarch64" {
            bundle_paths.push(
                "/Applications/Linggen.app/Contents/MacOS/linggen-server-aarch64-apple-darwin"
                    .to_string(),
            );
        } else {
            bundle_paths.push(
                "/Applications/Linggen.app/Contents/MacOS/linggen-server-x86_64-apple-darwin"
                    .to_string(),
            );
        }

        for path in bundle_paths {
            if std::path::Path::new(&path).exists() {
                return path;
            }
        }
    }

    // 2. Try PATH
    if is_in_path(base_name) {
        return base_name.to_string();
    }

    // 3. Check alongside the current executable
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            let alongside = parent.join(base_name);
            if alongside.exists() {
                return alongside.to_string_lossy().to_string();
            }
        }
    }

    // Fallback to just the name
    base_name.to_string()
}

/// Check if a binary exists in the system PATH
pub fn is_in_path(name: &str) -> bool {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            if dir.join(name).exists() {
                return true;
            }
        }
    }
    false
}

pub fn get_local_app_version() -> Option<String> {
    // 1. Try to read version from installed Linggen.app (macOS)
    #[cfg(target_os = "macos")]
    {
        use std::path::Path;
        let plist_path = Path::new("/Applications/Linggen.app/Contents/Info.plist");
        if plist_path.exists() {
            if let Ok(output) = Command::new("defaults")
                .args([
                    "read",
                    "/Applications/Linggen.app/Contents/Info",
                    "CFBundleShortVersionString",
                ])
                .output()
            {
                if output.status.success() {
                    return String::from_utf8(output.stdout)
                        .ok()
                        .map(|s| s.trim().to_string());
                }
            }
        }
    }

    // 2. Try to get version from linggen-server (Linux/macOS fallback)
    // On Linux, the "app" is the server.
    // Try both the bare command and absolute paths.
    let mut candidates = vec![
        "linggen-server".to_string(),
        "/usr/local/bin/linggen-server".to_string(),
        "/usr/bin/linggen-server".to_string(),
    ];

    #[cfg(target_os = "macos")]
    {
        candidates.push("/Applications/Linggen.app/Contents/MacOS/linggen-server".to_string());
    }

    for cmd in candidates {
        if let Some(raw) = run_and_capture_version(&cmd, &["--version"]) {
            // Expected: "linggen-server 0.5.1"
            for token in raw.split_whitespace() {
                let t = token.trim_start_matches('v');
                if !t.is_empty()
                    && t.chars().all(|c| c.is_ascii_digit() || c == '.')
                    && t.contains('.')
                {
                    return Some(t.to_string());
                }
            }
        }
    }

    None
}

pub fn format_timestamp(ts: &str) -> String {
    // Simple formatting - just show date and time without seconds
    if let Some(pos) = ts.find('T') {
        let date = &ts[..pos];
        let time = &ts[pos + 1..];
        if let Some(time_end) = time.find('.') {
            return format!("{} {}", date, &time[..time_end]);
        }
    }
    ts.to_string()
}
