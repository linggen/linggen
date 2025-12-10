use anyhow::{Context, Result};
use colored::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tabled::{Table, Tabled};
use tempfile::{tempdir, NamedTempFile};

use super::client::{ApiClient, CreateSourceRequest};
use crate::manifest::{current_platform, fetch_manifest, select_artifact, ArtifactKind, Platform};

/// Install using manifest (preferred); fall back to package manager when available.
pub async fn handle_install() -> Result<()> {
    println!("{}", "🔧 Installing Linggen...".cyan());
    let platform = current_platform();
    let manifest = fetch_manifest().await.context("Failed to fetch manifest")?;

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
    println!("{}", "⬆️  Updating Linggen...".cyan());
    let platform = current_platform();
    let manifest = fetch_manifest().await.context("Failed to fetch manifest")?;

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
    let manifest = fetch_manifest().await?;
    let platform = current_platform();

    let cli_version_local =
        run_and_capture_version("linggen", &["--version"]).unwrap_or_else(|| "unknown".into());
    let server_version_local = run_and_capture_version("linggen-server", &["--version"])
        .unwrap_or_else(|| "unknown".into());

    let cli_remote = select_artifact(&manifest, platform, ArtifactKind::Cli)
        .map(|a| a.url)
        .unwrap_or_else(|| "n/a".into());
    let server_remote = select_artifact(&manifest, platform, ArtifactKind::Server)
        .map(|a| a.url)
        .unwrap_or_else(|| "n/a".into());

    println!("Local CLI:    {}", cli_version_local);
    println!("Local server: {}", server_version_local);
    println!("Remote CLI:   {}", cli_remote);
    println!("Remote server: {}", server_remote);
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

/// ----- platform-specific installers -----

async fn install_macos(manifest: &crate::manifest::Manifest) -> Result<()> {
    // Install/update app bundle to /Applications from tarball
    // (CLI is installed separately via install-cli.sh, server is bundled in the app)
    if let Some(app_art) = select_artifact(manifest, Platform::Mac, ArtifactKind::App) {
        println!("{}", "⬇️  Downloading Linggen app tarball...".cyan());
        let tar = download_to_temp(&app_art.url).await?;
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
        let tar = download_to_temp(&cli_art.url).await?;
        extract_tarball(&tar, "/usr/local/bin")?;
        println!("{}", "✅ Installed/updated CLI from tarball".green());
    } else {
        println!("{}", "⚠️ No CLI tarball found in manifest".yellow());
    }

    if let Some(srv_art) = select_artifact(manifest, Platform::Linux, ArtifactKind::Server) {
        println!("{}", "⬇️  Downloading server tarball...".cyan());
        let tar = download_to_temp(&srv_art.url).await?;
        extract_tarball(&tar, "/usr/local/bin")?;
        install_systemd_unit()?;
        println!("{}", "✅ Installed/updated server from tarball".green());
    } else {
        println!("{}", "⚠️ No server tarball found in manifest".yellow());
    }

    Ok(())
}

async fn download_to_temp(url: &str) -> Result<PathBuf> {
    let tmp = tempfile::NamedTempFile::new().context("Failed to create temp file")?;
    let tmp_path = tmp.path().to_path_buf();
    let resp = reqwest::get(url)
        .await
        .with_context(|| format!("Failed to download {}", url))?;
    if !resp.status().is_success() {
        anyhow::bail!("Download failed: {} {}", resp.status(), url);
    }
    let bytes = resp
        .bytes()
        .await
        .with_context(|| format!("Failed to read body for {}", url))?;
    fs::write(&tmp_path, &bytes)?;
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
