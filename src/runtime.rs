//! Managed Python runtime — a pinned, relocatable CPython plus named
//! virtualenvs under `~/.linggen/runtime/`, so skills and engine sidecars
//! never depend on whatever `python3` a machine happens to have (a stock
//! Mac without Xcode CLT has none at all).
//!
//! Everything here is fetched by [`prewarm`], a background task spawned on
//! engine start, so nothing user-facing ever waits on a download. Each
//! component is stamped on disk and re-verified on every boot; a stamp
//! mismatch (version bump, changed package pins) rebuilds just that
//! component. Downloads resume from a `.part` file and are SHA-256
//! verified against the pins below — the mirror is never trusted.
//!
//! The interpreter comes from python-build-standalone, re-hosted in
//! `linggen/linggen-releases` so the linggen.dev `/dl` mirror covers it.

use anyhow::{bail, Context, Result};
use sha2::Digest;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

/// Interpreter build: python-build-standalone `install_only` tarballs,
/// re-hosted under this tag in linggen/linggen-releases.
const PY_RELEASE_TAG: &str = "runtime-cpython-3.12.14";
const PY_BUILD: &str = "3.12.14+20260814";

/// (arch, asset file name, sha256) — the asset is chosen by the arch the
/// engine was built for, which is the arch its children will run as.
const PY_ASSETS: [(&str, &str, &str); 2] = [
    (
        "aarch64",
        "cpython-3.12.14+20260814-aarch64-apple-darwin-install_only.tar.gz",
        "4572133a5542f306b9bdb155da5800f9e38950cd0a98d469b832ce256fe299ea",
    ),
    (
        "x86_64",
        "cpython-3.12.14+20260814-x86_64-apple-darwin-install_only.tar.gz",
        "1a94c83264731e9603fbea78e57e7ca8f20e7d91eb866627ac2304621b0f6f1f",
    ),
];

/// Named venvs and their pinned package specs. A spec change rebuilds the
/// venv on the next boot (the stamp no longer matches). `tools` carries
/// yt-dlp as a thin script — the PyInstaller `yt-dlp_macos` binary pays a
/// ~10s unpack-and-validate tax on every exec, this pays it never.
/// `[default]` keeps mutagen so `--embed-thumbnail` still works;
/// `curl-cffi` gives browser impersonation. Installed `--pre`: YouTube
/// blocks rot faster than yt-dlp's stable cadence (2026-08-19: stable
/// 2026.7.4 403'd on every video, the nightly on PyPI downloaded fine).
const VENVS: [(&str, &[&str]); 2] = [
    ("tools", &["--pre", "yt-dlp[default,curl-cffi]"]),
    ("tts", &["mlx-audio==0.5.0"]),
];

/// The TTS voice model the `tts` venv serves; warmed into the shared HF
/// cache so first playback never downloads.
const TTS_MODEL: &str = "mlx-community/Qwen3-TTS-12Hz-0.6B-CustomVoice-4bit";

pub fn runtime_dir() -> PathBuf {
    crate::paths::linggen_home().join("runtime")
}

pub fn python_dir() -> PathBuf {
    runtime_dir().join("py")
}

pub fn python_bin() -> PathBuf {
    python_dir().join("bin/python3")
}

pub fn env_dir(name: &str) -> PathBuf {
    runtime_dir().join("envs").join(name)
}

pub fn env_bin(name: &str, tool: &str) -> PathBuf {
    env_dir(name).join("bin").join(tool)
}

/// Bin dirs to append to the shell PATH — appended, never prepended, per
/// the [`crate::util::shell_path`] rule: they only resolve commands that
/// would otherwise fail (no system python3 at all).
pub fn path_dirs() -> Vec<PathBuf> {
    vec![env_dir("tools").join("bin"), python_dir().join("bin")]
}

pub fn python_ready() -> bool {
    stamp_matches(&python_dir().join(".linggen-stamp"), PY_BUILD)
}

fn env_ready(name: &str, spec: &[&str]) -> bool {
    stamp_matches(&env_dir(name).join(".linggen-stamp"), &spec.join(" "))
}

fn stamp_matches(path: &Path, want: &str) -> bool {
    std::fs::read_to_string(path).is_ok_and(|s| s.trim() == want)
}

fn write_stamp(dir: &Path, value: &str) -> Result<()> {
    std::fs::write(dir.join(".linggen-stamp"), value).context("write stamp")
}

// ---------------------------------------------------------------------------
// Prewarm — the boot task
// ---------------------------------------------------------------------------

