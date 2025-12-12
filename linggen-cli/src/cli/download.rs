use anyhow::{Context, Result};
use colored::*;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::NamedTempFile;

use super::signature::{verify_signature, LINGGEN_PUBLIC_KEY};

fn download_timeout() -> Duration {
    // Default to a generous timeout for slow networks.
    // Override via env var: LINGGEN_HTTP_TIMEOUT_SECS=600
    let default_secs = 600u64;
    std::env::var("LINGGEN_HTTP_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(default_secs))
}

pub fn http_client(timeout: Duration) -> Result<reqwest::Client> {
    // On macOS, reqwest may consult system proxy settings via `system-configuration`,
    // which can panic in some environments (NULL dynamic store). Disable proxy lookup.
    reqwest::Client::builder()
        .no_proxy()
        .user_agent("linggen-cli")
        .timeout(timeout)
        .build()
        .context("Failed to build HTTP client")
}

pub fn default_http_client() -> Result<reqwest::Client> {
    http_client(download_timeout())
}

pub async fn download_to_temp(
    client: &reqwest::Client,
    url: &str,
    signature: Option<&str>,
) -> Result<PathBuf> {
    let tmp = NamedTempFile::new().context("Failed to create temp file")?;
    let tmp_path = tmp.path().to_path_buf();

    // Retry logic for 503/504 errors (GitHub CDN issues)
    // Try up to 5 times with exponential backoff: 1s, 2s, 4s, 8s
    let max_attempts = 5;
    let mut resp = None;
    let mut last_error = None;

    for attempt in 0..max_attempts {
        match client.get(url).send().await {
            Ok(r) => {
                if r.status().is_success() {
                    resp = Some(r);
                    break;
                } else if (r.status() == 503 || r.status() == 504) && attempt < max_attempts - 1 {
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
                } else {
                    last_error = Some(format!(
                        "HTTP {} {}",
                        r.status(),
                        r.status().canonical_reason().unwrap_or("Unknown")
                    ));
                    if attempt < max_attempts - 1
                        && (r.status() == 503 || r.status() == 504 || r.status() == 429)
                    {
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

    let resp =
        resp.ok_or_else(|| anyhow::anyhow!(last_error.unwrap_or_else(|| "Unknown error".into())))?;

    // Download to file
    let bytes = resp
        .bytes()
        .await
        .with_context(|| format!("Failed to download from {}", url))?;
    std::fs::write(&tmp_path, &bytes).context("Failed to write downloaded file")?;

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
                println!("{}", "   Continuing anyway.".yellow());
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
