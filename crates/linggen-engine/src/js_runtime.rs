//! Managed JS runtime — a pinned bun at `~/.linggen/bin/bun`, the runtime
//! skill actions run on (`<skill>/scripts/run-js.sh` looks there first) and
//! the Claude Agent SDK runner needs. The engine owns it: a Mac that got
//! Linggen from the DMG, with no install script and no node, still gets one.
//!
//! [`ensure_in_background`] is spawned on engine start, so startup never
//! waits on it; one fetch runs at a time. The zip comes straight from bun's
//! own GitHub release, pinned by version and checked against the SHA-256
//! pins below — never `latest`. A runtime, like the managed Python
//! (`crate::runtime`), not part of a Linggen release: `LINGGEN_RELEASE_BASE`
//! does not apply, so the release gate's mirror needs no bun asset.
//!
//! Nothing half-written is ever left at the binary's path: the file is
//! unpacked into a staging folder beside it, run once (`--version`), and
//! renamed into place — a new inode, never a write over an existing binary.
//! A failed fetch leaves nothing behind and is tried again next start, or
//! when asked ([`ensure_in_background`] from a page's ask), at most once per
//! [`RETRY_GAP`].

use anyhow::{bail, Context, Result};
use sha2::Digest;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const BUN_VERSION: &str = "1.4.4";

/// (os, arch, asset slug, sha256 of `<slug>.zip`) — bun's SHASUMS256.txt for
/// bun-v1.4.4, matching GitHub's per-asset digest (both read 2026-11-11).
const BUN_ASSETS: [(&str, &str, &str, &str); 4] = [
    (
        "macos",
        "aarch64",
        "bun-darwin-aarch64",
        "c240d7f0ccb90c263a849fcdb09b7ee62630d921348ed6aafda86b9d577bc510",
    ),
    (
        "macos",
        "x86_64",
        "bun-darwin-x64",
        "50a3491f73abc2fe81673171a85762d3bc2cc1680ccecadd983de99adf77977e",
    ),
    (
        "linux",
        "aarch64",
        "bun-linux-aarch64",
        "baa6bb36f51d6e58561e02bbea825038a9df454f673d6d776e8bbf86f8c7850e",
    ),
    (
        "linux",
        "x86_64",
        "bun-linux-x64",
        "bf8ccbfb2b0b80159e0f9a6d15c55317add8077534dbc6130a3edb52fccb395c",
    ),
];

/// A failed fetch is asked again no sooner than this.
const RETRY_GAP: Duration = Duration::from_secs(30);

/// A new binary's first run waits on the macOS scan (8-22 s measured in the
/// release gate VM, `self_update.rs`).
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

static FETCH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static RUNNING: AtomicBool = AtomicBool::new(false);
/// The last failure and when it ended; kept until bun lands, so a retry in
/// flight still reads as failed rather than flipping back to preparing.
static FAILED: Mutex<Option<(String, Instant)>> = Mutex::new(None);

/// Where the managed bun lives.
pub fn bun_bin() -> PathBuf {
    crate::paths::linggen_home().join("bin/bun")
}

/// The runtime as a page or a tool sees it: `ready`, `preparing`,
/// `failed` (with `reason`), or `missing` (no fetch started yet).
pub fn status() -> serde_json::Value {
    let path = bun_bin();
    let failed = FAILED
        .lock()
        .ok()
        .and_then(|f| f.as_ref().map(|(r, _)| r.clone()));
    let state = if path.is_file() {
        "ready"
    } else if failed.is_some() {
        "failed"
    } else if RUNNING.load(Ordering::SeqCst) {
        "preparing"
    } else {
        "missing"
    };
    serde_json::json!({
        "name": "bun",
        "version": BUN_VERSION,
        "state": state,
        "reason": if state == "failed" { failed } else { None },
        "path": path,
    })
}

/// Start a fetch when bun is absent and none is running — the boot call, and
/// an on-demand ask. A recent failure waits out [`RETRY_GAP`] first.
pub fn ensure_in_background() {
    if bun_bin().is_file() || RUNNING.load(Ordering::SeqCst) || failed_recently() {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async {
        if let Err(e) = ensure().await {
            tracing::warn!("[js-runtime] bun not installed: {e:#}");
        }
    });
}

fn failed_recently() -> bool {
    FAILED
        .lock()
        .ok()
        .and_then(|f| f.as_ref().map(|(_, at)| at.elapsed() < RETRY_GAP))
        .unwrap_or(false)
}

