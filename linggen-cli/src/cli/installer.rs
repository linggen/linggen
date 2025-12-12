use anyhow::{Context, Result};
use colored::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use tempfile::{tempdir, NamedTempFile};

use crate::manifest::{select_artifact, ArtifactKind, Platform};

use super::download::{default_http_client, download_to_temp};
use super::util::{command_exists, run_cmd};

pub async fn install_macos(manifest: &crate::manifest::Manifest) -> Result<()> {
    let client = default_http_client()?;

    // Update CLI first (self-update) so `linggen update` actually updates itself.
    if let Some(cli_art) = select_artifact(manifest, Platform::Mac, ArtifactKind::Cli) {
        println!("{}", "⬇️  Downloading CLI tarball...".cyan());
        let tar = download_to_temp(&client, &cli_art.url, cli_art.signature.as_deref()).await?;
        install_cli_from_tarball(&tar)?;
        println!("{}", "✅ Updated CLI".green());
    } else {
        println!(
            "{}",
            "⚠️ No macOS CLI artifact in manifest; skipping CLI update.".yellow()
        );
    }

    // Install/update app bundle to /Applications from tarball
    // (server is bundled in the app)
    if let Some(app_art) = select_artifact(manifest, Platform::Mac, ArtifactKind::App) {
        println!("{}", "⬇️  Downloading Linggen app tarball...".cyan());
        let tar = download_to_temp(&client, &app_art.url, app_art.signature.as_deref()).await?;
        install_app_bundle(&tar)?;
    } else {
        println!(
            "{}",
            "⚠️ No macOS app artifact in manifest; cannot install.".yellow()
        );
        anyhow::bail!("No macOS app artifact found in manifest");
    }

    Ok(())
}

pub async fn install_linux(manifest: &crate::manifest::Manifest) -> Result<()> {
    if command_exists("apt") {
        run_cmd("sudo", &["apt", "update"])?;
        run_cmd("sudo", &["apt", "install", "-y", "linggen"])?;
        run_cmd("sudo", &["apt", "install", "-y", "linggen-server"])?;
        println!(
            "{}",
            "✅ Installed/updated linggen CLI and linggen-server via APT".green()
        );
        return Ok(());
    }

    // Fallback: tarball install from manifest
    let client = default_http_client()?;
    if let Some(cli_art) = select_artifact(manifest, Platform::Linux, ArtifactKind::Cli) {
        println!("{}", "⬇️  Downloading CLI tarball...".cyan());
        let tar = download_to_temp(&client, &cli_art.url, cli_art.signature.as_deref()).await?;
        extract_tarball(&tar, "/usr/local/bin")?;
        println!("{}", "✅ Installed/updated CLI from tarball".green());
    } else {
        println!("{}", "⚠️ No CLI tarball found in manifest".yellow());
    }

    if let Some(srv_art) = select_artifact(manifest, Platform::Linux, ArtifactKind::Server) {
        println!("{}", "⬇️  Downloading server tarball...".cyan());
        let tar = download_to_temp(&client, &srv_art.url, srv_art.signature.as_deref()).await?;
        extract_tarball(&tar, "/usr/local/bin")?;
        install_systemd_unit()?;
        println!("{}", "✅ Installed/updated server from tarball".green());
    } else {
        println!("{}", "⚠️ No server tarball found in manifest".yellow());
    }

    Ok(())
}

fn install_cli_from_tarball(tar_path: &PathBuf) -> Result<()> {
    let tmp_dir = tempdir().context("Failed to create temp dir for CLI extraction")?;
    let tmp_path = tmp_dir.path();

    let dest_str = tmp_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid temp dir path"))?;
    extract_tarball(tar_path, dest_str)?;

    // Expecting `linggen` at the tar root
    let extracted = tmp_path.join("linggen");
    if !extracted.exists() {
        anyhow::bail!("No `linggen` binary found in extracted tarball");
    }

    // Prefer replacing the currently running executable path (self-update).
    let mut tried = Vec::<String>::new();
    if let Ok(current_exe) = std::env::current_exe() {
        if current_exe.file_name().and_then(|n| n.to_str()) == Some("linggen") {
            if try_replace_binary(&extracted, &current_exe)? {
                return Ok(());
            }
            tried.push(current_exe.display().to_string());
        }
    }

    // Fallback to common install locations.
    // - Intel macs often use /usr/local/bin
    // - Apple Silicon Homebrew often uses /opt/homebrew/bin
    for candidate in ["/usr/local/bin/linggen", "/opt/homebrew/bin/linggen"] {
        let dest = Path::new(candidate);
        if try_replace_binary(&extracted, dest)? {
            return Ok(());
        }
        tried.push(candidate.to_string());
    }

    anyhow::bail!(
        "Failed to install CLI. Tried: {} (permission may be required)",
        tried.join(", ")
    );
}

