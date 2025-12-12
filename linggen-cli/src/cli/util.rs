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
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {}", cmd))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn get_local_app_version() -> Option<String> {
    // Try to read version from installed Linggen.app (macOS)
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
