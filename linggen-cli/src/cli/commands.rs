use anyhow::{Context, Result};
use base64::Engine;
use colored::*;
use std::path::PathBuf;
use tabled::{Table, Tabled};

use super::client::{ApiClient, CreateSourceRequest};
use super::installer::{install_linux, install_macos};
use super::jobs::wait_for_job;
use super::signature::LINGGEN_PUBLIC_KEY;
use super::util::{format_timestamp, get_local_app_version, run_and_capture_version};
use crate::manifest::{current_platform, fetch_manifest, Platform};

/// Returns the user-facing current directory when possible.
///
/// On macOS `/tmp` is a symlink to `/private/tmp`, and `std::env::current_dir()`
/// may resolve to the physical path. If `$PWD` is set and points to the same
/// directory, prefer it for display / UX.
fn logical_current_dir() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let Some(pwd_os) = std::env::var_os("PWD") else {
        return cwd;
    };

    let pwd = PathBuf::from(pwd_os);
    if !pwd.is_absolute() {
        return cwd;
    }

    match (cwd.canonicalize(), pwd.canonicalize()) {
        (Ok(cwd_can), Ok(pwd_can)) if cwd_can == pwd_can => pwd,
        _ => cwd,
    }
}

/// Compute an absolute path without resolving symlinks.
///
/// This keeps "logical" segments (e.g. `/tmp`) while still removing `.` / `..`.
fn logical_absolute(path: &PathBuf) -> PathBuf {
    let p = if path.is_absolute() {
        path.clone()
    } else {
        logical_current_dir().join(path)
    };

    // Normalize `.` and `..` without hitting the filesystem (no symlink resolution).
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                let _ = out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }

    // Ensure we don't end up empty (e.g. path was ".")
    if out.as_os_str().is_empty() {
        logical_current_dir()
    } else {
        out
    }
}

fn parse_cli_version(raw: &str) -> Option<String> {
    // Expected output is typically "linggen X.Y.Z" but keep it resilient.
    for token in raw.split_whitespace() {
        let t = token.trim_start_matches('v');
        if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit() || c == '.') && t.contains('.') {
            return Some(t.to_string());
        }
    }
    None
}

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

    let latest = manifest.version.clone().unwrap_or_else(|| "unknown".into());
    let local_cli = env!("CARGO_PKG_VERSION").to_string();
    let local_app = get_local_app_version().unwrap_or_else(|| "not installed".into());

    let app_label = if std::env::consts::OS == "linux" {
        "Server"
    } else {
        "App"
    };

    println!("Local CLI: {}", local_cli);
    if let Ok(exe) = std::env::current_exe() {
        println!("  (running from: {})", exe.display().to_string().dimmed());
    }
    println!("Latest CLI: {}", latest);
    println!("Local {}: {}", app_label, local_app);
    println!("Latest {}: {}", app_label, latest);

    if local_cli == latest && local_app == latest {
        println!("{}", "✓ Already up to date!".green());
        return Ok(());
    }

    println!("{}", "⬇️  New version available, updating...".cyan());

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
    let latest = manifest.version.clone().unwrap_or_else(|| "unknown".into());
    let local_cli = env!("CARGO_PKG_VERSION").to_string();
    let local_app = get_local_app_version().unwrap_or_else(|| "not installed".into());

    let app_label = if std::env::consts::OS == "linux" {
        "Server"
    } else {
        "App"
    };

    println!("Local CLI: {}", local_cli);
    if let Ok(exe) = std::env::current_exe() {
        println!("  (running from: {})", exe.display().to_string().dimmed());
    }
    println!("Latest CLI: {}", latest);
    println!("Local {}: {}", app_label, local_app);
    println!("Latest {}: {}", app_label, latest);
    Ok(())
}

