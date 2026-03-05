use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

#[derive(Deserialize)]
struct ReleaseManifest {
    version: String,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    url: String,
}

/// Returns platform slug matching the release script convention, e.g. "macos-aarch64", "linux-x86_64".
fn platform_slug() -> &'static str {
    if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "macos-aarch64"
        } else {
            "macos-x86_64"
        }
    } else if cfg!(target_os = "linux") {
        if cfg!(target_arch = "aarch64") {
            "linux-aarch64"
        } else {
            "linux-x86_64"
        }
    } else {
        "unknown"
    }
}

/// Install/update the ling binary.
pub async fn run() -> Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");

    let client = reqwest::Client::builder()
        .user_agent("linggen")
        .timeout(Duration::from_secs(60))
        .build()
        .context("Failed to build HTTP client")?;

    println!("Current ling version: v{}", current_version);
    update_binary(
        &client,
        "ling",
        "https://github.com/linggen/linggen/releases/latest/download/manifest.json",
        Some(current_version),
        None,
    )
    .await?;

    Ok(())
}

async fn update_binary(
    client: &reqwest::Client,
    binary_name: &str,
    manifest_url: &str,
    current_version: Option<&str>,
    install_dir: Option<&Path>,
) -> Result<()> {
    let manifest = match client.get(manifest_url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<ReleaseManifest>().await {
            Ok(m) => m,
            Err(e) => {
                println!("[{}] Failed to parse release manifest: {}", binary_name, e);
                return Ok(());
            }
        },
        Ok(resp) => {
            println!(
                "[{}] Release manifest returned HTTP {}. No releases available yet.",
                binary_name,
                resp.status()
            );
            return Ok(());
        }
        Err(e) => {
            println!("[{}] Failed to fetch release manifest: {}", binary_name, e);
            return Ok(());
        }
    };

    if let Some(cv) = current_version {
        if manifest.version == cv {
            println!("[{}] Already up to date (v{}).", binary_name, cv);
            return Ok(());
        }
    }

    let slug = platform_slug();
    let asset_name = format!("{}-{}", binary_name, slug);
    let asset = match manifest.assets.iter().find(|a| a.name == asset_name) {
        Some(a) => a,
        None => {
            println!(
                "[{}] No release asset for '{}'. Available: {}",
                binary_name,
                asset_name,
                manifest
                    .assets
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            return Ok(());
        }
    };

    println!("[{}] Downloading v{} ...", binary_name, manifest.version);

    let resp = client
        .get(&asset.url)
        .send()
        .await
        .context("Failed to download release asset")?;

    if !resp.status().is_success() {
        anyhow::bail!("[{}] Download failed with HTTP {}", binary_name, resp.status());
    }

    let bytes = resp.bytes().await.context("Failed to read download")?;

    let target_dir = if let Some(dir) = install_dir {
        dir.to_path_buf()
    } else {
        let exe = std::env::current_exe().context("Failed to get current executable path")?;
        exe.parent()
            .ok_or_else(|| anyhow::anyhow!("Failed to get executable parent directory"))?
            .to_path_buf()
    };

    let target_path = target_dir.join(binary_name);

    // The release asset is a .tar.gz containing the binary — extract it
    let temp_dir = target_dir.join(format!(".{}-extract-{}", binary_name, std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let tarball_path = temp_dir.join("download.tar.gz");
    std::fs::write(&tarball_path, &bytes).context("Failed to write temp tarball")?;

    let output = std::process::Command::new("tar")
        .args(["xzf", &tarball_path.to_string_lossy(), "-C", &temp_dir.to_string_lossy()])
        .output()
        .context("Failed to run tar to extract binary")?;

    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&temp_dir);
        anyhow::bail!(
            "[{}] Failed to extract tarball: {}",
            binary_name,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let extracted_binary = temp_dir.join(binary_name);
    if !extracted_binary.exists() {
        let _ = std::fs::remove_dir_all(&temp_dir);
        anyhow::bail!("[{}] Binary not found in tarball", binary_name);
    }

    std::fs::rename(&extracted_binary, &target_path).context("Failed to install binary")?;
    let _ = std::fs::remove_dir_all(&temp_dir);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target_path, std::fs::Permissions::from_mode(0o755))?;
    }

    match current_version {
        Some(cv) => println!("[{}] Updated v{} -> v{}", binary_name, cv, manifest.version),
        None => println!("[{}] Installed v{} at {}", binary_name, manifest.version, target_path.display()),
    }

    Ok(())
}
