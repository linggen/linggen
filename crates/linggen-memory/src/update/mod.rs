//! Self-update for the `ling-mem` binary.
//!
//! Three callable surfaces:
//!
//! | Function | Used by |
//! |:---------|:--------|
//! | [`check`] / [`check_quiet`] | `ling-mem upgrade --check`, and (cached) `ling-mem start` / `ling-mem restart` |
//! | [`read_cached`]             | `ling-mem status` (no network) |
//! | [`apply`]                   | `ling-mem upgrade --yes` |
//!
//! Release source: `linggen/linggen-memory` GitHub releases. Tag pattern
//! `vX.Y.Z`. Asset layout per release (see `scripts/release.sh`):
//!
//! ```text
//! ling-mem-<slug>.tar.gz          (+ .sha256 sibling)
//! ```
//!
//! Slug values: `macos-aarch64`, `macos-x86_64`, `linux-x86_64`,
//! `linux-aarch64`. The tarball contains a flat `ling-mem` binary plus
//! `README.md` and `LICENSE`.
//!
//! Update flow:
//! 1. Resolve current platform → asset name.
//! 2. Hit `releases/latest`; pick the matching `.tar.gz` + `.sha256`.
//! 3. Download tarball + checksum to a temp dir under the binary's parent.
//! 4. Verify SHA256 (streaming, in-process — no `sha2` crate dep).
//! 5. Extract via `tar -xzf` (already a release-pipeline dep).
//! 6. Stop the daemon if running, atomic-rename the new binary into place
//!    (keeping the prior at `<bin-dir>/ling-mem.prev`), then restart by
//!    explicitly invoking the new binary path so the running (old) process
//!    doesn't relaunch its own inode.
//!
//! `LINGGEN_RELEASE_BASE=<url>` replaces GitHub (the release gate's copy of
//! a draft): the release comes from `<url>/linggen/linggen-memory/release.json`
//! (`{"tag_name", "assets": [{"name"}]}`), assets from the same directory,
//! never cached. The SHA256 check is unchanged.
//!
//! A swap whose new binary fails to answer `--version` or to start the
//! daemon puts `ling-mem.prev` back; [`rollback`] swaps the two by hand.
//!
//! Network errors during `--check` are swallowed when used from `start`
//! (`check_quiet`) — `start` should never fail just because GitHub is down.

use crate::hash::Sha256;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const REPO: &str = "linggen/linggen-memory";
const RELEASES_LATEST_URL: &str =
    "https://api.github.com/repos/linggen/linggen-memory/releases/latest";
const USER_AGENT: &str = concat!("ling-mem/", env!("CARGO_PKG_VERSION"));
const CACHE_TTL_SECS: u64 = 24 * 60 * 60;
const NETWORK_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a swapped-in binary has to answer `--version`.
const START_TIMEOUT: Duration = Duration::from_secs(10);
const RELEASE_BASE_ENV: &str = "LINGGEN_RELEASE_BASE";

/// The release mirror that replaces GitHub, when `LINGGEN_RELEASE_BASE` is set.
fn release_base() -> Option<String> {
    base_from(std::env::var(RELEASE_BASE_ENV).ok().as_deref())
}

fn base_from(value: Option<&str>) -> Option<String> {
    let base = value?.trim().trim_end_matches('/');
    (!base.is_empty()).then(|| base.to_string())
}

fn latest_url(base: Option<&str>) -> String {
    match base {
        Some(b) => format!("{b}/{REPO}/release.json"),
        None => RELEASES_LATEST_URL.to_string(),
    }
}

fn asset_url(base: Option<&str>, tag: &str, asset: &str) -> String {
    match base {
        Some(b) => format!("{b}/{REPO}/{asset}"),
        None => format!("https://github.com/{REPO}/releases/download/{tag}/{asset}"),
    }
}

/// Result of an update probe — what `--check` prints, and what `start`
/// embeds in its lifecycle JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub current: String,
    pub latest: Option<String>,
    pub url: Option<String>,
    pub notes_summary: Option<String>,
    /// Set when the platform isn't releasable (e.g. unsupported arch) or
    /// when the latest release lacks an asset for our platform. The CLI
    /// surfaces this so the user knows why the update path is unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsupported: Option<String>,
}

