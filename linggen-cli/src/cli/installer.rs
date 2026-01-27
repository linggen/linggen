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

    // 1. Update CLI first (self-update)
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

    // 2. Install/update Server (embedded UI) to user-local directory
    if let Some(srv_art) = select_artifact(manifest, Platform::Mac, ArtifactKind::Server) {
        println!("{}", "⬇️  Downloading Linggen server tarball...".cyan());
        let tar = download_to_temp(&client, &srv_art.url, srv_art.signature.as_deref()).await?;
        install_server_macos(&tar)?;
    } else {
        println!(
            "{}",
            "⚠️ No macOS server artifact in manifest; cannot install.".yellow()
        );
        anyhow::bail!("No macOS server artifact found in manifest");
    }

    Ok(())
}

pub async fn install_linux(manifest: &crate::manifest::Manifest) -> Result<()> {
    let client = default_http_client()?;
    let platform = Platform::Linux;

    #[cfg(unix)]
    if !is_root_unix() {
        anyhow::bail!("Linux install requires root. Please run: sudo linggen install");
    }

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

        run_cmd("mkdir", &["-p", share_dir])?;

        let tmp_extract = tempdir()?;
        extract_tarball(&tar, tmp_extract.path().to_str().unwrap())?;

        // seed_library_from_extracted_path(tmp_extract.path())?;

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
                let src = format!("{}/.", path.to_str().unwrap());
                run_cmd("cp", &["-rf", &src, share_dir])?;
            }
        }

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

    let extracted = tmp_path.join("linggen");
    if !extracted.exists() {
        anyhow::bail!("No `linggen` binary found in extracted tarball");
    }

    let mut tried = Vec::<String>::new();
    if let Ok(current_exe) = std::env::current_exe() {
        if current_exe.file_name().and_then(|n| n.to_str()) == Some("linggen") {
            if try_replace_binary(&extracted, &current_exe)? {
                return Ok(());
            }
            tried.push(current_exe.display().to_string());
        }
    }

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
    if let Some(parent) = dest.parent() {
        if parent.exists() {
            let tmp_name = format!(
                ".linggen.tmp.{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            );
            let tmp_dest = parent.join(tmp_name);

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

            if fs::rename(&tmp_dest, dest).is_ok() {
                return Ok(true);
            }

            let _ = fs::remove_file(&tmp_dest);
        }
    }

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
    if std::env::var("LINGGEN_SKIP_HOST_PERMISSIONS")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        return Ok(());
    }

    let sudo_user = std::env::var("SUDO_USER").unwrap_or_default();
    let sudo_uid = std::env::var("SUDO_UID").unwrap_or_default();

    let target_user = if !sudo_user.trim().is_empty() && sudo_user != "root" {
        Some(sudo_user)
    } else if !sudo_uid.trim().is_empty() {
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

    let _ = run_cmd("groupadd", &["-f", "linggen"]);
    let _ = run_cmd("usermod", &["-aG", "linggen", &target_user]);

    let passwd = Command::new("getent")
        .args(["passwd", &target_user])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let home_path = passwd.split(':').nth(5).unwrap_or("").trim().to_string();

    if !home_path.is_empty() && std::path::Path::new(&home_path).exists() {
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
        .env_remove("TAR_OPTIONS")
        .stdout(Stdio::null())
        .status()
        .with_context(|| format!("Failed to run tar on {:?}", tar_path))?;
    if !status.success() {
        anyhow::bail!("tar failed for {:?}", tar_path);
    }
    Ok(())
}

fn install_server_macos(tar_path: &PathBuf) -> Result<()> {
    let tmp_dir = tempdir().context("Failed to create temp dir for server extraction")?;
    let tmp_path = tmp_dir.path();

    let dest_str = tmp_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid temp dir path"))?;
    extract_tarball(tar_path, dest_str)?;

    // Seed library
    // seed_library_from_extracted_path(tmp_path)?;

    // Find the server binary (expecting it inside a folder named linggen-server-macos)
    let mut server_bin_src = None;
    for entry in fs::read_dir(tmp_path)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s == "linggen-server-macos")
                .unwrap_or(false)
        {
            let bin = path.join("linggen-server");
            if bin.exists() {
                server_bin_src = Some(bin);
                break;
            }
        }
    }

    let server_bin_src = server_bin_src
        .ok_or_else(|| anyhow::anyhow!("No `linggen-server` found in extracted tarball"))?;

    // Determine user-local install directory
    let home = resolve_human_home_dir()?;
    let install_dir = home.join("Library/Application Support/Linggen/bin");
    fs::create_dir_all(&install_dir).context("Failed to create install directory")?;

    let dest_bin = install_dir.join("linggen-server");

    // Atomic replacement
    let tmp_dest = install_dir.join(".linggen-server.tmp");
    fs::copy(&server_bin_src, &tmp_dest).context("Failed to copy server binary")?;
    
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&tmp_dest)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&tmp_dest, perms)?;
    }

    fs::rename(&tmp_dest, &dest_bin).context("Failed to rename server binary")?;

    println!(
        "{}",
        format!(
            "✅ Installed/updated linggen-server to {}",
            dest_bin.display()
        )
        .green()
    );

    // Check if server is running and restart it
    if is_server_running() {
        println!(
            "{}",
            "🔄 linggen-server is currently running. Restarting to use new version...".cyan()
        );
        restart_server(&dest_bin)?;
    } else {
        println!(
            "{}",
            "ℹ️  Run `linggen start` to use the new version".yellow()
        );
    }

    Ok(())
}

fn is_server_running() -> bool {
    let output = Command::new("pgrep")
        .arg("-x")
        .arg("linggen-server")
        .output();
    output.map(|o| o.status.success()).unwrap_or(false)
}

fn restart_server(bin_path: &Path) -> Result<()> {
    // Kill existing server
    let _ = Command::new("pkill").arg("-x").arg("linggen-server").status();
    
    // Wait a bit for it to die
    std::thread::sleep(Duration::from_millis(500));

    // Launch new server in background
    Command::new(bin_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Failed to launch new server version")?;

    println!("{}", "✅ linggen-server restarted successfully".green());
    Ok(())
}

fn install_systemd_unit() -> Result<()> {
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