fn try_replace_binary(src: &Path, dest: &Path) -> Result<bool> {
    // Try a plain copy first (works when dest is user-writable).
    if let Some(parent) = dest.parent() {
        if parent.exists() {
            // Copy to a temp file in the same directory then rename for atomic replacement.
            let tmp_name = format!(
                ".linggen.tmp.{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            );
            let tmp_dest = parent.join(tmp_name);

            // Ensure permissions on the new binary are executable.
            fs::copy(src, &tmp_dest).with_context(|| {
                format!("Failed to copy {} to {}", src.display(), tmp_dest.display())
            })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&tmp_dest)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&tmp_dest, perms)?;
            }

            // Attempt atomic swap.
            if fs::rename(&tmp_dest, dest).is_ok() {
                return Ok(true);
            }

            // Cleanup temp file if rename failed.
            let _ = fs::remove_file(&tmp_dest);
        }
    }

    // If direct replace failed, try sudo (will fail in GUI contexts; succeeds in terminal).
    let (Some(src_str), Some(dest_str)) = (src.to_str(), dest.to_str()) else {
        return Ok(false);
    };
    let sudo_result = Command::new("sudo")
        .args(["cp", src_str, dest_str])
        .status();
    if sudo_result.as_ref().map(|s| s.success()).unwrap_or(false) {
        let _ = Command::new("sudo")
            .args(["chmod", "755", dest_str])
            .status();
        return Ok(true);
    }

    Ok(false)
}

fn extract_tarball(tar_path: &PathBuf, dest_dir: &str) -> Result<()> {
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(tar_path)
        .arg("-C")
        .arg(dest_dir)
        // Some environments set `TAR_OPTIONS` (e.g. including `-O`) which can cause tar to dump
        // extracted *file contents* to stdout (binary garbage in terminal). Ignore it.
        .env_remove("TAR_OPTIONS")
        // Never allow tar to spam stdout; keep stderr for actionable errors.
        .stdout(Stdio::null())
        .status()
        .with_context(|| format!("Failed to run tar on {:?}", tar_path))?;
    if !status.success() {
        anyhow::bail!("tar failed for {:?}", tar_path);
    }
    Ok(())
}

fn install_app_bundle(tar_path: &PathBuf) -> Result<()> {
    let tmp_dir = tempdir().context("Failed to create temp dir for app extraction")?;
    let tmp_path = tmp_dir.path();

    let dest_str = tmp_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid temp dir path"))?;
    extract_tarball(tar_path, dest_str)?;

    // Locate the .app bundle (expecting Linggen.app at the tar root)
    let mut app_bundle = tmp_path.join("Linggen.app");
    if !app_bundle.exists() {
        app_bundle = std::fs::read_dir(tmp_path)
            .context("Failed to read extracted app directory")?
            .filter_map(|e| e.ok().map(|entry| entry.path()))
            .find(|p| p.is_dir() && p.extension().map(|ext| ext == "app").unwrap_or(false))
            .ok_or_else(|| anyhow::anyhow!("No .app bundle found in extracted tarball"))?;
    }

    let app_src = app_bundle
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid app bundle path"))?;

    let applications_dir = Path::new("/Applications");
    if !applications_dir.exists() {
        fs::create_dir_all(applications_dir).context("Failed to create /Applications")?;
    }

    let dest_app = applications_dir.join("Linggen.app");
    if dest_app.exists() {
        fs::remove_dir_all(&dest_app)
            .context("Failed to remove existing /Applications/Linggen.app")?;
    }

    // Copy the bundle; retry with sudo if permissions fail
    let copy_result = Command::new("cp")
        .args(["-R", app_src, "/Applications/"])
        .status();

    let mut success = copy_result.as_ref().map(|s| s.success()).unwrap_or(false);
    if !success {
        println!(
            "{}",
            "⚠️ Copy to /Applications failed; retrying with sudo (may prompt for password)..."
                .yellow()
        );
        let sudo_result = Command::new("sudo")
            .args(["cp", "-R", app_src, "/Applications/"])
            .status();
        success = sudo_result.as_ref().map(|s| s.success()).unwrap_or(false);
    }

    if !success {
        anyhow::bail!("Failed to copy app bundle to /Applications");
    }

    println!(
        "{}",
        "✅ Installed/updated Linggen.app to /Applications".green()
    );

    // Check if app is running and restart it
    if is_app_running() {
        println!(
            "{}",
            "🔄 Linggen app is currently running. Restarting to use new version...".cyan()
        );
        restart_app()?;
    } else {
        println!(
            "{}",
            "ℹ️  Restart Linggen app to use the new version".yellow()
        );
    }

    Ok(())
}

