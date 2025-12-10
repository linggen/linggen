use std::{collections::HashMap, time::Duration};

use anyhow::{Context, Result};
use serde::Deserialize;

const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/linggen/linggen-releases/releases/latest/download/manifest.json";

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

#[derive(Debug, Clone, Copy)]
pub enum Platform {
    Mac,
    Linux,
    Other,
}

#[derive(Debug, Clone, Copy)]
pub enum ArtifactKind {
    Cli,
    App,
    Server,
}

pub async fn fetch_manifest() -> Result<Manifest> {
    let url = std::env::var("LINGGEN_MANIFEST_URL").unwrap_or_else(|_| DEFAULT_MANIFEST_URL.into());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("Failed to build HTTP client")?;

    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("Failed to GET manifest from {url}"))?;

    if !resp.status().is_success() {
        anyhow::bail!("Manifest request failed: {} {}", resp.status(), url);
    }

    let manifest = resp.json::<Manifest>().await.context("Failed to parse manifest JSON")?;
    Ok(manifest)
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

pub fn select_artifact(manifest: &Manifest, platform: Platform, kind: ArtifactKind) -> Option<Artifact> {
    let key = match (platform, kind) {
        (Platform::Mac, ArtifactKind::Cli) => "cli-macos-universal",
        (Platform::Mac, ArtifactKind::App) => "app-macos-dmg",
        (Platform::Mac, ArtifactKind::Server) => "server-macos-universal",
        (Platform::Linux, ArtifactKind::Cli) => "cli-linux-x86_64",
        (Platform::Linux, ArtifactKind::Server) => "server-linux-x86_64",
        (Platform::Linux, ArtifactKind::App) => "app-linux",
        _ => "",
    };
    manifest.artifacts.get(key).cloned()
}
