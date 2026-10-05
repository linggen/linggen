//! `ling install` / `ling update` / `ling update --rollback`.
//!
//! Releases come from GitHub (linggen.dev's `/dl` mirror as fallback), or —
//! when `LINGGEN_RELEASE_BASE` is set — from one override base laid out as
//! `<base>/<owner>/<repo>/<asset>` (the release gate's local mirror of a
//! draft). The sha256 check applies either way; an override must carry it.
//!
//! An update keeps the previous binary as `ling.prev`, swaps the new one in
//! by rename (a fresh inode, never a copy over a live file), and keeps it
//! only if `ling --version` answers in time — else the previous one is put
//! back. `--rollback` swaps `ling` and `ling.prev` by hand.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::engine_restart;

/// Env var naming a release mirror that replaces GitHub for every
/// installer and updater (see doc/cli.md § Release override).
pub const RELEASE_BASE_ENV: &str = "LINGGEN_RELEASE_BASE";

const REPO: &str = "linggen/linggen";

/// How long a freshly installed binary has to answer `--version`.
const START_TIMEOUT: Duration = Duration::from_secs(10);

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

/// Where release files come from.
#[derive(Clone, Debug, PartialEq)]
pub enum ReleaseSource {
    /// GitHub's latest release, linggen.dev `/dl` when GitHub is unreachable.
    GitHub,
    /// `LINGGEN_RELEASE_BASE`: `<base>/<owner>/<repo>/<asset>`, nothing else.
    Override(String),
}

impl ReleaseSource {
    pub fn from_env() -> Self {
        Self::from_value(std::env::var(RELEASE_BASE_ENV).ok().as_deref())
    }

    fn from_value(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().trim_end_matches('/')) {
            Some(base) if !base.is_empty() => Self::Override(base.to_string()),
            _ => Self::GitHub,
        }
    }

    pub fn manifest_url(&self) -> String {
        match self {
            Self::GitHub => {
                format!("https://github.com/{REPO}/releases/latest/download/manifest.json")
            }
            Self::Override(base) => format!("{base}/{REPO}/manifest.json"),
        }
    }

    /// The manifest's asset URL points at GitHub; an override serves the
    /// same file name from its own directory.
    fn asset_url(&self, url: &str) -> String {
        match self {
            Self::GitHub => url.to_string(),
            Self::Override(base) => {
                let name = url.rsplit('/').next().unwrap_or(url);
                format!("{base}/{REPO}/{name}")
            }
        }
    }

    fn is_override(&self) -> bool {
        matches!(self, Self::Override(_))
    }

    async fn get(&self, client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
        if !self.is_override() {
            return crate::mirror::get_bytes(client, url).await;
        }
        let resp = client.get(url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("{url}: HTTP {}", resp.status());
        }
        Ok(resp.bytes().await?.to_vec())
    }
}