/// Check if Linggen app is currently running
fn is_app_running() -> bool {
    // Check for running Linggen process
    // On macOS, the app process name is typically the bundle name
    let output = Command::new("pgrep").arg("-f").arg("Linggen.app").output();

    if let Ok(output) = output {
        return output.status.success();
    }

    // Fallback: check with ps
    let output = Command::new("ps").args(["-ax"]).output();

    if let Ok(output) = output {
        if let Ok(stdout) = String::from_utf8(output.stdout) {
            return stdout.contains("Linggen.app");
        }
    }

    false
}

/// Restart the Linggen app by quitting the old instance and launching the new one
fn restart_app() -> Result<()> {
    // Quit the running app - try multiple process names
    // The actual process name is "linggen-desktop" but killall might need "Linggen"
    let mut quit_success = false;

    // Prefer graceful quit so the app can clean up its sidecar backend.
    // (Force-killing can orphan the backend process on macOS.)
    let osa = Command::new("osascript")
        .args(["-e", "tell application \"Linggen\" to quit"])
        .output();
    if osa.as_ref().map(|o| o.status.success()).unwrap_or(false) {
        quit_success = true;
    }

    // Try killing by bundle name first (most reliable on macOS)
    let killall_result = Command::new("killall").arg("Linggen").output();

    if killall_result.is_ok() && killall_result.as_ref().unwrap().status.success() {
        quit_success = true;
    } else {
        // Try killing by actual process name
        let killall_desktop = Command::new("killall").arg("linggen-desktop").output();

        if killall_desktop.is_ok() && killall_desktop.as_ref().unwrap().status.success() {
            quit_success = true;
        }
    }

    // Wait longer for the app to fully quit (especially if it needs to save state)
    // Check if process is still running and wait up to 3 seconds
    for _ in 0..6 {
        std::thread::sleep(Duration::from_millis(500));

        // Check if app is still running
        let check_result = Command::new("pgrep").arg("-f").arg("Linggen.app").output();

        if let Ok(output) = check_result {
            if !output.status.success() {
                // Process is gone, we can proceed
                break;
            }
        }
    }

    // Best-effort cleanup: if a previous version orphaned the sidecar backend, stop it.
    // We only target the server binary inside the app bundle to avoid killing dev backends.
    let _ = Command::new("pkill")
        .args([
            "-f",
            "/Applications/Linggen.app/Contents/MacOS/linggen-server",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // Launch the new version
    let open_result = Command::new("open")
        .arg("/Applications/Linggen.app")
        .status();

    if let Ok(status) = open_result {
        if status.success() {
            println!("{}", "✅ Linggen app restarted successfully".green());
            return Ok(());
        }
    }

    // If open failed, at least we tried to quit it
    if quit_success {
        println!(
            "{}",
            "⚠️  App was quit, but failed to auto-launch. Please start Linggen.app manually."
                .yellow()
        );
    } else {
        println!(
            "{}",
            "⚠️  Could not restart app automatically. Please quit and restart Linggen.app manually."
                .yellow()
        );
    }

    Ok(())
}

fn install_systemd_unit() -> Result<()> {
    let unit = r#"[Unit]
Description=Linggen Server
After=network.target

[Service]
ExecStart=/usr/local/bin/linggen-server
Restart=on-failure

[Install]
WantedBy=multi-user.target
"#;
    let tmp = NamedTempFile::new().context("Failed to create temp file for unit")?;
    fs::write(tmp.path(), unit)?;

    let tmp_str = tmp
        .path()
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("invalid temp path"))?
        .to_string();

    run_cmd(
        "sudo",
        &["cp", &tmp_str, "/etc/systemd/system/linggen-server.service"],
    )?;
    run_cmd("sudo", &["systemctl", "daemon-reload"])?;
    run_cmd("sudo", &["systemctl", "enable", "--now", "linggen-server"])?;
    println!(
        "{}",
        "✅ systemd unit installed/enabled (linggen-server)".green()
    );
    Ok(())
}
