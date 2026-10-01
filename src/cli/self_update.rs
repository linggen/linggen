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
    /// Hex SHA-256 of the tarball. Older manifests carry none.
    #[serde(default)]
    sha256: Option<String>,
}

/// Check the downloaded tarball against the manifest's digest — the bytes
/// may have come from the mirror rather than GitHub. A manifest without a
/// digest (an older release) passes with a warning; a mismatch fails.
fn verify_tarball(path: &Path, expected: Option<&str>, binary_name: &str) -> Result<()> {
    let Some(expected) = expected.map(str::trim).filter(|s| !s.is_empty()) else {
        println!(
            "[{binary_name}] Warning: release manifest has no sha256; skipping integrity check"
        );
        return Ok(());
    };
    let got = crate::runtime::file_sha256(path)?;
    if !got.eq_ignore_ascii_case(expected) {
        anyhow::bail!("[{binary_name}] Download failed its integrity check (sha256 {got}, expected {expected})");
    }
    Ok(())
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
        // Release assets are ~70 MB. A single OVERALL `.timeout()` strangles the
        // download on a slow link — 60s wasn't enough at ~1 MB/s, so the body
        // read tripped "operation timed out" mid-transfer. Use a connect timeout
        // (fail fast on a dead host) plus a read/stall timeout (fail only if the
        // stream goes quiet), so a slow-but-progressing transfer of any size
        // completes instead of being killed by a wall-clock cap. The connect
        // timeout stays short so a blocked GitHub hands over to the mirror fast.
        .connect_timeout(crate::mirror::CONNECT_TIMEOUT)
        .read_timeout(Duration::from_secs(60))
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
    // GitHub first, linggen.dev's /dl mirror when GitHub is unreachable.
    let manifest = match crate::mirror::get_bytes(client, manifest_url).await {
        Ok(body) => match serde_json::from_slice::<ReleaseManifest>(&body) {
            Ok(m) => m,
            Err(e) => {
                println!("[{}] Failed to parse release manifest: {}", binary_name, e);
                return Ok(());
            }
        },
        Err(e) => {
            println!(
                "[{}] Failed to fetch release manifest: {:#}",
                binary_name, e
            );
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

    let bytes = crate::mirror::get_bytes(client, &asset.url)
        .await
        .with_context(|| format!("[{}] Failed to download release asset", binary_name))?;

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
    if let Err(e) = verify_tarball(&tarball_path, asset.sha256.as_deref(), binary_name) {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err(e);
    }

    let output = std::process::Command::new("tar")
        .args([
            "xzf",
            &tarball_path.to_string_lossy(),
            "-C",
            &temp_dir.to_string_lossy(),
        ])
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
        Some(cv) => {
            crate::telemetry::global().bump("update.ok");
            println!("[{}] Updated v{} -> v{}", binary_name, cv, manifest.version);
            println!(
                "\n  Tip: run `ling init` to update agents and skills to match the new version."
            );
        }
        None => {
            println!(
                "[{}] Installed v{} at {}",
                binary_name,
                manifest.version,
                target_path.display()
            );
            println!("\n  Tip: run `ling init` to set up agents, skills, and default config.");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tarball(body: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("download.tar.gz");
        std::fs::write(&path, body).unwrap();
        (dir, path)
    }

    #[test]
    fn a_matching_digest_passes_and_a_wrong_one_aborts() {
        let (_d, path) = tarball(b"abc");
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify_tarball(&path, Some(abc), "ling").is_ok());
        assert!(verify_tarball(&path, Some(&abc.to_uppercase()), "ling").is_ok());
        assert!(verify_tarball(&path, Some(&"0".repeat(64)), "ling").is_err());
    }

    #[test]
    fn an_old_manifest_without_a_digest_still_installs() {
        let (_d, path) = tarball(b"abc");
        assert!(verify_tarball(&path, None, "ling").is_ok());
        assert!(verify_tarball(&path, Some(""), "ling").is_ok());
        let m: ReleaseManifest = serde_json::from_str(
            r#"{"version":"1.8.2","assets":[{"name":"ling-macos-aarch64","url":"u"}]}"#,
        )
        .unwrap();
        assert!(m.assets[0].sha256.is_none());
    }
}