/// Check the downloaded tarball against the manifest's digest — the bytes
/// may have come from a mirror rather than GitHub. A GitHub manifest without
/// a digest (an older release) passes with a warning; an override must carry
/// one; a mismatch fails.
fn verify_tarball(
    path: &Path,
    expected: Option<&str>,
    require: bool,
    binary_name: &str,
) -> Result<()> {
    let Some(expected) = expected.map(str::trim).filter(|s| !s.is_empty()) else {
        if require {
            anyhow::bail!("[{binary_name}] The release manifest has no sha256; refusing an unverified install");
        }
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

fn exe_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("Failed to get current executable path")?;
    Ok(exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Failed to get executable parent directory"))?
        .to_path_buf())
}

/// Install/update the ling binary, or put the previous one back. Either
/// way, an engine running from the swapped binary is restarted on the one
/// now in place (`engine_restart`); `engine` gives the configured
/// host/port for an engine started without `--host`/`--port`.
pub async fn run(rollback: bool, engine: (String, u16)) -> Result<()> {
    let dir = exe_dir()?;
    let found = engine_restart::find(&dir.join("ling"), "ling");
    let swapped = if rollback {
        let (from, to) = rollback_binary(&dir, "ling", START_TIMEOUT)?;
        println!("[ling] Rolled back v{from} -> v{to}");
        println!("  Run `ling update --rollback` again to return to v{from}.");
        true
    } else {
        update().await?
    };
    engine_restart::report_others(&found);
    if !swapped {
        return Ok(());
    }
    let opts = engine_restart::Opts::new(engine.0, engine.1);
    engine_restart::restart(&dir, "ling", &found.ours, &opts).await
}

/// Fetch and swap in the latest release; Ok(true) when the binary changed.
async fn update() -> Result<bool> {
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

    let source = ReleaseSource::from_env();
    if let ReleaseSource::Override(base) = &source {
        println!("Release source: {base} ({RELEASE_BASE_ENV})");
    }
    println!("Current ling version: v{}", current_version);
    update_binary(&client, &source, "ling", Some(current_version), None).await
}

/// A fetch problem: the default path reports it and exits cleanly (as it
/// always has); an override is a test run, so it fails loudly.
fn soft_fail(source: &ReleaseSource, msg: String) -> Result<bool> {
    if source.is_override() {
        anyhow::bail!(msg);
    }
    println!("{msg}");
    Ok(false)
}

async fn update_binary(
    client: &reqwest::Client,
    source: &ReleaseSource,
    binary_name: &str,
    current_version: Option<&str>,
    install_dir: Option<&Path>,
) -> Result<bool> {
    let manifest_url = source.manifest_url();
    let manifest = match source.get(client, &manifest_url).await {
        Ok(body) => match serde_json::from_slice::<ReleaseManifest>(&body) {
            Ok(m) => m,
            Err(e) => {
                return soft_fail(
                    source,
                    format!("[{binary_name}] Failed to parse release manifest: {e}"),
                )
            }
        },
        Err(e) => {
            return soft_fail(
                source,
                format!("[{binary_name}] Failed to fetch release manifest: {e:#}"),
            )
        }
    };

    if let Some(cv) = current_version {
        if manifest.version == cv {
            println!("[{}] Already up to date (v{}).", binary_name, cv);
            return Ok(false);
        }
    }

    let slug = platform_slug();
    let asset_name = format!("{}-{}", binary_name, slug);
    let Some(asset) = manifest.assets.iter().find(|a| a.name == asset_name) else {
        let available = manifest
            .assets
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return soft_fail(
            source,
            format!("[{binary_name}] No release asset for '{asset_name}'. Available: {available}"),
        );
    };

    println!("[{}] Downloading v{} ...", binary_name, manifest.version);

    let bytes = source
        .get(client, &source.asset_url(&asset.url))
        .await
        .with_context(|| format!("[{}] Failed to download release asset", binary_name))?;

    let target_dir = match install_dir {
        Some(dir) => dir.to_path_buf(),
        None => exe_dir()?,
    };
    let digest = Verify {
        sha256: asset.sha256.as_deref(),
        required: source.is_override(),
    };
    let target_path = install_tarball(&bytes, digest, &target_dir, binary_name, START_TIMEOUT)?;

    match current_version {
        Some(cv) => {
            crate::telemetry::global().bump("update.ok");
            println!("[{}] Updated v{} -> v{}", binary_name, cv, manifest.version);
            println!(
                "  The previous binary is kept as {binary_name}.prev — `ling update --rollback` returns to it."
            );
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

    Ok(true)
}

struct Verify<'a> {
    sha256: Option<&'a str>,
    required: bool,
}

fn prev_path(dir: &Path, binary_name: &str) -> PathBuf {
    dir.join(format!("{binary_name}.prev"))
}

/// Verify, extract and swap in a release tarball; returns the installed
/// path. Any failure leaves the binary that was there before in place.
fn install_tarball(
    bytes: &[u8],
    digest: Verify<'_>,
    dir: &Path,
    binary_name: &str,
    timeout: Duration,
) -> Result<PathBuf> {
    let temp_dir = dir.join(format!(".{}-extract-{}", binary_name, std::process::id()));
    std::fs::create_dir_all(&temp_dir).context("Failed to create the extract dir")?;
    let result = stage_and_swap(bytes, digest, dir, &temp_dir, binary_name, timeout);
    let _ = std::fs::remove_dir_all(&temp_dir);
    result
}

fn stage_and_swap(
    bytes: &[u8],
    digest: Verify<'_>,
    dir: &Path,
    temp_dir: &Path,
    binary_name: &str,
    timeout: Duration,
) -> Result<PathBuf> {
    let tarball_path = temp_dir.join("download.tar.gz");
    std::fs::write(&tarball_path, bytes).context("Failed to write temp tarball")?;
    verify_tarball(&tarball_path, digest.sha256, digest.required, binary_name)?;

    let output = std::process::Command::new("tar")
        .arg("xzf")
        .arg(&tarball_path)
        .arg("-C")
        .arg(temp_dir)
        .output()
        .context("Failed to run tar to extract binary")?;
    if !output.status.success() {
        anyhow::bail!(
            "[{}] Failed to extract tarball: {}",
            binary_name,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let extracted = temp_dir.join(binary_name);
    if !extracted.is_file() {
        anyhow::bail!("[{}] Binary not found in tarball", binary_name);
    }
    set_executable(&extracted)?;

    let target = dir.join(binary_name);
    let prev = prev_path(dir, binary_name);
    commit_with_probe(&extracted, &target, &prev, binary_name, timeout)?;
    Ok(target)
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// Move `target` aside to `prev`, rename `new` into place, and keep it only
/// if it answers `--version` in time; otherwise put `prev` back.
fn commit_with_probe(
    new: &Path,
    target: &Path,
    prev: &Path,
    binary_name: &str,
    timeout: Duration,
) -> Result<()> {
    let had_previous = target.exists();
    if had_previous {
        let _ = std::fs::remove_file(prev);
        std::fs::rename(target, prev).with_context(|| {
            format!(
                "[{binary_name}] Failed to keep the previous binary at {}",
                prev.display()
            )
        })?;
    }
    if let Err(e) = std::fs::rename(new, target) {
        if had_previous {
            let _ = std::fs::rename(prev, target);
        }
        return Err(e).context(format!("[{binary_name}] Failed to install binary"));
    }
    if let Err(e) = probe_version(target, binary_name, timeout) {
        let _ = std::fs::remove_file(target);
        if had_previous {
            std::fs::rename(prev, target).with_context(|| {
                format!("[{binary_name}] The new binary failed to start ({e:#}) and restoring {} failed", prev.display())
            })?;
            anyhow::bail!("[{binary_name}] The new binary failed to start ({e:#}); the previous one is restored");
        }
        anyhow::bail!("[{binary_name}] The new binary failed to start ({e:#})");
    }
    Ok(())
}

/// Run `<path> --version`; Ok(version) when it exits 0 within `timeout`
/// and prints `<binary_name> <version>`.
pub(crate) fn probe_version(path: &Path, binary_name: &str, timeout: Duration) -> Result<String> {
    let mut child = std::process::Command::new(path)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("could not run {}", path.display()))?;
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!(
                "`--version` gave no answer within {}s",
                timeout.as_secs_f32()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        anyhow::bail!("`--version` exited with {}", out.status);
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .trim()
        .strip_prefix(&format!("{binary_name} "))
        .map(|v| v.trim().to_string())
        .ok_or_else(|| anyhow::anyhow!("`--version` printed {:?}", stdout.trim()))
}

/// Swap `<dir>/<name>` with `<dir>/<name>.prev`, so a second rollback
/// returns. Keeps the swap only if the restored binary answers `--version`.
/// Returns (from, to) versions.
pub(crate) fn rollback_binary(
    dir: &Path,
    binary_name: &str,
    timeout: Duration,
) -> Result<(String, String)> {
    let target = dir.join(binary_name);
    let prev = prev_path(dir, binary_name);
    if !prev.is_file() {
        anyhow::bail!(
            "[{binary_name}] No previous binary kept at {} — nothing to roll back to",
            prev.display()
        );
    }
    let from = probe_version(&target, binary_name, timeout).unwrap_or_else(|_| "?".into());
    let aside = dir.join(format!(".{binary_name}.rollback-{}", std::process::id()));
    std::fs::rename(&target, &aside).context("Failed to move the current binary aside")?;
    if let Err(e) = std::fs::rename(&prev, &target) {
        let _ = std::fs::rename(&aside, &target);
        return Err(e).context("Failed to restore the previous binary");
    }
    match probe_version(&target, binary_name, timeout) {
        Ok(to) => {
            std::fs::rename(&aside, &prev).context("Failed to keep the replaced binary")?;
            Ok((from, to))
        }
        Err(e) => {
            let _ = std::fs::rename(&target, &prev);
            let _ = std::fs::rename(&aside, &target);
            anyhow::bail!(
                "[{binary_name}] The previous binary failed to start ({e:#}); kept the current one"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    const T: Duration = Duration::from_secs(10);

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
        assert!(verify_tarball(&path, Some(abc), false, "ling").is_ok());
        assert!(verify_tarball(&path, Some(&abc.to_uppercase()), false, "ling").is_ok());
        assert!(verify_tarball(&path, Some(&"0".repeat(64)), false, "ling").is_err());
    }

    #[test]
    fn an_old_manifest_without_a_digest_still_installs() {
        let (_d, path) = tarball(b"abc");
        assert!(verify_tarball(&path, None, false, "ling").is_ok());
        assert!(verify_tarball(&path, Some(""), false, "ling").is_ok());
        let m: ReleaseManifest = serde_json::from_str(
            r#"{"version":"1.8.2","assets":[{"name":"ling-macos-aarch64","url":"u"}]}"#,
        )
        .unwrap();
        assert!(m.assets[0].sha256.is_none());
    }

    #[test]
    fn an_override_without_a_digest_is_refused() {
        let (_d, path) = tarball(b"abc");
        assert!(verify_tarball(&path, None, true, "ling").is_err());
    }

    #[test]
    fn the_override_replaces_github_and_keeps_asset_names() {
        assert_eq!(ReleaseSource::from_value(None), ReleaseSource::GitHub);
        assert_eq!(ReleaseSource::from_value(Some("  ")), ReleaseSource::GitHub);
        assert_eq!(
            ReleaseSource::GitHub.manifest_url(),
            "https://github.com/linggen/linggen/releases/latest/download/manifest.json"
        );
        let gh =
            "https://github.com/linggen/linggen/releases/download/1.9.0/ling-macos-aarch64.tar.gz";
        assert_eq!(ReleaseSource::GitHub.asset_url(gh), gh);

        let o = ReleaseSource::from_value(Some("http://10.0.0.2:8765/"));
        assert_eq!(o, ReleaseSource::Override("http://10.0.0.2:8765".into()));
        assert_eq!(
            o.manifest_url(),
            "http://10.0.0.2:8765/linggen/linggen/manifest.json"
        );
        assert_eq!(
            o.asset_url(gh),
            "http://10.0.0.2:8765/linggen/linggen/ling-macos-aarch64.tar.gz"
        );
    }

    // ── install / rollback on temp dirs ────────────────────────────────

    /// A fake `ling` whose `--version` runs `body`.
    fn script(path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        set_executable(path).unwrap();
    }

    fn says(version: &str) -> String {
        format!("echo 'ling {version}'")
    }

    /// A release tarball holding a fake `ling`; returns (bytes, sha256).
    fn release(body: &str) -> (Vec<u8>, String) {
        let d = tempfile::tempdir().unwrap();
        script(&d.path().join("ling"), body);
        let tgz = d.path().join("out.tar.gz");
        let ok = std::process::Command::new("tar")
            .arg("czf")
            .arg(&tgz)
            .arg("-C")
            .arg(d.path())
            .arg("ling")
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let sha = crate::runtime::file_sha256(&tgz).unwrap();
        (std::fs::read(&tgz).unwrap(), sha)
    }

    /// An install dir holding an old `ling` that answers 1.0.0.
    fn installed() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        script(&d.path().join("ling"), &says("1.0.0"));
        d
    }

    fn version_in(dir: &Path, name: &str) -> String {
        probe_version(&dir.join(name), "ling", T).unwrap()
    }

    /// Serves `files` (path → body) on 127.0.0.1; 404 for anything else.
    fn serve(files: Vec<(String, Vec<u8>)>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
                let (status, body) = files
                    .iter()
                    .find(|(p, _)| *p == path)
                    .map(|(_, b)| ("200 OK", b.clone()))
                    .unwrap_or(("404 Not Found", Vec::new()));
                let head = format!(
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            }
        });
        format!("http://{addr}")
    }

    /// A mirror serving one release, laid out as the gate builds it.
    fn mirror(body: &str, sha: Option<String>) -> ReleaseSource {
        let (bytes, real) = release(body);
        let asset = format!("ling-{}", platform_slug());
        let manifest = serde_json::json!({
            "version": "9.9.9",
            "assets": [{
                "name": asset,
                "url": format!("https://github.com/linggen/linggen/releases/download/9.9.9/{asset}.tar.gz"),
                "sha256": sha.unwrap_or(real),
            }],
        });
        let base = serve(vec![
            (
                "/linggen/linggen/manifest.json".into(),
                manifest.to_string().into_bytes(),
            ),
            (format!("/linggen/linggen/{asset}.tar.gz"), bytes),
        ]);
        ReleaseSource::from_value(Some(&base))
    }

    async fn update(source: &ReleaseSource, dir: &Path) -> Result<()> {
        let client = reqwest::Client::new();
        update_binary(&client, source, "ling", Some("1.0.0"), Some(dir))
            .await
            .map(|_| ())
    }

    #[tokio::test]
    async fn an_update_from_the_override_installs_and_keeps_the_previous() {
        let dir = installed();
        update(&mirror(&says("9.9.9"), None), dir.path())
            .await
            .unwrap();
        assert_eq!(version_in(dir.path(), "ling"), "9.9.9");
        assert_eq!(version_in(dir.path(), "ling.prev"), "1.0.0");
    }

    #[tokio::test]
    async fn a_wrong_sha256_is_refused_and_the_old_binary_kept() {
        let dir = installed();
        let err = update(&mirror(&says("9.9.9"), Some("0".repeat(64))), dir.path())
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("integrity"), "{err:#}");
        assert_eq!(version_in(dir.path(), "ling"), "1.0.0");
        assert!(!dir.path().join("ling.prev").exists());
    }

    #[tokio::test]
    async fn a_new_binary_that_fails_to_start_is_rolled_back() {
        let dir = installed();
        let err = update(&mirror("exit 1", None), dir.path())
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("restored"), "{err:#}");
        assert_eq!(version_in(dir.path(), "ling"), "1.0.0");
    }

    #[tokio::test]
    async fn an_unreachable_override_fails_instead_of_falling_back() {
        let dir = installed();
        let gone = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", gone.local_addr().unwrap());
        drop(gone);
        assert!(update(&ReleaseSource::from_value(Some(&base)), dir.path())
            .await
            .is_err());
        assert_eq!(version_in(dir.path(), "ling"), "1.0.0");
    }

    #[test]
    fn a_new_binary_that_hangs_is_rolled_back() {
        let dir = installed();
        let (bytes, sha) = release("sleep 30");
        let digest = Verify {
            sha256: Some(&sha),
            required: true,
        };
        let short = Duration::from_millis(300);
        assert!(install_tarball(&bytes, digest, dir.path(), "ling", short).is_err());
        assert_eq!(version_in(dir.path(), "ling"), "1.0.0");
    }

    #[test]
    fn rollback_swaps_back_and_forth() {
        let dir = installed();
        let (bytes, sha) = release(&says("9.9.9"));
        let digest = Verify {
            sha256: Some(&sha),
            required: true,
        };
        install_tarball(&bytes, digest, dir.path(), "ling", T).unwrap();
        assert_eq!(
            rollback_binary(dir.path(), "ling", T).unwrap(),
            ("9.9.9".into(), "1.0.0".into())
        );
        assert_eq!(version_in(dir.path(), "ling.prev"), "9.9.9");
        rollback_binary(dir.path(), "ling", T).unwrap();
        assert_eq!(version_in(dir.path(), "ling"), "9.9.9");
    }

    #[test]
    fn rollback_without_a_kept_binary_refuses() {
        let dir = installed();
        assert!(rollback_binary(dir.path(), "ling", T).is_err());
        assert_eq!(version_in(dir.path(), "ling"), "1.0.0");
    }

    #[test]
    fn a_broken_previous_binary_is_not_rolled_back_to() {
        let dir = installed();
        script(&dir.path().join("ling.prev"), "exit 1");
        assert!(rollback_binary(dir.path(), "ling", T).is_err());
        assert_eq!(version_in(dir.path(), "ling"), "1.0.0");
        assert!(dir.path().join("ling.prev").exists());
    }
}