/// Fetch and repair every runtime component in the background. Spawned on
/// engine start; safe to run on every boot (completed stages are stamp
/// checks, nothing else). Progress rides the retained `tasks` topic so the
/// task widget on any surface can show it; telemetry must never break the
/// prewarm, so publish failures are swallowed inside [`publish_progress`].
pub async fn prewarm() {
    // One prewarm at a time; a second boot-time spawn or an on-demand
    // ensure_* call waits instead of racing the unpack.
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = LOCK.lock().await;

    if !cfg!(target_os = "macos") {
        tracing::info!("[runtime] prewarm skipped: unsupported platform");
        return;
    }

    for attempt in 0..3u32 {
        match prewarm_stages().await {
            Ok(()) => return,
            Err(e) => {
                tracing::warn!("[runtime] prewarm attempt {} failed: {e:#}", attempt + 1);
                tokio::time::sleep(std::time::Duration::from_secs(600 << attempt)).await;
            }
        }
    }
    tracing::warn!("[runtime] prewarm giving up until next boot");
}

async fn prewarm_stages() -> Result<()> {
    let stages: &[(&str, bool)] = &[
        ("python", true),
        ("tools", true),
        // MLX is Apple-Silicon-only; Intel Macs keep the runtime + tools
        // and simply never grow a TTS lane.
        ("tts", cfg!(target_arch = "aarch64")),
        ("tts-model", cfg!(target_arch = "aarch64")),
    ];
    let total = stages.iter().filter(|(_, on)| *on).count();
    let mut done = 0;

    for (stage, enabled) in stages {
        if !enabled {
            continue;
        }
        publish_progress(done, total, stage, false);
        match *stage {
            "python" => ensure_python().await.map(|_| ())?,
            "tools" => ensure_venv("tools").await.map(|_| ())?,
            "tts" => ensure_venv("tts").await.map(|_| ())?,
            "tts-model" => ensure_tts_model().await?,
            _ => unreachable!(),
        }
        done += 1;
        publish_progress(done, total, stage, done == total);
    }
    tracing::info!("[runtime] prewarm complete");
    Ok(())
}

fn publish_progress(done: usize, total: usize, current: &str, finished: bool) {
    crate::server::api::topic::retain(
        "tasks",
        "runtime",
        &serde_json::json!({
            "app": "runtime",
            "task_id": "runtime-prewarm",
            "label": "Preparing runtime",
            "done": done, "total": total, "current": current,
            "finished": finished,
            "at": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        }),
    );
}

// ---------------------------------------------------------------------------
// Stages
// ---------------------------------------------------------------------------

/// Ensure the pinned interpreter is installed; returns its `python3` path.
pub async fn ensure_python() -> Result<PathBuf> {
    if python_ready() {
        return Ok(python_bin());
    }
    let (_, asset, sha) = PY_ASSETS
        .iter()
        .find(|(arch, _, _)| *arch == std::env::consts::ARCH)
        .with_context(|| format!("no python build for {}", std::env::consts::ARCH))?;

    check_disk_headroom(&runtime_dir(), 1)?;
    let tarball = runtime_dir().join(format!("{asset}.download"));
    download_verified(asset, sha, &tarball).await?;

    // Unpack beside the target then rename, so a crash never leaves a
    // half-written `py/` that stamps as broken-but-present.
    let unpack = runtime_dir().join("py.unpack");
    let _ = std::fs::remove_dir_all(&unpack);
    std::fs::create_dir_all(&unpack)?;
    run_ok(
        tokio::process::Command::new("tar")
            .arg("-xzf")
            .arg(&tarball)
            .arg("-C")
            .arg(&unpack),
        "unpack python",
    )
    .await?;
    let _ = std::fs::remove_dir_all(python_dir());
    // The tarball root is `python/`.
    std::fs::rename(unpack.join("python"), python_dir()).context("move python into place")?;
    let _ = std::fs::remove_dir_all(&unpack);
    let _ = std::fs::remove_file(&tarball);

    let out = run_ok(
        tokio::process::Command::new(python_bin()).arg("--version"),
        "verify python",
    )
    .await?;
    tracing::info!("[runtime] installed {}", out.trim());
    write_stamp(&python_dir(), PY_BUILD)?;
    Ok(python_bin())
}

/// Ensure a named venv exists with its pinned packages; returns its dir.
pub async fn ensure_venv(name: &str) -> Result<PathBuf> {
    let (_, spec) = VENVS
        .iter()
        .find(|(n, _)| *n == name)
        .with_context(|| format!("unknown venv {name}"))?;
    if env_ready(name, spec) {
        // yt-dlp rots as YouTube changes; a quick upgrade check each boot
        // keeps it alive the same way the old binary's self-update did.
        if name == "tools" {
            let _ = pip(
                name,
                &[
                    "install",
                    "-q",
                    "--upgrade",
                    "--pre",
                    "yt-dlp[default,curl-cffi]",
                ],
            )
            .await;
        }
        return Ok(env_dir(name));
    }
    let python = ensure_python().await?;
    check_disk_headroom(&runtime_dir(), 2)?;

    let dir = env_dir(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.parent().context("envs parent")?)?;
    run_ok(
        tokio::process::Command::new(&python)
            .arg("-m")
            .arg("venv")
            .arg(&dir),
        "create venv",
    )
    .await?;
    let mut args = vec!["install", "-q"];
    args.extend(spec.iter().copied());
    pip(name, &args).await?;
    write_stamp(&dir, &spec.join(" "))?;
    tracing::info!("[runtime] venv {name} ready");
    Ok(dir)
}