impl UpdateInfo {
    fn current_only() -> Self {
        Self {
            available: false,
            current: env!("CARGO_PKG_VERSION").to_string(),
            latest: None,
            url: None,
            notes_summary: None,
            unsupported: None,
        }
    }

    fn unsupported(reason: impl Into<String>) -> Self {
        let mut info = Self::current_only();
        info.unsupported = Some(reason.into());
        info
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    fetched_at: u64,
    info: UpdateInfo,
}

fn cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join(".ling-mem-update-cache.json")
}

fn read_cache(data_dir: &Path) -> Option<UpdateInfo> {
    let raw = fs::read(cache_path(data_dir)).ok()?;
    let entry: CacheEntry = serde_json::from_slice(&raw).ok()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if now.saturating_sub(entry.fetched_at) > CACHE_TTL_SECS {
        return None;
    }
    // Cache is keyed on the binary's own version. If the running binary
    // is newer than what was cached (just upgraded), discard.
    if entry.info.current != env!("CARGO_PKG_VERSION") {
        return None;
    }
    Some(entry.info)
}

fn write_cache(data_dir: &Path, info: &UpdateInfo) {
    let entry = CacheEntry {
        fetched_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        info: info.clone(),
    };
    if let Some(parent) = cache_path(data_dir).parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(
        cache_path(data_dir),
        serde_json::to_vec(&entry).unwrap_or_default(),
    );
}

/// Slug used in release asset names — must match
/// `scripts/lib-common.sh::detect_platform`.
fn platform_slug() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("macos-aarch64"),
        ("macos", "x86_64") => Some("macos-x86_64"),
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("linux", "aarch64") => Some("linux-aarch64"),
        _ => None,
    }
}

fn asset_name(slug: &str) -> String {
    format!("ling-mem-{slug}.tar.gz")
}

