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
use super::util::run_cmd;

fn resolve_human_home_dir() -> Result<PathBuf> {
    // Normal case: not running under sudo/root.
    #[cfg(unix)]
    {
        if !is_root_unix() {
            return dirs::home_dir()
                .map(|p| p.to_path_buf())
                .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"));
        }

        // If we're root (e.g. `sudo linggen install`), prefer the invoking human user's home.
        let sudo_user = std::env::var("SUDO_USER").unwrap_or_default();
        if !sudo_user.trim().is_empty() && sudo_user != "root" {
            // macOS: prefer dscl; fallback to /Users/<name>
            #[cfg(target_os = "macos")]
            {
                if let Ok(out) = Command::new("dscl")
                    .args([
                        ".",
                        "-read",
                        &format!("/Users/{}", sudo_user),
                        "NFSHomeDirectory",
                    ])
                    .output()
                {
                    if out.status.success() {
                        if let Ok(s) = String::from_utf8(out.stdout) {
                            // Example: "NFSHomeDirectory: /Users/lianghuang\n"
                            if let Some(path) = s.split(':').nth(1).map(|v| v.trim()) {
                                if !path.is_empty() {
                                    return Ok(PathBuf::from(path));
                                }
                            }
                        }
                    }
                }
                return Ok(PathBuf::from("/Users").join(sudo_user));
            }

            // Linux/other Unix: try passwd DB via getent; fallback to /home/<name>
            #[cfg(not(target_os = "macos"))]
            {
                if let Ok(out) = Command::new("getent").args(["passwd", &sudo_user]).output() {
                    if out.status.success() {
                        if let Ok(s) = String::from_utf8(out.stdout) {
                            let home = s.split(':').nth(5).unwrap_or("").trim();
                            if !home.is_empty() {
                                return Ok(PathBuf::from(home));
                            }
                        }
                    }
                }
                return Ok(PathBuf::from("/home").join(sudo_user));
            }
        }
    }

    // Non-unix or root with no sudo context: fall back to dirs.
    dirs::home_dir()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))
}

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
        install_app_bundle(&tar, manifest.version.as_deref())?;
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
    // If we have apt, try to use it, but since we are not yet in official repos,
    // this might fail unless the user added our PPA.
    // For now, we prefer the tarball install as it's more universal.

    let client = default_http_client()?;
    let platform = Platform::Linux;

    // Linux install sets up a system-wide shared server (systemd unit, /usr/local, /var/lib).
    // Require root. We intentionally do NOT invoke `sudo` inside the CLI; users should run:
    //   sudo linggen install
    #[cfg(unix)]
    if !is_root_unix() {
        anyhow::bail!("Linux install requires root. Please run: sudo linggen install");
    }

    // Best-effort: configure shared indexing permissions so the system service (DynamicUser)
    // can index projects under /home/<user>/... without requiring users to chmod their home.
    #[cfg(target_os = "linux")]
    {
        if let Err(e) = setup_shared_indexing_permissions_linux() {
            println!(
                "{}",
                format!(
                    "⚠️  Could not fully configure shared indexing permissions automatically: {}",
                    e
                )
                .yellow()
            );
        }
    }

    // 1. Install CLI
    if let Some(cli_art) = select_artifact(manifest, platform, ArtifactKind::Cli) {
        println!("{}", "⬇️  Downloading CLI tarball...".cyan());
        let tar = download_to_temp(&client, &cli_art.url, cli_art.signature.as_deref()).await?;

        // Extract to /usr/local/bin (requires root)
        println!("{}", "🔧 Installing CLI to /usr/local/bin...".cyan());
        let status = Command::new("tar")
            .args(["-xzf", tar.to_str().unwrap(), "-C", "/usr/local/bin"])
            .status()?;
        if !status.success() {
            anyhow::bail!("Failed to extract CLI tarball to /usr/local/bin");
        }
        println!("{}", "✅ CLI installed to /usr/local/bin/linggen".green());
    }

    // 2. Install Server
    if let Some(srv_art) = select_artifact(manifest, platform, ArtifactKind::Server) {
        println!("{}", "⬇️  Downloading server tarball...".cyan());
        let tar = download_to_temp(&client, &srv_art.url, srv_art.signature.as_deref()).await?;

        let share_dir = "/usr/local/share/linggen";
        println!(
            "{}",
            format!("🔧 Installing server to {}...", share_dir).cyan()
        );

        // Ensure share dir exists
        run_cmd("mkdir", &["-p", share_dir])?;

        // Extract to share dir. Note: tarball has a root folder like linggen-server-linux-x86_64/
        // We want the contents of that folder to be in /usr/local/share/linggen/
        let tmp_extract = tempdir()?;
        extract_tarball(&tar, tmp_extract.path().to_str().unwrap())?;

        // Seed library before moving contents to /usr/local/share
        seed_library_from_extracted_path(tmp_extract.path())?;

        // Find the extracted folder (it will be linggen-server-linux-*)
        let entries = fs::read_dir(tmp_extract.path())?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir()
                && path
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("linggen-server-linux-")
            {
                // Copy contents to /usr/local/share/linggen
                let src = format!("{}/.", path.to_str().unwrap());
                run_cmd("cp", &["-rf", &src, share_dir])?;
            }
        }

        // Symlink binary to /usr/local/bin
        println!(
            "{}",
            "🔗 Creating symlink /usr/local/bin/linggen-server...".cyan()
        );
        run_cmd(
            "ln",
            &[
                "-sf",
                "/usr/local/share/linggen/linggen-server",
                "/usr/local/bin/linggen-server",
            ],
        )?;

        // Install systemd unit
        install_systemd_unit()?;

        println!(
            "{}",
            "✅ Server installed and systemd service started".green()
        );
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

    // We intentionally do NOT invoke sudo inside the CLI.
    // If the destination is not writable, instruct the user to re-run with sudo.
    let Some(dest_str) = dest.to_str() else {
        return Ok(false);
    };
    eprintln!(
        "⚠️  Failed to replace binary at {} (permission denied?). Try re-running with sudo: sudo linggen update",
        dest_str
    );

    Ok(false)
}

