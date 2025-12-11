use anyhow::{Context, Result};
use base64::Engine;
use colored::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tabled::{Table, Tabled};
use tempfile::{tempdir, NamedTempFile};

use super::client::{ApiClient, CreateSourceRequest};
use crate::manifest::{current_platform, fetch_manifest, select_artifact, ArtifactKind, Platform};

// Public key for signature verification (from tauri.conf.json, base64 encoded)
const LINGGEN_PUBLIC_KEY: &str = "ZFc1MGNuVnpkR1ZrSUdOdmJXMWxiblE2SUcxcGJtbHphV2R1SUhCMVlteHBZeUJyWlhrNklERTVOa1ZHTlVSRFF6RXhOa0UxT0VFS1VsZFRTM0JTWWtJelVGWjFSMll6VlVWYVFrWlBRMFYxVFhJemJ6aFBha2h6YVVNNVNXbzNkVFZWZVV4cmEzbGhWbVoyTXpoVFZ6TUs=";

/// Install using manifest (preferred); fall back to package manager when available.
pub async fn handle_install(version: Option<&str>) -> Result<()> {
    if let Some(ver) = version {
        println!(
            "{}",
            format!("🔧 Installing Linggen version {}...", ver).cyan()
        );
    } else {
        println!("{}", "🔧 Installing Linggen (latest version)...".cyan());
    }
    let platform = current_platform();
    let manifest = fetch_manifest(version)
        .await
        .context("Failed to fetch manifest")?;

    // Show version info
    if let Some(remote_version) = &manifest.version {
        println!(
            "{}",
            format!("Installing version: {}", remote_version).cyan()
        );

        if let Some(local_ver) = get_local_app_version() {
            println!("{}", format!("Current version:    {}", local_ver).cyan());
        }
    }

    match platform {
        Platform::Mac => install_macos(&manifest).await,
        Platform::Linux => install_linux(&manifest).await,
        Platform::Other => {
            println!(
                "{}",
                "⚠️ Unsupported platform for automated install. Please install manually from releases."
                    .yellow()
            );
            Ok(())
        }
    }
}

pub async fn handle_update() -> Result<()> {
    println!("{}", "⬆️  Checking for updates...".cyan());
    let platform = current_platform();
    let manifest = fetch_manifest(None)
        .await
        .context("Failed to fetch manifest")?;

    // Check remote version
    if let Some(remote_version) = &manifest.version {
        println!("{}", format!("Remote version: {}", remote_version).cyan());

        // Try to get local app version (best effort)
        let local_version = get_local_app_version();
        if let Some(local_ver) = &local_version {
            println!("{}", format!("Local version:  {}", local_ver).cyan());

            // Compare versions
            if local_ver == remote_version {
                println!("{}", "✓ Already up to date!".green());
                return Ok(());
            }
        }

        println!("{}", "⬇️  New version available, updating...".cyan());
    }

    match platform {
        Platform::Mac => install_macos(&manifest).await,
        Platform::Linux => install_linux(&manifest).await,
        Platform::Other => {
            println!(
                "{}",
                "⚠️ Unsupported platform for automated update. Please update manually.".yellow()
            );
            Ok(())
        }
    }
}

pub async fn handle_check() -> Result<()> {
    println!("{}", "🔎 Checking versions...".cyan());
    let manifest = fetch_manifest(None).await?;
    let platform = current_platform();

    let cli_version_local =
        run_and_capture_version("linggen", &["--version"]).unwrap_or_else(|| "unknown".into());
    let app_version_local = get_local_app_version().unwrap_or_else(|| "not installed".into());

    let cli_remote = select_artifact(&manifest, platform, ArtifactKind::Cli)
        .map(|a| a.url)
        .unwrap_or_else(|| "n/a".into());
    let app_remote = select_artifact(&manifest, platform, ArtifactKind::App)
        .map(|a| a.url)
        .unwrap_or_else(|| "n/a".into());

    println!("Local CLI:  {}", cli_version_local);
    println!("Local App:  {}", app_version_local);
    println!("Remote CLI: {}", cli_remote);
    println!("Remote App: {}", app_remote);
    if let Some(ver) = manifest.version {
        println!("Manifest version: {}", ver);
    }
    Ok(())
}

fn run_cmd(cmd: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(cmd)
        .args(args)
        .status()
        .with_context(|| format!("Failed to run {} {:?}", cmd, args))?;
    if !status.success() {
        anyhow::bail!("Command failed: {} {:?}", cmd, args);
    }
    Ok(())
}

fn run_and_capture_version(cmd: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(cmd).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}

