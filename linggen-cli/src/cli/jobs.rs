use anyhow::Result;
use colored::*;
use std::time::Duration;

use super::client::ApiClient;

/// Wait for a job to complete by polling.
pub async fn wait_for_job(api_client: &ApiClient, job_id: &str) -> Result<()> {
    let mut last_status = String::new();
    let mut last_progress = (0, 0); // (files_indexed, total_files)
    let poll_interval = Duration::from_secs(1); // Poll more frequently for better UX

    loop {
        tokio::time::sleep(poll_interval).await;

        let jobs = api_client.list_jobs().await?;
        let job = jobs.jobs.iter().find(|j| j.id == job_id);

        match job {
            Some(job) => {
                let current_progress = (job.files_indexed.unwrap_or(0), job.total_files.unwrap_or(0));
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