/// Cached version probe. Falls through to a network call on cache miss.
/// `bypass_cache=true` always hits the network.
pub async fn check(data_dir: &Path, bypass_cache: bool) -> Result<UpdateInfo> {
    let Some(slug) = platform_slug() else {
        return Ok(UpdateInfo::unsupported(format!(
            "no release asset for {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )));
    };

    let base = release_base();
    if !bypass_cache && base.is_none() {
        if let Some(cached) = read_cache(data_dir) {
            return Ok(cached);
        }
    }

    let info = fetch_latest(slug, base.as_deref())
        .await
        .context("update check failed")?;
    if base.is_none() {
        write_cache(data_dir, &info);
    }
    Ok(info)
}

/// Best-effort version probe used during `start`. Network failures are
/// swallowed — `start` should never fail just because GitHub is down.
pub async fn check_quiet(data_dir: &Path) -> UpdateInfo {
    match check(data_dir, false).await {
        Ok(info) => info,
        Err(_) => UpdateInfo::current_only(),
    }
}

/// Cache-only probe — never hits the network. Used by `status`, which is
/// called frequently and must stay fast. Returns `None` if the cache is
/// missing, expired, or stale relative to the current binary version.
/// Callers should treat `None` as "no recent check available" rather than
/// "no update available."
pub fn read_cached(data_dir: &Path) -> Option<UpdateInfo> {
    read_cache(data_dir)
}

/// Returns the cache's `fetched_at` (unix seconds) when present and fresh,
/// so callers can surface a `checked_at` timestamp alongside a cached
/// `UpdateInfo`. None when the cache is missing or expired.
pub fn cache_fetched_at(data_dir: &Path) -> Option<u64> {
    let raw = fs::read(cache_path(data_dir)).ok()?;
    let entry: CacheEntry = serde_json::from_slice(&raw).ok()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if now.saturating_sub(entry.fetched_at) > CACHE_TTL_SECS {
        return None;
    }
    if entry.info.current != env!("CARGO_PKG_VERSION") {
        return None;
    }
    Some(entry.fetched_at)
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    html_url: Option<String>,
    body: Option<String>,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
}

async fn fetch_latest(slug: &str, base: Option<&str>) -> Result<UpdateInfo> {
    let client = reqwest::Client::builder()
        .timeout(NETWORK_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()?;

    let mut req = client.get(latest_url(base));
    // The token is GitHub's; a mirror never sees it.
    if let (None, Ok(token)) = (base, std::env::var("GITHUB_TOKEN")) {
        if !token.is_empty() {
            req = req.bearer_auth(token);
        }
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("GitHub returned {}", resp.status()));
    }
    let release: Release = resp.json().await?;

    let latest_ver = release.tag_name.trim_start_matches('v').to_string();
    let current = env!("CARGO_PKG_VERSION");

    let asset = asset_name(slug);
    let asset_match = release.assets.iter().any(|a| a.name == asset);

    let available = asset_match && version_lt(current, &latest_ver);
    let notes_summary = release
        .body
        .as_deref()
        .map(headline)
        .filter(|s| !s.is_empty());

    Ok(UpdateInfo {
        available,
        current: current.to_string(),
        latest: Some(latest_ver),
        url: release.html_url,
        notes_summary,
        unsupported: if asset_match {
            None
        } else {
            Some(format!("no `{asset}` in latest release"))
        },
    })
}

/// First non-empty line of release notes, trimmed and bounded.
fn headline(body: &str) -> String {
    body.lines()
        .map(|l| l.trim_start_matches(|c: char| c == '#' || c.is_whitespace()))
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| {
            if l.chars().count() > 200 {
                let head: String = l.chars().take(199).collect();
                format!("{head}…")
            } else {
                l.to_string()
            }
        })
        .unwrap_or_default()
}

/// Naive semver compare. Strips any pre-release / build suffix at the
/// first non-`.`/non-digit char (so `0.3.0-rc.1` collapses to `0.3.0`),
/// then splits on `.` and compares numerically. The release pipeline only
/// ever tags `vX.Y.Z`, so this is sufficient — we just need to not crash
/// on the rare hand-edited tag.
fn version_lt(current: &str, latest: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        let core: String = v
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        core.split('.')
            .map(|s| s.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let a = parse(current);
    let b = parse(latest);
    let len = a.len().max(b.len());
    for i in 0..len {
        let ai = a.get(i).copied().unwrap_or(0);
        let bi = b.get(i).copied().unwrap_or(0);
        if ai != bi {
            return ai < bi;
        }
    }
    false
}

/// Leading major component of a version string (same lenient parse as
/// [`version_lt`]): `"1.2.3"` → 1, `"2.0.0-rc.1"` → 2.
fn major_of(v: &str) -> u64 {
    v.chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

/// Outcome of an `apply` call — rendered as JSON to stdout.
#[derive(Debug, Serialize)]
pub struct UpdateOutcome {
    pub updated: bool,
    pub from: String,
    pub to: String,
    pub restarted: bool,
    /// Set when no swap happened (e.g. already-current).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

pub struct ApplyOptions<'a> {
    pub data_dir: &'a Path,
    pub skill_dir: &'a Path,
    pub port: u16,
    pub force: bool,
}

/// Run a real update: download, verify, swap, optionally restart.
pub async fn apply(opts: ApplyOptions<'_>) -> Result<UpdateOutcome> {
    let info = check(opts.data_dir, true).await?;
    let current = env!("CARGO_PKG_VERSION").to_string();

    if let Some(reason) = &info.unsupported {
        return Err(anyhow!("cannot update: {reason}"));
    }

    let Some(latest) = info.latest.clone() else {
        return Err(anyhow!("update check returned no latest version"));
    };

    if !info.available && !opts.force {
        return Ok(UpdateOutcome {
            updated: false,
            from: current.clone(),
            to: current,
            restarted: false,
            note: Some("already on latest version".to_string()),
        });
    }

    // A major bump can carry a non-migratable store-schema change (that is
    // the release policy — see doc/schema-versioning.md). The new binary's
    // open-time guard protects the data either way, but a blind swap would
    // strand the user on a binary that refuses their store. Make the jump
    // explicit.
    if major_of(&latest) != major_of(&current) && !opts.force {
        return Err(anyhow!(
            "refusing to cross a major version ({current} → {latest}): the store \
             schema may be incompatible. Run `ling-mem export memory.jsonl` first, \
             then re-run with --force; if the new binary refuses the store, reset \
             it and `ling-mem import memory.jsonl`."
        ));
    }

    let slug = platform_slug().ok_or_else(|| anyhow!("unsupported platform"))?;
    let exe = std::env::current_exe().context("resolving current executable path")?;
    let bin_dir = exe
        .parent()
        .ok_or_else(|| anyhow!("binary has no parent directory"))?
        .to_path_buf();

    refuse_managed_path(&bin_dir)?;

    let tag = format!("v{latest}");
    let asset = asset_name(slug);
    let base = release_base();
    let download_url = asset_url(base.as_deref(), &tag, &asset);
    let sha_url = asset_url(base.as_deref(), &tag, &format!("{asset}.sha256"));

    let staging = StagingDir::create_under(&bin_dir)?;
    let tarball_path = staging.path().join(&asset);
    let sha_path = staging.path().join(format!("{asset}.sha256"));

    download_to(&download_url, &tarball_path)
        .await
        .with_context(|| format!("downloading {download_url}"))?;
    download_to(&sha_url, &sha_path)
        .await
        .with_context(|| format!("downloading {sha_url}"))?;

    verify_sha256(&tarball_path, &sha_path)?;

    let extracted = extract_binary(&tarball_path, staging.path())?;

    let was_running = stop_daemon_if_running(opts.skill_dir).await?;

    let new_canonical = bin_dir.join("ling-mem");
    swap_binary(&extracted, &new_canonical, &bin_dir)?;

    // Spawn the *new* binary explicitly. Using `current_exe()` here is
    // unsafe: on Linux `/proc/self/exe` follows the inode, which is now at
    // `ling-mem.prev`, so we'd relaunch the old version.
    if let Err(e) = start_swapped(&new_canonical, was_running, opts.data_dir, opts.port) {
        restore_previous(&new_canonical, &bin_dir)?;
        if was_running {
            let _ = spawn_new_daemon(&new_canonical, opts.data_dir, opts.port);
        }
        return Err(e.context("the new binary failed to start; the previous one is restored"));
    }
    let restarted = was_running;

    // Best-effort cache invalidation so future `--check` calls reflect reality.
    let _ = fs::remove_file(cache_path(opts.data_dir));

    Ok(UpdateOutcome {
        updated: true,
        from: current,
        to: latest,
        restarted,
        note: None,
    })
}

fn refuse_managed_path(bin_dir: &Path) -> Result<()> {
    let s = bin_dir.to_string_lossy();
    let managed = [
        ("/usr/local/bin", "homebrew or system"),
        ("/usr/local/Cellar", "homebrew"),
        ("/opt/homebrew", "homebrew"),
        ("/usr/bin", "system"),
        ("/opt/local", "macports"),
    ];
    for (prefix, kind) in managed {
        if s.starts_with(prefix) {
            return Err(anyhow!(
                "binary lives under {prefix} ({kind}-managed) — use that package manager to update"
            ));
        }
    }
    Ok(())
}

async fn download_to(url: &str, dest: &Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {} for {url}", resp.status()));
    }
    let bytes = resp.bytes().await?;
    fs::write(dest, &bytes).with_context(|| format!("writing {}", dest.display()))?;
    Ok(())
}

fn verify_sha256(file: &Path, sha_file: &Path) -> Result<()> {
    use std::io::Read;
    let raw = fs::read_to_string(sha_file).context("reading sha256 file")?;
    // `shasum -a 256` format: "<hex>  <filename>"
    let expected = raw
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("empty sha256 file"))?
        .to_lowercase();
    if expected.len() != 64 {
        return Err(anyhow!("malformed sha256: {expected:?}"));
    }

    let mut hasher = Sha256::new();
    let mut f = fs::File::open(file).context("opening tarball for hashing")?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = hasher.hex();
    if actual != expected {
        return Err(anyhow!(
            "sha256 mismatch: expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

/// Extract the `ling-mem` binary from the tarball into `into/ling-mem.new`,
/// chmod 755, and sanity-check `--version`. Uses `tar` from the host (the
/// release pipeline already requires it).
fn extract_binary(tarball: &Path, into: &Path) -> Result<PathBuf> {
    let status = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(tarball)
        .arg("-C")
        .arg(into)
        .status()
        .context("spawning `tar -xzf`")?;
    if !status.success() {
        return Err(anyhow!("`tar -xzf` exited with {status}"));
    }

    let extracted_at_top = into.join("ling-mem");
    if !extracted_at_top.is_file() {
        return Err(anyhow!(
            "expected `ling-mem` at top level of tarball; not found in {}",
            into.display()
        ));
    }

    // Move to a `.new` name so the extracted-from-tar inode is what we
    // ultimately rename into place.
    let new_path = into.join("ling-mem.new");
    fs::rename(&extracted_at_top, &new_path)
        .with_context(|| format!("renaming staged binary to {}", new_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(&new_path)?.permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&new_path, perm)?;
    }

    probe_version(&new_path, START_TIMEOUT).context("extracted binary")?;
    Ok(new_path)
}

/// Run `<bin> --version`; Ok(version) when it exits 0 within `timeout` and
/// prints `ling-mem <version>`.
fn probe_version(bin: &Path, timeout: Duration) -> Result<String> {
    let mut child = std::process::Command::new(bin)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("running {} --version", bin.display()))?;
    let deadline = std::time::Instant::now() + timeout;
    while child.try_wait()?.is_none() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!("--version gave no answer within {timeout:?}"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(anyhow!("--version exited with {}", out.status));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .trim()
        .strip_prefix("ling-mem ")
        .map(|v| v.trim().to_string())
        .ok_or_else(|| anyhow!("--version unexpected output: {stdout:?}"))
}

/// The swapped-in binary must answer `--version` at its real path and,
/// when the daemon was running, start it again.
fn start_swapped(bin: &Path, was_running: bool, data_dir: &Path, port: u16) -> Result<()> {
    probe_version(bin, START_TIMEOUT)?;
    if was_running {
        spawn_new_daemon(bin, data_dir, port)?;
    }
    Ok(())
}

/// Put `ling-mem.prev` back at the canonical path, dropping the new binary.
fn restore_previous(canonical: &Path, bin_dir: &Path) -> Result<()> {
    let prev = bin_dir.join("ling-mem.prev");
    if !prev.is_file() {
        return Err(anyhow!(
            "no previous binary at {} to restore",
            prev.display()
        ));
    }
    let _ = fs::remove_file(canonical);
    fs::rename(&prev, canonical)
        .with_context(|| format!("restoring {} → {}", prev.display(), canonical.display()))
}

/// Swap `ling-mem` and `ling-mem.prev` in `bin_dir`, keeping the swap only
/// if the restored binary answers `--version`. Returns (from, to).
fn swap_with_prev(bin_dir: &Path, timeout: Duration) -> Result<(String, String)> {
    let canonical = bin_dir.join("ling-mem");
    let prev = bin_dir.join("ling-mem.prev");
    if !prev.is_file() {
        return Err(anyhow!(
            "no previous binary kept at {} — nothing to roll back to",
            prev.display()
        ));
    }
    let from = probe_version(&canonical, timeout).unwrap_or_else(|_| "?".into());
    let to = probe_version(&prev, timeout).context("the previous binary does not start")?;
    let aside = bin_dir.join(format!(".ling-mem.rollback-{}", std::process::id()));
    fs::rename(&canonical, &aside).context("moving the current binary aside")?;
    if let Err(e) = fs::rename(&prev, &canonical) {
        let _ = fs::rename(&aside, &canonical);
        return Err(anyhow!("restoring the previous binary: {e}"));
    }
    fs::rename(&aside, &prev).context("keeping the replaced binary as ling-mem.prev")?;
    Ok((from, to))
}

/// `ling-mem upgrade --rollback`: swap back to `ling-mem.prev` (a second
/// rollback returns), restarting the daemon on the restored binary.
pub async fn rollback(opts: ApplyOptions<'_>) -> Result<UpdateOutcome> {
    let exe = std::env::current_exe().context("resolving current executable path")?;
    let bin_dir = exe
        .parent()
        .ok_or_else(|| anyhow!("binary has no parent directory"))?
        .to_path_buf();
    refuse_managed_path(&bin_dir)?;
    if !bin_dir.join("ling-mem.prev").is_file() {
        return Err(anyhow!(
            "no previous binary kept in {} — nothing to roll back to",
            bin_dir.display()
        ));
    }

    let was_running = stop_daemon_if_running(opts.skill_dir).await?;
    let (from, to) = swap_with_prev(&bin_dir, START_TIMEOUT)?;
    let canonical = bin_dir.join("ling-mem");
    if was_running {
        if let Err(e) = spawn_new_daemon(&canonical, opts.data_dir, opts.port) {
            swap_with_prev(&bin_dir, START_TIMEOUT)?;
            let _ = spawn_new_daemon(&canonical, opts.data_dir, opts.port);
            return Err(
                e.context("the previous binary failed to start the daemon; kept the current one")
            );
        }
    }
    let _ = fs::remove_file(cache_path(opts.data_dir));
    Ok(UpdateOutcome {
        updated: true,
        from,
        to,
        restarted: was_running,
        note: Some("rolled back; run `ling-mem upgrade --rollback` again to return".into()),
    })
}

async fn stop_daemon_if_running(skill_dir: &Path) -> Result<bool> {
    use crate::daemon::lifecycle::{stop, LifecycleOutcome};
    match stop(skill_dir).await? {
        LifecycleOutcome::Stopped { .. } => Ok(true),
        LifecycleOutcome::NotRunning => Ok(false),
        // `stop` only ever returns Stopped or NotRunning.
        _ => Ok(false),
    }
}

/// Atomic-rename: move the running binary aside (rollback copy), then put
/// the new binary at its canonical path. On failure mid-way, restore.
fn swap_binary(new_bin: &Path, current_canonical: &Path, bin_dir: &Path) -> Result<()> {
    let prev = bin_dir.join("ling-mem.prev");
    if prev.exists() {
        let _ = fs::remove_file(&prev);
    }
    if current_canonical.exists() {
        fs::rename(current_canonical, &prev).with_context(|| {
            format!(
                "moving {} → {} (rollback copy)",
                current_canonical.display(),
                prev.display()
            )
        })?;
    }
    if let Err(e) = fs::rename(new_bin, current_canonical) {
        // Best-effort restore.
        let _ = fs::rename(&prev, current_canonical);
        return Err(anyhow!(
            "installing new binary at {}: {e}",
            current_canonical.display()
        ));
    }
    Ok(())
}

/// Spawn the new binary's `start` subcommand directly, so we don't relaunch
/// our own (now-renamed) inode. Wait briefly for it to print its lifecycle
/// JSON and exit; if anything goes wrong, surface the stderr.
fn spawn_new_daemon(new_bin: &Path, data_dir: &Path, port: u16) -> Result<()> {
    let out = std::process::Command::new(new_bin)
        .arg("start")
        .arg("--port")
        .arg(port.to_string())
        .env("LINGGEN_DATA_DIR", data_dir)
        .output()
        .with_context(|| format!("spawning {} start", new_bin.display()))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!(
            "new binary failed to start daemon (exit {}): {}",
            out.status,
            stderr.trim()
        ));
    }
    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────────────────────

struct StagingDir {
    path: PathBuf,
}

impl StagingDir {
    fn create_under(base: &Path) -> Result<Self> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = base.join(format!(".ling-mem-update-{nanos}"));
        fs::create_dir_all(&dir).with_context(|| format!("creating staging {}", dir.display()))?;
        Ok(Self { path: dir })
    }
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for StagingDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_lt_basic() {
        assert!(version_lt("0.2.0", "0.2.1"));
        assert!(version_lt("0.2.1", "0.3.0"));
        assert!(version_lt("0.9.0", "1.0.0"));
        assert!(!version_lt("0.2.1", "0.2.1"));
        assert!(!version_lt("0.3.0", "0.2.9"));
    }

    #[test]
    fn version_lt_strips_suffix() {
        // Pre-release / build suffixes collapse to the numeric prefix, so a
        // bare X.Y.Z is considered equal to X.Y.Z-rc.N for our comparison.
        // Acceptable because release.sh only ever tags plain vX.Y.Z.
        assert!(!version_lt("0.3.0", "0.3.0-rc.1"));
        assert!(version_lt("0.2.9", "0.3.0-rc.1"));
    }

    #[test]
    fn major_of_parses_leading_component() {
        assert_eq!(major_of("1.2.3"), 1);
        assert_eq!(major_of("2.0.0-rc.1"), 2);
        assert_eq!(major_of("0.7.2"), 0);
        assert_eq!(major_of("garbage"), 0);
    }

    #[test]
    fn headline_picks_first_nonempty_line() {
        assert_eq!(headline(""), "");
        assert_eq!(headline("\n\n  ## Highlights\nbody\n"), "Highlights");
        assert_eq!(headline("First line.\nSecond."), "First line.");
    }

    #[test]
    fn the_override_replaces_github_and_never_carries_the_tag() {
        assert_eq!(base_from(None), None);
        assert_eq!(base_from(Some(" ")), None);
        assert_eq!(latest_url(None), RELEASES_LATEST_URL);
        assert_eq!(
            asset_url(None, "v1.9.0", "ling-mem-macos-aarch64.tar.gz"),
            "https://github.com/linggen/linggen-memory/releases/download/v1.9.0/ling-mem-macos-aarch64.tar.gz"
        );
        let base = base_from(Some("http://10.0.0.2:8765/"));
        assert_eq!(base.as_deref(), Some("http://10.0.0.2:8765"));
        assert_eq!(
            latest_url(base.as_deref()),
            "http://10.0.0.2:8765/linggen/linggen-memory/release.json"
        );
        assert_eq!(
            asset_url(
                base.as_deref(),
                "v1.9.0",
                "ling-mem-macos-aarch64.tar.gz.sha256"
            ),
            "http://10.0.0.2:8765/linggen/linggen-memory/ling-mem-macos-aarch64.tar.gz.sha256"
        );
        let r: Release =
            serde_json::from_str(r#"{"tag_name":"v1.9.0","assets":[{"name":"x"}]}"#).unwrap();
        assert!(r.html_url.is_none());
    }

    /// A fake `ling-mem` whose `--version` runs `body`.
    fn script(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    const T: Duration = Duration::from_secs(10);

    #[test]
    fn a_wrong_sha256_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let tar = d.path().join("t.tar.gz");
        let sha = d.path().join("t.tar.gz.sha256");
        fs::write(&tar, b"abc").unwrap();
        fs::write(&sha, format!("{}  t.tar.gz\n", "0".repeat(64))).unwrap();
        assert!(verify_sha256(&tar, &sha).is_err());
        fs::write(
            &sha,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  t.tar.gz\n",
        )
        .unwrap();
        assert!(verify_sha256(&tar, &sha).is_ok());
    }

    #[test]
    fn a_binary_that_fails_or_hangs_does_not_pass_the_probe() {
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("ling-mem");
        script(&bin, "echo 'ling-mem 1.2.3'");
        assert_eq!(probe_version(&bin, T).unwrap(), "1.2.3");
        script(&bin, "exit 1");
        assert!(probe_version(&bin, T).is_err());
        script(&bin, "sleep 30");
        assert!(probe_version(&bin, Duration::from_millis(300)).is_err());
    }

    #[test]
    fn a_failed_start_restores_the_previous_binary() {
        let d = tempfile::tempdir().unwrap();
        let canonical = d.path().join("ling-mem");
        script(&canonical, "echo 'ling-mem 1.0.0'");
        let new = d.path().join("ling-mem.new");
        script(&new, "exit 1");
        swap_binary(&new, &canonical, d.path()).unwrap();
        assert!(start_swapped(&canonical, false, d.path(), 0).is_err());
        restore_previous(&canonical, d.path()).unwrap();
        assert_eq!(probe_version(&canonical, T).unwrap(), "1.0.0");
    }

    #[test]
    fn rollback_swaps_back_and_forth() {
        let d = tempfile::tempdir().unwrap();
        assert!(swap_with_prev(d.path(), T).is_err());
        script(&d.path().join("ling-mem"), "echo 'ling-mem 2.0.0'");
        script(&d.path().join("ling-mem.prev"), "echo 'ling-mem 1.0.0'");
        assert_eq!(
            swap_with_prev(d.path(), T).unwrap(),
            ("2.0.0".into(), "1.0.0".into())
        );
        assert_eq!(
            probe_version(&d.path().join("ling-mem.prev"), T).unwrap(),
            "2.0.0"
        );
        swap_with_prev(d.path(), T).unwrap();
        assert_eq!(
            probe_version(&d.path().join("ling-mem"), T).unwrap(),
            "2.0.0"
        );
    }

    #[test]
    fn a_broken_previous_binary_is_not_rolled_back_to() {
        let d = tempfile::tempdir().unwrap();
        script(&d.path().join("ling-mem"), "echo 'ling-mem 2.0.0'");
        script(&d.path().join("ling-mem.prev"), "exit 1");
        assert!(swap_with_prev(d.path(), T).is_err());
        assert_eq!(
            probe_version(&d.path().join("ling-mem"), T).unwrap(),
            "2.0.0"
        );
    }

    #[test]
    fn sha256_known_vectors() {
        let mut h = Sha256::new();
        h.update(b"abc");
        assert_eq!(
            h.hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let h = Sha256::new();
        assert_eq!(
            h.hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