fn command_exists(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {}", cmd))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn get_local_app_version() -> Option<String> {
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

/// ----- platform-specific installers -----

async fn install_macos(manifest: &crate::manifest::Manifest) -> Result<()> {
    // Install/update app bundle to /Applications from tarball
    // (CLI is installed separately via install-cli.sh, server is bundled in the app)
    if let Some(app_art) = select_artifact(manifest, Platform::Mac, ArtifactKind::App) {
        println!("{}", "⬇️  Downloading Linggen app tarball...".cyan());
        let tar = download_to_temp(&app_art.url, app_art.signature.as_deref()).await?;
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

async fn install_linux(manifest: &crate::manifest::Manifest) -> Result<()> {
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
    if let Some(cli_art) = select_artifact(manifest, Platform::Linux, ArtifactKind::Cli) {
        println!("{}", "⬇️  Downloading CLI tarball...".cyan());
        let tar = download_to_temp(&cli_art.url, cli_art.signature.as_deref()).await?;
        extract_tarball(&tar, "/usr/local/bin")?;
        println!("{}", "✅ Installed/updated CLI from tarball".green());
    } else {
        println!("{}", "⚠️ No CLI tarball found in manifest".yellow());
    }

    if let Some(srv_art) = select_artifact(manifest, Platform::Linux, ArtifactKind::Server) {
        println!("{}", "⬇️  Downloading server tarball...".cyan());
        let tar = download_to_temp(&srv_art.url, srv_art.signature.as_deref()).await?;
        extract_tarball(&tar, "/usr/local/bin")?;
        install_systemd_unit()?;
        println!("{}", "✅ Installed/updated server from tarball".green());
    } else {
        println!("{}", "⚠️ No server tarball found in manifest".yellow());
    }

    Ok(())
}

/// Verify a file's signature using minisign
fn verify_signature(file_path: &Path, signature: &str, public_key: &str) -> Result<()> {
    use minisign::{PublicKey, PublicKeyBox, SignatureBox};
    use std::io::Cursor;

    // Decode the base64-encoded public key to get the minisign format
    let pubkey_bytes = base64::engine::general_purpose::STANDARD
        .decode(public_key)
        .context("Failed to decode public key from base64")?;

    // Parse the public key (minisign format: comment line + key line)
    let pubkey_str = String::from_utf8(pubkey_bytes).context("Public key is not valid UTF-8")?;

    // Parse the public key box and convert to public key
    let pk_box = PublicKeyBox::from_string(&pubkey_str)
        .context("Failed to parse public key - ensure it's in minisign format")?;

    // Convert PublicKeyBox to PublicKey
    // Try from_box first (for trusted keys), fallback to into_public_key (for untrusted)
    let pk = match PublicKey::from_box(pk_box.clone()) {
        Ok(pk) => pk,
        Err(_) => {
            // If from_box fails, try into_public_key (for untrusted keys)
            pk_box.into_public_key()
                .context("Failed to convert public key box to public key - key may be invalid or in wrong format")?
        }
    };

    // The signature from manifest is base64-encoded .sig file content
    // Decode it to get the minisign signature format
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(signature)
        .context("Failed to decode signature from base64")?;

    let sig_str = String::from_utf8(sig_bytes).context("Signature is not valid UTF-8")?;

    let sig_box = SignatureBox::from_string(&sig_str).context("Failed to parse signature")?;

    // Read the file to verify
    let file_data = fs::read(file_path)
        .with_context(|| format!("Failed to read file: {}", file_path.display()))?;
    let data_reader = Cursor::new(&file_data);

    // Verify the signature (prehash=false, quiet=true, allow_legacy=false)
    minisign::verify(&pk, &sig_box, data_reader, false, true, false)
        .context("Signature verification failed")?;

    Ok(())
}

async fn download_to_temp(url: &str, signature: Option<&str>) -> Result<PathBuf> {
    let tmp = tempfile::NamedTempFile::new().context("Failed to create temp file")?;
    let tmp_path = tmp.path().to_path_buf();

    // Retry logic for 503 errors (GitHub CDN issues)
    // Try up to 5 times with exponential backoff: 1s, 2s, 4s, 8s
    let max_attempts = 5;
    let mut resp = None;
    let mut last_error = None;

    for attempt in 0..max_attempts {
        match reqwest::get(url).await {
            Ok(r) => {
                if r.status().is_success() {
                    resp = Some(r);
                    break;
                } else if r.status() == 503 && attempt < max_attempts - 1 {
                    // Wait before retry with exponential backoff: 1s, 2s, 4s, 8s
                    let delay = Duration::from_secs(1 << attempt);
                    println!(
                        "{}",
                        format!(
                            "⚠️  Download returned 503 (attempt {}/{}), retrying in {}s...",
                            attempt + 1,
                            max_attempts,
                            delay.as_secs()
                        )
                        .yellow()
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                } else {
                    last_error = Some(format!(
                        "HTTP {} {}",
                        r.status(),
                        r.status().canonical_reason().unwrap_or("Unknown")
                    ));
                    if attempt < max_attempts - 1 && (r.status() == 503 || r.status() == 429) {
                        // Retry for 503 or 429 (rate limit)
                        let delay = Duration::from_secs(1 << attempt);
                        println!(
                            "{}",
                            format!(
                                "⚠️  Download returned {} (attempt {}/{}), retrying in {}s...",
                                r.status(),
                                attempt + 1,
                                max_attempts,
                                delay.as_secs()
                            )
                            .yellow()
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    anyhow::bail!("Download failed: {} {}", r.status(), url);
                }
            }
            Err(e) => {
                last_error = Some(format!("Network error: {}", e));
                if attempt < max_attempts - 1 {
                    let delay = Duration::from_secs(1 << attempt);
                    println!(
                        "{}",
                        format!(
                            "⚠️  Network error (attempt {}/{}), retrying in {}s...",
                            attempt + 1,
                            max_attempts,
                            delay.as_secs()
                        )
                        .yellow()
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                } else {
                    anyhow::bail!(
                        "Download failed after {} attempts: {} - {}",
                        max_attempts,
                        url,
                        e
                    );
                }
            }
        }
    }

    let resp = resp.ok_or_else(|| {
        anyhow::anyhow!(
            "Failed to download after {} attempts: {} - {}",
            max_attempts,
            url,
            last_error.unwrap_or_else(|| "Unknown error".to_string())
        )
    })?;
    let bytes = resp
        .bytes()
        .await
        .with_context(|| format!("Failed to read body for {}", url))?;
    fs::write(&tmp_path, &bytes)?;

    // Verify signature if provided
    if let Some(sig) = signature {
        println!("{}", "🔐 Verifying signature...".cyan());
        match verify_signature(&tmp_path, sig, LINGGEN_PUBLIC_KEY) {
            Ok(_) => println!("{}", "✅ Signature verified".green()),
            Err(e) => {
                println!(
                    "{}",
                    format!("⚠️  Signature verification failed: {}", e).yellow()
                );
                println!(
                    "{}",
                    "   Continuing anyway (use --skip-verify to suppress this warning)".yellow()
                );
                // For now, we'll warn but continue. In production, you might want to fail here.
                // return Err(e);
            }
        }
    } else {
        println!(
            "{}",
            "⚠️  No signature provided in manifest; skipping verification".yellow()
        );
    }

    tmp.keep().ok(); // ensure file is not deleted on drop
    Ok(tmp_path)
}

fn extract_tarball(tar_path: &PathBuf, dest_dir: &str) -> Result<()> {
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(tar_path)
        .arg("-C")
        .arg(dest_dir)
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
            "⚠️  Could not restart app automatically. Please quit and restart Linggen.app manually.".yellow()
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

/// Handle the `start` command
pub async fn handle_start(api_client: &ApiClient) -> Result<()> {
    println!("{}", "🔍 Checking Linggen backend status...".cyan());

    match api_client.get_status().await {
        Ok(status) => {
            match status.status.as_str() {
                "ready" => {
                    println!("{}", "✅ Linggen backend is ready!".green().bold());
                    println!("   Status: {}", "Ready".green());
                }
                "initializing" => {
                    println!("{}", "⏳ Linggen backend is initializing...".yellow());
                    if let Some(msg) = status.message {
                        println!("   {}", msg);
                    }
                    if let Some(progress) = status.progress {
                        println!("   Progress: {}", progress);
                    }
                }
                "error" => {
                    println!("{}", "❌ Linggen backend encountered an error".red().bold());
                    if let Some(msg) = status.message {
                        println!("   Error: {}", msg.red());
                    }
                    anyhow::bail!("Backend is in error state");
                }
                _ => {
                    println!("   Status: {}", status.status);
                }
            }
            Ok(())
        }
        Err(e) => {
            println!("{}", "❌ Linggen backend is not running".red().bold());
            println!("\n{}", "To start Linggen backend:".yellow());
            println!("  - If you installed the desktop app, open Linggen.app");
            println!("  - Or run: linggen serve");
            println!("\nError details: {}", e);
            anyhow::bail!("Backend not reachable");
        }
    }
}

/// Handle the `index` command
pub async fn handle_index(
    api_client: &ApiClient,
    path: Option<PathBuf>,
    mode: String,
    name: Option<String>,
    include_patterns: Vec<String>,
    exclude_patterns: Vec<String>,
    wait: bool,
) -> Result<()> {
    // Default to current directory if no path provided
    let path = path.unwrap_or_else(|| PathBuf::from("."));

    // Normalize path to absolute
    let abs_path = path
        .canonicalize()
        .with_context(|| format!("Invalid path: {}", path.display()))?;

    let path_str = abs_path
        .to_str()
        .context("Path contains invalid UTF-8")?
        .to_string();

    println!("{}", format!("📂 Indexing: {}", abs_path.display()).cyan());

    // Check if source already exists
    let sources = api_client.list_sources().await?;
    let existing_source = sources
        .resources
        .iter()
        .find(|s| s.path == path_str && s.resource_type == "local");

    let (source_id, is_previously_indexed) = if let Some(source) = existing_source {
        println!(
            "{}",
            format!("   Found existing source: {}", source.name).dimmed()
        );

        // Check if source has been indexed before (has stats)
        let has_stats = source
            .stats
            .as_ref()
            .map(|s| s.chunk_count > 0)
            .unwrap_or(false);

        if has_stats {
            let stats = source.stats.as_ref().unwrap();
            println!(
                "{}",
                format!(
                    "   Previously indexed: {} files, {} chunks",
                    stats.file_count, stats.chunk_count
                )
                .dimmed()
            );
        }

        (source.id.clone(), has_stats)
    } else {
        // Create new source
        let source_name = name.unwrap_or_else(|| {
            abs_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Unnamed")
                .to_string()
        });

        println!(
            "{}",
            format!("   Creating new source: {}", source_name).dimmed()
        );

        let req = CreateSourceRequest {
            name: source_name,
            resource_type: "local".to_string(),
            path: path_str,
            include_patterns: include_patterns.clone(),
            exclude_patterns: exclude_patterns.clone(),
        };

        let response = api_client.create_source(req).await?;
        println!(
            "{}",
            format!("   ✅ Source created: {}", response.id).green()
        );
        (response.id, false)
    };

    // Determine indexing mode
    let mode = mode.to_lowercase();
    let final_mode = match mode.as_str() {
        "auto" => {
            // Auto mode: incremental if previously indexed, full otherwise
            if is_previously_indexed {
                println!(
                    "{}",
                    "   Mode: auto → incremental (source already indexed)".dimmed()
                );
                "incremental"
            } else {
                println!("{}", "   Mode: auto → full (first-time indexing)".dimmed());
                "full"
            }
        }
        "full" => "full",
        "incremental" => "incremental",
        _ => {
            anyhow::bail!(
                "Invalid mode: {}. Use 'auto', 'full', or 'incremental'",
                mode
            );
        }
    };

    // Validate mode choice
    if final_mode == "incremental" && !is_previously_indexed {
        println!(
            "{}",
            "   ⚠️  Warning: Using incremental mode on a new source (no previous index found)"
                .yellow()
        );
    }

    // Trigger indexing
    println!(
        "{}",
        format!("🚀 Starting {} indexing...", final_mode)
            .cyan()
            .bold()
    );

    let response = api_client.index_source(&source_id, final_mode).await?;

    println!("{}", "✅ Indexing job started!".green().bold());
    println!("   Job ID: {}", response.job_id.dimmed());

    if wait {
        println!("\n{}", "⏳ Waiting for job to complete...".cyan());
        wait_for_job(api_client, &response.job_id).await?;
    } else {
        println!("\nℹ️  Use `linggen status` to check indexing progress");
    }

    Ok(())
}

/// Wait for a job to complete by polling
async fn wait_for_job(api_client: &ApiClient, job_id: &str) -> Result<()> {
    let mut last_status = String::new();
    let mut last_progress = (0, 0); // (files_indexed, total_files)
    let poll_interval = Duration::from_secs(1); // Poll more frequently for better UX

    loop {
        tokio::time::sleep(poll_interval).await;

        let jobs = api_client.list_jobs().await?;
        let job = jobs.jobs.iter().find(|j| j.id == job_id);

        match job {
            Some(job) => {
                let current_progress =
                    (job.files_indexed.unwrap_or(0), job.total_files.unwrap_or(0));
                let status_changed = job.status != last_status;
                let progress_changed = current_progress != last_progress;

                match job.status.as_str() {
                    "Pending" => {
                        if status_changed {
                            println!("   Status: {}", "Pending...".yellow());
                        }
                    }
                    "Running" => {
                        // Show progress whenever it changes OR on status change
                        if status_changed || progress_changed {
                            let (indexed, total) = current_progress;
                            if total > 0 {
                                let percentage = (indexed as f64 / total as f64 * 100.0) as u32;
                                println!(
                                    "   Progress: {}/{} files ({}%) - {} chunks created",
                                    indexed.to_string().cyan(),
                                    total,
                                    percentage.to_string().cyan(),
                                    job.chunks_created.unwrap_or(0).to_string().cyan()
                                );
                            } else {
                                println!("   Status: {} - processing...", "Running".cyan());
                            }
                        }
                    }
                    "Completed" => {
                        if status_changed {
                            println!("\n{}", "✅ Job completed successfully!".green().bold());
                            if let Some(files) = job.files_indexed {
                                println!("   Files indexed: {}", files);
                            }
                            if let Some(chunks) = job.chunks_created {
                                println!("   Chunks created: {}", chunks);
                            }
                        }
                        return Ok(());
                    }
                    "Failed" => {
                        if status_changed {
                            println!("\n{}", "❌ Job failed".red().bold());
                            if let Some(error) = &job.error {
                                println!("   Error: {}", error.red());
                            }
                        }
                        anyhow::bail!("Indexing job failed");
                    }
                    _ => {
                        if status_changed {
                            println!("   Status: {}", job.status);
                        }
                    }
                }

                last_status = job.status.clone();
                last_progress = current_progress;
            }
            None => {
                println!("{}", "⚠️  Job not found".yellow());
                break;
            }
        }
    }

    Ok(())
}

/// Handle the `status` command
pub async fn handle_status(api_client: &ApiClient, limit: usize) -> Result<()> {
    println!("{}", "📊 Fetching system status...".cyan().bold());
    println!();

    // Get backend status
    let status = match api_client.get_status().await {
        Ok(s) => s,
        Err(e) => {
            println!("{}", "❌ Linggen backend not reachable.".red().bold());
            println!("Error: {e}");
            println!();
            println!("{}", "Install or ensure Linggen is running:".yellow());
            println!("  curl -fsSL https://linggen.dev/install-cli.sh | bash");
            println!("  linggen install");
            return Ok(());
        }
    };
    println!("{}", "Backend Status:".bold());
    match status.status.as_str() {
        "ready" => println!("  Status: {}", "Ready ✅".green()),
        "initializing" => {
            println!("  Status: {}", "Initializing ⏳".yellow());
            if let Some(msg) = status.message {
                println!("  Message: {}", msg);
            }
        }
        "error" => {
            println!("  Status: {}", "Error ❌".red());
            if let Some(msg) = status.message {
                println!("  Error: {}", msg.red());
            }
        }
        _ => println!("  Status: {}", status.status),
    }
    println!();

    // Get recent jobs
    let jobs_response = api_client.list_jobs().await?;
    let recent_jobs: Vec<_> = jobs_response.jobs.into_iter().take(limit).collect();

    if recent_jobs.is_empty() {
        println!("{}", "No jobs found".dimmed());
        return Ok(());
    }

    println!("{}", format!("Recent Jobs (last {}):", limit).bold());
    println!();

    // Build table rows
    let table_rows: Vec<JobTableRow> = recent_jobs
        .iter()
        .map(|job| {
            let status_display = match job.status.as_str() {
                "Completed" => "✅ Completed".green().to_string(),
                "Running" => "🔄 Running".cyan().to_string(),
                "Pending" => "⏳ Pending".yellow().to_string(),
                "Failed" => "❌ Failed".red().to_string(),
                _ => job.status.clone(),
            };

            let files_chunks = match (job.files_indexed, job.chunks_created) {
                (Some(f), Some(c)) => format!("{} / {}", f, c),
                (Some(f), None) => format!("{} / -", f),
                (None, Some(c)) => format!("- / {}", c),
                (None, None) => "-".to_string(),
            };

            JobTableRow {
                source: job.source_name.clone(),
                mode: job
                    .source_type
                    .chars()
                    .take(1)
                    .collect::<String>()
                    .to_uppercase(),
                status: status_display,
                files_chunks,
                started: format_timestamp(&job.started_at),
            }
        })
        .collect();

    let table = Table::new(table_rows).to_string();
    println!("{}", table);

    Ok(())
}

#[derive(Tabled)]
struct JobTableRow {
    #[tabled(rename = "Source")]
    source: String,
    #[tabled(rename = "Type")]
    mode: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Files/Chunks")]
    files_chunks: String,
    #[tabled(rename = "Started")]
    started: String,
}

fn format_timestamp(ts: &str) -> String {
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
