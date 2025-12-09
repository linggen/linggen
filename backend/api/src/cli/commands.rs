use anyhow::{Context, Result};
use colored::*;
use std::path::PathBuf;
use std::time::Duration;
use tabled::{Table, Tabled};

use super::client::{ApiClient, CreateSourceRequest};

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
            println!("\n{}", "To start Linggen:".yellow());
            println!("  1. Navigate to the frontend directory");
            println!("  2. Run: npm run tauri dev");
            println!("     OR");
            println!("     Start the backend and frontend separately:");
            println!("     - Backend: cargo run --bin linggen-api");
            println!("     - Frontend: npm run dev");
            println!("\nError details: {}", e);
            anyhow::bail!("Backend not reachable");
        }
    }
}

/// Handle the `index` command
pub async fn handle_index(
    api_client: &ApiClient,
    path: PathBuf,
    mode: String,
    name: Option<String>,
    include_patterns: Vec<String>,
    exclude_patterns: Vec<String>,
    wait: bool,
) -> Result<()> {
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
    let poll_interval = Duration::from_secs(2);

    loop {
        tokio::time::sleep(poll_interval).await;

        let jobs = api_client.list_jobs().await?;
        let job = jobs.jobs.iter().find(|j| j.id == job_id);

        match job {
            Some(job) => {
                // Only print if status changed
                if job.status != last_status {
                    match job.status.as_str() {
                        "Pending" => println!("   Status: {}", "Pending...".yellow()),
                        "Running" => {
                            let progress = if let (Some(indexed), Some(total)) =
                                (job.files_indexed, job.total_files)
                            {
                                format!(" ({}/{})", indexed, total)
                            } else {
                                String::new()
                            };
                            println!("   Status: {}{}", "Running...".cyan(), progress);
                        }
                        "Completed" => {
                            println!("{}", "✅ Job completed successfully!".green().bold());
                            if let Some(files) = job.files_indexed {
                                println!("   Files indexed: {}", files);
                            }
                            if let Some(chunks) = job.chunks_created {
                                println!("   Chunks created: {}", chunks);
                            }
                            return Ok(());
                        }
                        "Failed" => {
                            println!("{}", "❌ Job failed".red().bold());
                            if let Some(error) = &job.error {
                                println!("   Error: {}", error.red());
                            }
                            anyhow::bail!("Indexing job failed");
                        }
                        _ => println!("   Status: {}", job.status),
                    }
                    last_status = job.status.clone();
                }
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
    let status = api_client.get_status().await?;
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
