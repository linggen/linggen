use std::{collections::HashMap, time::Duration};

use anyhow::{Context, Result};
use colored::Colorize;
use serde::Deserialize;

const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/linggen/linggen/releases/latest/download/manifest.json";
const REPO_BASE_URL: &str = "https://github.com/linggen/linggen/releases";

#[derive(Debug, Deserialize, Clone)]
pub struct Manifest {
    pub version: Option<String>,
    pub artifacts: HashMap<String, Artifact>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Artifact {
    pub url: String,
    pub sha256: Option<String>,
    pub signature: Option<String>,
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Linux,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    Cli,
    App,
    Server,
}

pub async fn fetch_manifest(version: Option<&str>) -> Result<Manifest> {
    let url = if let Some(ver) = version {
        // Construct versioned URL: /download/v0.2.1/manifest.json
        let version_tag = if ver.starts_with('v') {
            ver.to_string()
        } else {
            format!("v{}", ver)
        };
        format!("{}/download/{}/manifest.json", REPO_BASE_URL, version_tag)
    } else {
        std::env::var("LINGGEN_MANIFEST_URL").unwrap_or_else(|_| DEFAULT_MANIFEST_URL.into())
    };
    // On macOS, reqwest may consult system proxy settings via `system-configuration`.
    // In some environments this can fail (NULL dynamic store). Disable proxy lookup.
    let client = reqwest::Client::builder()
        .no_proxy()
        .user_agent("linggen-cli")
        .timeout(Duration::from_secs(30)) // Increased timeout for slow CDN
        .build()
        .context("Failed to build HTTP client")?;

    // Retry logic for network errors (504, 503, timeouts, etc.)
    let max_attempts = 5;
    let mut last_error = None;

    for attempt in 0..max_attempts {
        match client.get(&url).send().await {
            Ok(resp) => {
                // Success - parse and return
                if resp.status().is_success() {
                    let manifest = resp
                        .json::<Manifest>()
                        .await
                        .context("Failed to parse manifest JSON")?;
                    return Ok(manifest);
                }

                // If /latest/download/ fails with 503/504/404, fetch actual latest tag from GitHub API and retry
                if !resp.status().is_success()
                    && version.is_none()
                    && url.contains("/latest/download/")
                {
                    if resp.status() == 503 || resp.status() == 504 || resp.status() == 404 {
                        println!(
                            "{}",
                            format!(
                                "⚠️  /latest/download/ returned {} - fetching actual latest release tag...",
                                resp.status()
                            )
                            .yellow()
                        );

                        // Fetch latest release tag from GitHub API
                        // Use /releases (all releases) and get the first one (most recent) to avoid API cache issues
                        let api_url = "https://api.github.com/repos/linggen/linggen/releases";
                        let api_resp = client
                            .get(api_url)
                            .header("User-Agent", "linggen-cli")
                            .send()
                            .await
                            .context("Failed to fetch releases from GitHub API")?;

                        if api_resp.status().is_success() {
                            #[derive(serde::Deserialize)]
                            struct Release {
                                tag_name: String,
                                draft: bool,
                            }
                            let releases: Vec<Release> = api_resp
                                .json()
                                .await
                                .context("Failed to parse GitHub API response")?;

                            // Find the first non-draft release (most recent published release)
                            let release = releases
                                .into_iter()
                                .find(|r| !r.draft)
                                .ok_or_else(|| anyhow::anyhow!("No published releases found"))?;

                            let fallback_url = format!(
                                "{}/download/{}/manifest.json",
                                REPO_BASE_URL, release.tag_name
                            );
                            println!(
                                "{}",
                                format!("   Using versioned URL: {}", fallback_url).yellow()
                            );

                            // Try the versioned URL with retry logic
                            for fallback_attempt in 0..3 {
                                match client.get(&fallback_url).send().await {
                                    Ok(fallback_resp) => {
                                        if fallback_resp.status().is_success() {
                                            let manifest =
                                                fallback_resp
                                                    .json::<Manifest>()
                                                    .await
                                                    .context("Failed to parse manifest JSON")?;
                                            return Ok(manifest);
                                        } else if fallback_resp.status() == 503
                                            || fallback_resp.status() == 504
                                        {
                                            if fallback_attempt < 2 {
                                                let delay =
                                                    Duration::from_secs(1 << fallback_attempt);
                                                println!(
                                                    "{}",
                                                    format!(
                                                        "   Fallback URL returned {} (attempt {}/3), retrying in {}s...",
                                                        fallback_resp.status(),
                                                        fallback_attempt + 1,
                                                        delay.as_secs()
                                                    )
                                                    .yellow()
                                                );
                                                tokio::time::sleep(delay).await;
                                                continue;
                                            }
                                        }
                                        break; // Non-retryable error or max attempts
                                    }
                                    Err(e) => {
                                        if fallback_attempt < 2
                                            && (e.is_timeout() || e.is_request())
                                        {
                                            let delay = Duration::from_secs(1 << fallback_attempt);
                                            println!(
                                                "{}",
                                                format!(
                                                    "   Fallback URL request failed (attempt {}/3), retrying in {}s...",
                                                    fallback_attempt + 1,
                                                    delay.as_secs()
                                                )
                                                .yellow()
                                            );
                                            tokio::time::sleep(delay).await;
                                            continue;
                                        }
                                        break; // Non-retryable error
                                    }
                                }
                            }
                        }
                    }
                }

                // If we got here, the fallback didn't work - check if we should retry the original request
                if resp.status() == 503 || resp.status() == 504 {
                    if attempt < max_attempts - 1 {
                        let delay = Duration::from_secs(1 << attempt);
                        println!(
                            "{}",
                            format!(
                                "⚠️  Request returned {} (attempt {}/{}), retrying in {}s...",
                                resp.status(),
                                attempt + 1,
                                max_attempts,
                                delay.as_secs()
                            )
                            .yellow()
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                }
                // Non-retryable error or max attempts reached
                anyhow::bail!(
                    "Manifest request failed: {} {}\n\
                    This may be a temporary GitHub CDN issue. Please try again in a few moments.",
                    resp.status(),
                    url
                );
            }
            Err(e) => {
                last_error = Some(e.to_string());
                // Check if it's a retryable error (timeout, network error, etc.)
                let is_retryable = e.is_timeout() || e.is_request() || e.is_connect();

                if is_retryable && attempt < max_attempts - 1 {
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
                    // Non-retryable error or max attempts reached
                    anyhow::bail!(
                        "Failed to fetch manifest after {} attempts: {}\n\
                        This may be a temporary GitHub CDN issue. Please try again in a few moments.",
                        max_attempts,
                        e
                    );
                }
            }
        }
    }

    // This should never be reached, but just in case
    anyhow::bail!(
        "Failed to fetch manifest: {}\n\
        This may be a temporary GitHub CDN issue. Please try again in a few moments.",
        last_error.unwrap_or_else(|| "Unknown error".to_string())
    )
}

pub fn current_platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::Mac
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else {
        Platform::Other
    }
}

pub fn select_artifact(
    manifest: &Manifest,
    platform: Platform,
    kind: ArtifactKind,
) -> Option<Artifact> {
    // Detect architecture
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };

    let key = match (platform, kind) {
        (Platform::Mac, ArtifactKind::Cli) => format!("cli-macos-{}", arch),
        (Platform::Mac, ArtifactKind::Server) => "server-macos".to_string(),
        (Platform::Mac, ArtifactKind::App) => "app-macos-tarball".to_string(),
        (Platform::Linux, ArtifactKind::Cli) => format!("cli-linux-{}", arch),
        (Platform::Linux, ArtifactKind::Server) => format!("server-linux-{}", arch),
        _ => "".to_string(),
    };

    // Fallback for universal mac cli if specific arch not found
    if key.starts_with("cli-macos-") && !manifest.artifacts.contains_key(&key) {
        if let Some(art) = manifest.artifacts.get("cli-macos-universal") {
            return Some(art.clone());
        }
    }

    manifest.artifacts.get(&key).cloned()
}