#[cfg(unix)]
fn is_root_unix() -> bool {
    // Avoid adding extra deps; use `id -u`.
    let out = std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    out.trim() == "0"
}

#[cfg(target_os = "linux")]
fn setup_shared_indexing_permissions_linux() -> Result<()> {
    // Opt-out switch for locked-down environments.
    if std::env::var("LINGGEN_SKIP_HOST_PERMISSIONS")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        return Ok(());
    }

    // Determine which *human* user invoked the install.
    //
    // NOTE: `SUDO_USER`/`SUDO_UID` are only visible *inside* the sudo'd process.
    // Users can't see it via `sudo echo $SUDO_USER` because the shell expands `$SUDO_USER`
    // before sudo runs.
    let sudo_user = std::env::var("SUDO_USER").unwrap_or_default();
    let sudo_uid = std::env::var("SUDO_UID").unwrap_or_default();

    // Prefer SUDO_USER when present; otherwise fall back to SUDO_UID -> passwd lookup.
    let target_user = if !sudo_user.trim().is_empty() && sudo_user != "root" {
        Some(sudo_user)
    } else if !sudo_uid.trim().is_empty() {
        // getent passwd <uid> -> name:x:uid:gid:gecos:home:shell
        let out = Command::new("getent")
            .args(["passwd", sudo_uid.trim()])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();
        let name = out.split(':').next().unwrap_or("").trim().to_string();
        if name.is_empty() || name == "root" {
            None
        } else {
            Some(name)
        }
    } else {
        None
    };

    let Some(target_user) = target_user else {
        return Ok(());
    };

    // Create stable group `linggen` and add invoking user to it.
    // These are best-effort and typically idempotent.
    let _ = run_cmd("groupadd", &["-f", "linggen"]);
    let _ = run_cmd("usermod", &["-aG", "linggen", &target_user]);

    // Grant the group traverse permissions on /home/<user> so the service can reach
    // /home/<user>/workspace/... without opening the home dir to "others".
    // Do not assume /home/<user>; resolve from passwd database.
    let passwd = Command::new("getent")
        .args(["passwd", &target_user])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let home_path = passwd.split(':').nth(5).unwrap_or("").trim().to_string();

    if !home_path.is_empty() && std::path::Path::new(&home_path).exists() {
        // `setfacl` may not be installed.
        let has_setfacl = Command::new("setfacl")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok();

        if has_setfacl {
            let _ = run_cmd("setfacl", &["-m", "g:linggen:--x", &home_path]);
            println!(
                "{}",
                format!(
                    "✅ Granted linggen group traverse access to {} (via ACL).",
                    home_path
                )
                .green()
            );
        } else {
            println!(
                "{}",
                "⚠️  'setfacl' not found; linggen-server may not be able to index projects under /home/<user>.\n    Install ACL tools (e.g. `sudo apt install acl`) or move projects under a shared directory (e.g. /srv)."
                    .yellow()
            );
        }
    }

    Ok(())
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

