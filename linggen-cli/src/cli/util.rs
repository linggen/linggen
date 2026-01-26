use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
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

    // 2. On macOS, check the user-local install location
    #[cfg(target_os = "macos")]
    if cmd == "linggen-server" {
        if let Some(home) = dirs::home_dir() {
            if home
                .join("Library/Application Support/Linggen/bin/linggen-server")
                .exists()
            {
                return true;
            }
        }
    }

    false
}

/// Helper to find the linggen-server binary in common locations
pub fn find_server_binary() -> String {
    let base_name = "linggen-server";

    // 1. On macOS, prioritize the user-local install location.
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            let user_local = home.join("Library/Application Support/Linggen/bin/linggen-server");
            if user_local.exists() {
                return user_local.to_string_lossy().to_string();
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
    // Try to get version from linggen-server
    // Try both the bare command and absolute paths.
    let mut candidates = vec![
        "linggen-server".to_string(),
        "/usr/local/bin/linggen-server".to_string(),
        "/usr/bin/linggen-server".to_string(),
    ];

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            candidates.push(
                home.join("Library/Application Support/Linggen/bin/linggen-server")
                    .to_string_lossy()
                    .to_string(),
            );
        }
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