pub async fn handle_doctor(api_url: &str) -> Result<()> {
    println!("{}", "🩺 Linggen Doctor".cyan().bold());

    // CLI
    let exe = std::env::current_exe().ok();
    let cli_version = env!("CARGO_PKG_VERSION");
    println!("CLI version: {}", cli_version);
    if let Some(p) = exe {
        println!("CLI path:    {}", p.display());
    }

    // Desktop app install + version (macOS best effort)
    #[cfg(target_os = "macos")]
    {
        let app_path = std::path::Path::new("/Applications/Linggen.app");
        println!(
            "Linggen.app: {}",
            if app_path.exists() {
                "installed".green().to_string()
            } else {
                "not found".yellow().to_string()
            }
        );
    }

    // Server install check (Linux/macOS fallback)
    #[cfg(not(target_os = "macos"))]
    {
        use crate::cli::util::command_exists;
        println!(
            "Server:      {}",
            if command_exists("linggen-server") {
                "installed".green().to_string()
            } else {
                "not found".yellow().to_string()
            }
        );
    }

    let local_app = get_local_app_version().unwrap_or_else(|| "not installed".into());
    let app_label = if std::env::consts::OS == "linux" {
        "Server version"
    } else {
        "App version"
    };
    println!("{}: {}", app_label, local_app);

    // Backend reachability
    println!("API URL:     {}", api_url);
    let api_client = ApiClient::new(api_url.to_string());
    match api_client.get_status().await {
        Ok(s) => {
            println!("Backend:     {}", "reachable".green());
            println!("Status:      {}", s.status);
        }
        Err(e) => {
            println!("Backend:     {}", "not reachable".red());
            println!("Error:       {}", e);
            println!("Tip:         run `linggen start` (will launch Linggen.app on macOS)");
        }
    }

    // Manifest fetch
    match fetch_manifest(None).await {
        Ok(m) => {
            let latest = m.version.unwrap_or_else(|| "unknown".into());
            println!("Latest:      {}", latest);
        }
        Err(e) => {
            println!("Latest:      {}", "unknown".yellow());
            println!("Manifest:    {}", "failed to fetch".red());
            println!("Error:       {}", e);
        }
    }

    // Public key sanity check
    match base64::engine::general_purpose::STANDARD.decode(LINGGEN_PUBLIC_KEY) {
        Ok(pubkey_bytes) => match String::from_utf8(pubkey_bytes) {
            Ok(pubkey_str) => match minisign::PublicKeyBox::from_string(&pubkey_str) {
                Ok(_) => println!("Pubkey:      {}", "ok".green()),
                Err(e) => println!("Pubkey:      {} ({})", "invalid".red(), e),
            },
            Err(e) => println!("Pubkey:      {} ({})", "invalid".red(), e),
        },
        Err(e) => println!("Pubkey:      {} ({})", "invalid".red(), e),
    }

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

    // Keep a logical absolute path for UX (preserve `/tmp` on macOS).
    let display_abs_path = logical_absolute(&path);

    // Canonicalize for internal identity / safety.
    let abs_path = display_abs_path
        .canonicalize()
        .with_context(|| format!("Invalid path: {}", display_abs_path.display()))?;

    let abs_path_str = abs_path
        .to_str()
        .context("Path contains invalid UTF-8")?
        .to_string();

    let display_path_str = display_abs_path
        .to_str()
        .context("Path contains invalid UTF-8")?
        .to_string();

    println!(
        "{}",
        format!("📂 Indexing: {}", display_abs_path.display()).cyan()
    );

    // Check if source already exists
    let sources = api_client.list_sources().await?;
    let existing_source = sources.resources.iter().find(|s| {
        if s.resource_type != "local" {
            return false;
        }

        // Fast path: exact matches
        if s.path == display_path_str || s.path == abs_path_str {
            return true;
        }

        // Fallback: treat symlinked paths as the same source (e.g. `/tmp` vs `/private/tmp`).
        PathBuf::from(&s.path)
            .canonicalize()
            .map(|p| p == abs_path)
            .unwrap_or(false)
    });

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
            // Persist the logical path for user-facing display (e.g. keep `/tmp` on macOS).
            path: display_path_str,
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