/// Ensure bun is in place, fetching it now if not; returns its path.
pub async fn ensure() -> Result<PathBuf> {
    let _one = FETCH.lock().await;
    let dest = bun_bin();
    if dest.is_file() {
        return Ok(dest);
    }
    let result = {
        // Cleared however the fetch ends — a cancelled turn drops it mid-way.
        let _running = Running::start();
        fetch_pinned(&dest).await
    };
    if let Ok(mut failed) = FAILED.lock() {
        *failed = match &result {
            Ok(_) => None,
            Err(e) => Some((format!("{e:#}"), Instant::now())),
        };
    }
    if result.is_ok() {
        tracing::info!(
            "[js-runtime] bun {BUN_VERSION} installed at {}",
            dest.display()
        );
    }
    result
}

struct Running;

impl Running {
    fn start() -> Self {
        RUNNING.store(true, Ordering::SeqCst);
        Running
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

/// A bun to run now: the managed one, else one on PATH. Neither — a fetch is
/// started and the error says where it stands.
pub fn bun_now() -> Result<PathBuf> {
    if let Some(path) = existing_bun() {
        return Ok(path);
    }
    ensure_in_background();
    match status()["reason"].as_str() {
        Some(reason) => bail!("Bun could not be fetched ({reason}); it is tried again shortly."),
        None => bail!("Bun is being prepared; try again in a moment."),
    }
}

/// [`bun_now`], waiting for a fetch instead of failing.
pub async fn bun_ready() -> Result<PathBuf> {
    match existing_bun() {
        Some(path) => Ok(path),
        None => ensure().await,
    }
}

fn existing_bun() -> Option<PathBuf> {
    let managed = bun_bin();
    if managed.is_file() {
        return Some(managed);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("bun"))
            .find(|path| path.is_file())
    })
}