async fn pip(env: &str, args: &[&str]) -> Result<String> {
    run_ok(
        tokio::process::Command::new(env_bin(env, "pip")).args(args),
        "pip",
    )
    .await
}

/// Warm the TTS model into the shared HF cache (the same one Kokoro uses),
/// so the first spoken line never downloads. huggingface_hub resumes
/// partial downloads on its own; `HF_ENDPOINT` is inherited so the China
/// mirror keeps working.
async fn ensure_tts_model() -> Result<()> {
    check_disk_headroom(&runtime_dir(), 6)?;
    let code = format!("from mlx_audio.tts.utils import load_model; load_model({TTS_MODEL:?})");
    run_ok(
        tokio::process::Command::new(env_bin("tts", "python3"))
            .arg("-c")
            .arg(&code)
            .env(
                "HF_HOME",
                crate::paths::linggen_home().join("models/hf-hub"),
            ),
        "warm tts model",
    )
    .await
    .map(|_| ())
}

// ---------------------------------------------------------------------------
// Download + helpers
// ---------------------------------------------------------------------------

/// Download a release asset with `.part` resume and SHA-256 verification,
/// trying GitHub first and the linggen.dev `/dl` mirror second.
async fn download_verified(asset: &str, sha256: &str, dest: &Path) -> Result<()> {
    if dest.exists() && file_sha256(dest)? == sha256 {
        return Ok(());
    }
    std::fs::create_dir_all(dest.parent().context("dest parent")?)?;
    let urls = [
        format!(
            "https://github.com/linggen/linggen-releases/releases/download/{PY_RELEASE_TAG}/{asset}"
        ),
        format!("https://linggen.dev/dl/release/linggen-releases/{PY_RELEASE_TAG}/{asset}"),
    ];
    let part = dest.with_extension("part");
    let mut last = None;
    for url in &urls {
        match download_resumable(url, &part).await {
            Ok(()) => {
                let got = file_sha256(&part)?;
                if got != sha256 {
                    let _ = std::fs::remove_file(&part);
                    bail!("sha256 mismatch for {asset}: got {got}");
                }
                std::fs::rename(&part, dest)?;
                return Ok(());
            }
            Err(e) => {
                tracing::warn!("[runtime] download failed from {url}: {e:#}");
                last = Some(e);
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no download source for {asset}")))
}

async fn download_resumable(url: &str, part: &Path) -> Result<()> {
    let have = part.metadata().map(|m| m.len()).unwrap_or(0);
    let client = reqwest::Client::new();
    let mut req = client.get(url);
    if have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let resp = req.send().await?.error_for_status()?;
    // A server that ignores the Range header restarts the body from zero.
    let append = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT && have > 0;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(append)
        .write(true)
        .truncate(!append)
        .open(part)
        .await?;
    let mut stream = resp.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        file.write_all(&chunk?).await?;
    }
    file.flush().await?;
    Ok(())
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut hasher = sha2::Sha256::new();
    let mut f = std::fs::File::open(path)?;
    std::io::copy(&mut f, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Refuse to start a stage without `need_gb` of free disk — a full disk
/// mid-unpack is worse than a missing runtime.
fn check_disk_headroom(dir: &Path, need_gb: u64) -> Result<()> {
    std::fs::create_dir_all(dir).ok();
    let out = std::process::Command::new("df")
        .arg("-Pk")
        .arg(dir)
        .output()
        .context("df")?;
    let text = String::from_utf8_lossy(&out.stdout);
    let avail_kb: u64 = text
        .lines()
        .nth(1)
        .and_then(|l| l.split_whitespace().nth(3))
        .and_then(|v| v.parse().ok())
        .unwrap_or(u64::MAX); // unparseable df must not block the prewarm
    if avail_kb / 1024 / 1024 < need_gb {
        bail!("low disk: {}MB free, need {need_gb}GB", avail_kb / 1024);
    }
    Ok(())
}

async fn run_ok(cmd: &mut tokio::process::Command, what: &str) -> Result<String> {
    let out = cmd.output().await.with_context(|| what.to_string())?;
    if !out.status.success() {
        bail!(
            "{what} failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