fn install_app_bundle(tar_path: &PathBuf, _version: Option<&str>) -> Result<()> {
    let tmp_dir = tempdir().context("Failed to create temp dir for app extraction")?;
    let tmp_path = tmp_dir.path();

    let dest_str = tmp_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid temp dir path"))?;
    extract_tarball(tar_path, dest_str)?;

    // Seed library before moving bundle to /Applications
    seed_library_from_extracted_path(tmp_path)?;

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
        if let Err(e) = fs::remove_dir_all(&dest_app) {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                anyhow::bail!(
                    "Failed to remove existing /Applications/Linggen.app (permission denied). Try:\n  sudo linggen install"
                );
            }
            return Err(e).context("Failed to remove existing /Applications/Linggen.app");
        }
    }

    // Copy the bundle. (On macOS, we retry with sudo if permissions fail.)
    let copy_result = Command::new("cp")
        .args(["-R", app_src, "/Applications/"])
        .status();

    let success = copy_result.as_ref().map(|s| s.success()).unwrap_or(false);

    if !success {
        anyhow::bail!(
            "Failed to copy app bundle to /Applications (permission denied?). Try:\n  sudo linggen install"
        );
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
    // System-wide shared indexing server for multiple users.
    //
    // We use systemd's DynamicUser + StateDirectory so:
    // - we don't need to create a persistent `linggen` user
    // - state is stored under /var/lib/linggen (shared, persistent)
    // - logs go to journald by default (tail with `journalctl -u linggen-server -f`)
    let unit = r#"[Unit]
Description=Linggen Server
After=network.target

[Service]
ExecStart=/usr/local/bin/linggen-server
Restart=on-failure

# Shared, persistent state for the service
DynamicUser=yes
StateDirectory=linggen

# Allow the service to read user projects when admins grant access via group/ACL.
SupplementaryGroups=linggen

# Explicitly set paths so we never depend on HOME (avoids /root surprises)
Environment=LINGGEN_DATA_DIR=/var/lib/linggen
Environment=LINGGEN_LIBRARY_DIR=/var/lib/linggen/library
Environment=LINGGEN_FRONTEND_DIR=/usr/local/share/linggen/frontend

# Make sure common "home/cache" locations are writable even when HOME is unset.
Environment=HOME=/var/lib/linggen
Environment=XDG_CACHE_HOME=/var/lib/linggen/cache

# HuggingFace hub cache (embedding model downloads). Without this, services may try to write
# under / (read-only) or another non-writable location when HOME is unset.
Environment=HF_HOME=/var/lib/linggen/hf
Environment=HF_HUB_CACHE=/var/lib/linggen/hf/hub
Environment=HUGGINGFACE_HUB_CACHE=/var/lib/linggen/hf/hub

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
        "cp",
        &["-f", &tmp_str, "/etc/systemd/system/linggen-server.service"],
    )?;
    run_cmd("systemctl", &["daemon-reload"])?;
    run_cmd("systemctl", &["enable", "--now", "linggen-server"])?;
    println!(
        "{}",
        "✅ systemd unit installed/enabled (linggen-server)".green()
    );
    Ok(())
}

/// Seed library templates from an extracted directory (temporary extraction root).
fn seed_library_from_extracted_path(extracted_root: &Path) -> Result<()> {
    // 1. Find the library_templates directory anywhere in the extracted root.
    let template_src = match find_library_templates_dir(extracted_root)? {
        Some(path) => path,
        None => {
            // Non-fatal, just warn.
            println!(
                "{}",
                "⚠️  Could not find library_templates in extracted package; skipping library seed."
                    .yellow()
            );
            return Ok(());
        }
    };

    // 2. Determine target library path.
    //
    // For the shared Linux server install, library is system-wide:
    //   /var/lib/linggen/library/official
    //
    // (This matches the systemd unit Environment=LINGGEN_LIBRARY_DIR=/var/lib/linggen/library)
    //
    // On macOS, the CLI install is typically run without root and should seed into the
    // per-user library path instead of /var/lib.
    let library_root = if cfg!(target_os = "linux") {
        PathBuf::from("/var/lib/linggen/library")
    } else {
        let home = resolve_human_home_dir()
            .context("Could not determine home directory for library seed")?;
        home.join(".linggen").join("library")
    };
    let official_dst = library_root.join("official");

    // 3. Update official templates (wipe + replace)
    println!(
        "{}",
        format!(
            "🎨 Updating official library templates in {:?}...",
            official_dst
        )
        .cyan()
    );

    // Ensure base dirs exist and are writable for the service (systemd will chown StateDirectory,
    // but we create/populate it at install time).
    let library_root_str = library_root
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid library path: {:?}", library_root))?;
    let official_dst_str = official_dst
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid library path: {:?}", official_dst))?;

    run_cmd("mkdir", &["-p", library_root_str])?;
    // Wipe old official templates if present
    run_cmd("rm", &["-rf", official_dst_str])?;
    run_cmd("mkdir", &["-p", official_dst_str])?;

    // Copy templates into place (requires root).
    // Note: copy contents of template_src into official_dst.
    let src = format!("{}/.", template_src.display());
    run_cmd("cp", &["-rf", &src, official_dst_str])?;

    println!(
        "{}",
        format!(
            "✅ Updated official library templates in {:?}",
            official_dst
        )
        .green()
    );

    Ok(())
}

// We previously used a Rust recursive copy helper for seeding packs into a per-user directory.
// For the shared Linux system install we seed to /var/lib/linggen via root-owned operations, so we no
// longer need this helper.

fn find_library_templates_dir(root: &Path) -> Result<Option<PathBuf>> {
    // We do a breadth-first-ish search for a directory named "library_templates"
    let mut stack = vec![root.to_path_buf()];

    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().and_then(|n| n.to_str()) == Some("library_templates") {
                    return Ok(Some(path));
                }
                stack.push(path);
            }
        }
    }

    Ok(None)
}