async fn fetch_pinned(dest: &Path) -> Result<PathBuf> {
    let (_, _, slug, sha) = BUN_ASSETS
        .iter()
        .find(|(os, arch, _, _)| *os == std::env::consts::OS && *arch == std::env::consts::ARCH)
        .with_context(|| {
            format!(
                "no bun build for {}/{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?;
    let url =
        format!("https://github.com/oven-sh/bun/releases/download/bun-v{BUN_VERSION}/{slug}.zip");
    fetch_into(&[url], sha, slug, dest).await?;
    Ok(dest.to_path_buf())
}

/// Download the first of `urls` whose body matches `sha256`, take
/// `<slug>/bun` out of it, run it once, and rename it to `dest`.
async fn fetch_into(urls: &[String], sha256: &str, slug: &str, dest: &Path) -> Result<()> {
    let dir = dest.parent().context("bun's folder")?;
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    sweep_staging(dir);
    let zip = download_verified(urls, sha256).await?;
    let staging = Staging::new(dir)?;
    let staged = staging.path.join("bun");
    unpack(&zip, &format!("{slug}/bun"), &staged)?;
    probe(&staged).await?;
    std::fs::rename(&staged, dest).with_context(|| format!("move bun to {}", dest.display()))?;
    Ok(())
}

async fn download_verified(urls: &[String], sha256: &str) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .user_agent("linggen")
        .connect_timeout(crate::mirror::CONNECT_TIMEOUT)
        .read_timeout(Duration::from_secs(60))
        .build()?;
    let mut last = None;
    for url in urls {
        match download(&client, url).await {
            Ok(body) => {
                let got = format!("{:x}", sha2::Sha256::digest(&body));
                if got.eq_ignore_ascii_case(sha256) {
                    return Ok(body);
                }
                tracing::warn!("[js-runtime] sha256 mismatch from {url}: {got}");
                last = Some(anyhow::anyhow!("sha256 mismatch from {url}"));
            }
            Err(e) => {
                tracing::warn!("[js-runtime] download failed from {url}: {e:#}");
                last = Some(e);
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no source for bun")))
}

async fn download(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let resp = client.get(url).send().await?.error_for_status()?;
    Ok(resp.bytes().await?.to_vec())
}

fn unpack(zip: &[u8], entry: &str, to: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).context("open bun zip")?;
    let mut file = archive
        .by_name(entry)
        .with_context(|| format!("{entry} not in the bun zip"))?;
    let mut out = std::fs::File::create(to)?;
    std::io::copy(&mut file, &mut out)?;
    out.sync_all()?;
    set_executable(to)
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

/// The unpacked bun must answer `--version` before it takes the path.
async fn probe(bin: &Path) -> Result<()> {
    let run = tokio::process::Command::new(bin)
        .arg("--version")
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(PROBE_TIMEOUT, run)
        .await
        .context("bun --version did not answer in time")??;
    let said = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() || !said.trim().starts_with(|c: char| c.is_ascii_digit()) {
        bail!("the unpacked bun does not run ({})", out.status);
    }
    Ok(())
}

const STAGING_PREFIX: &str = ".bun-staging-";

/// A staging folder beside bun, removed however the fetch ends.
struct Staging {
    path: PathBuf,
}

impl Staging {
    fn new(dir: &Path) -> Result<Self> {
        let path = dir.join(format!("{STAGING_PREFIX}{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).with_context(|| format!("create {}", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Staging folders an interrupted fetch (a killed engine) left behind.
fn sweep_staging(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(STAGING_PREFIX)
        {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// A zip holding `<slug>/bun` as a shell script that prints `version`.
    fn bun_zip(slug: &str, version: &str) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            zip.start_file(
                format!("{slug}/bun"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(format!("#!/bin/sh\necho {version}\n").as_bytes())
                .unwrap();
            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    fn sha(body: &[u8]) -> String {
        format!("{:x}", sha2::Sha256::digest(body))
    }

    /// Serves `body` at every path on 127.0.0.1.
    fn serve(body: Vec<u8>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut req = [0u8; 4096];
                let _ = s.read(&mut req);
                let head = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            }
        });
        format!("http://{addr}/bun.zip")
    }

    /// Nothing but `bun` itself may be left in the folder.
    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n != "bun")
                    .collect()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn a_verified_zip_lands_as_a_runnable_bun() {
        let zip = bun_zip("bun-test", "9.9.9");
        let url = serve(zip.clone());
        let home = tempfile::tempdir().unwrap();
        let dest = home.path().join("bin/bun");
        fetch_into(&[url], &sha(&zip), "bun-test", &dest)
            .await
            .unwrap();
        let out = std::process::Command::new(&dest)
            .arg("--version")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "9.9.9");
        assert!(leftovers(&dest.parent().unwrap()).is_empty());
    }

    #[tokio::test]
    async fn a_wrong_checksum_leaves_nothing() {
        let zip = bun_zip("bun-test", "9.9.9");
        let url = serve(zip);
        let home = tempfile::tempdir().unwrap();
        let dest = home.path().join("bin/bun");
        let err = fetch_into(&[url], &"0".repeat(64), "bun-test", &dest)
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("sha256 mismatch"), "{err:#}");
        assert!(!dest.exists());
        assert!(leftovers(&dest.parent().unwrap()).is_empty());
    }

    #[tokio::test]
    async fn no_network_leaves_nothing() {
        // A port nothing listens on.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let home = tempfile::tempdir().unwrap();
        let dest = home.path().join("bin/bun");
        let url = format!("http://127.0.0.1:{port}/bun.zip");
        assert!(fetch_into(&[url], &"0".repeat(64), "bun-test", &dest)
            .await
            .is_err());
        assert!(!dest.exists());
        assert!(leftovers(&dest.parent().unwrap()).is_empty());
    }

    #[tokio::test]
    async fn a_bun_that_does_not_run_is_not_put_in_place() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            zip.start_file("bun-test/bun", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"#!/bin/sh\nexit 3\n").unwrap();
            zip.finish().unwrap();
        }
        let zip = buf.into_inner();
        let url = serve(zip.clone());
        let home = tempfile::tempdir().unwrap();
        let dest = home.path().join("bin/bun");
        assert!(fetch_into(&[url], &sha(&zip), "bun-test", &dest)
            .await
            .is_err());
        assert!(!dest.exists());
        assert!(leftovers(&dest.parent().unwrap()).is_empty());
    }

    #[test]
    fn an_interrupted_fetch_is_swept() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(format!("{STAGING_PREFIX}old"))).unwrap();
        std::fs::write(dir.path().join("other"), "kept").unwrap();
        sweep_staging(dir.path());
        assert_eq!(leftovers(dir.path()), vec!["other".to_string()]);
    }

    #[tokio::test]
    async fn a_dropped_fetch_does_not_stay_running() {
        let fetch = async {
            let _running = Running::start();
            std::future::pending::<()>().await;
        };
        let _ = tokio::time::timeout(Duration::from_millis(10), fetch).await;
        assert!(!RUNNING.load(Ordering::SeqCst));
    }

    #[test]
    fn every_pin_is_a_sha256() {
        for (_, _, slug, sha) in BUN_ASSETS {
            assert_eq!(sha.len(), 64, "{slug}");
            assert!(sha.chars().all(|c| c.is_ascii_hexdigit()), "{slug}");
        }
    }
}
