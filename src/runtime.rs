//! Managed Python runtime — a pinned, relocatable CPython plus named
//! virtualenvs under `~/.linggen/runtime/`, so skills and engine sidecars
//! never depend on whatever `python3` a machine happens to have (a stock
//! Mac without Xcode CLT has none at all).
//!
//! Everything here is fetched by [`prewarm`], a background task spawned on
//! engine start, so nothing user-facing ever waits on a download. The big
//! stages (each lane's venv and model) start only when the machine is
//! quiet — no agent run in flight and no one chatting for a few minutes —
//! and one download runs at a time ([`download_slot`]). Each
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
const VENVS: [(&str, &[&str]); 3] = [
    ("tools", &["--pre", "yt-dlp[default,curl-cffi]"]),
    ("tts", &["mlx-audio==0.5.0"]),
    ("pictures", &["mflux==0.18.0"]),
];

/// The TTS voice model the `tts` venv serves; warmed into the shared HF
/// cache so first playback never downloads, then stamped.
pub const TTS_MODEL: &str = "mlx-community/Qwen3-TTS-12Hz-0.6B-CustomVoice-4bit";

/// The picture model the `pictures` venv runs: FLUX.2 klein 4B, 4-bit,
/// in mflux's own weight layout (only that layout loads pre-quantized;
/// other MLX conversions fail in the text encoder). Chosen 2026-09-15
/// over Z-Image Turbo: half the time and memory at 512², and the look
/// suits the game. The upstream 4B weights are Apache-2.0; this
/// community quantization's card mislabels its license, so before a
/// public release the quantization is redone from the original.
pub const PICTURE_MODEL: &str = "ar9av/FLUX.2-klein-4B-mflux-4bit";

// ---------------------------------------------------------------------------
// Lanes — which local models this machine may carry
// ---------------------------------------------------------------------------

/// A local-model lane and the machine it needs. A lane is an engine
/// capability (voice, pictures), never an app. Both run on MLX, so both
/// are Apple-Silicon-only. The thresholds are measurements from 2026-09-15:
/// the voice sidecar peaks at 2.5 GB resident, a 512² picture at 6.9 GB;
/// beside the engine and a browser an 8 GB Mac carries neither, a 16 GB
/// Mac carries both. Disk is the download plus unpack headroom.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Lane {
    pub name: &'static str,
    pub min_memory_gb: u64,
    pub disk_gb: u64,
}

pub const VOICE_LANE: Lane = Lane {
    name: "voice",
    min_memory_gb: 16,
    disk_gb: 6,
};
pub const PICTURES_LANE: Lane = Lane {
    name: "pictures",
    min_memory_gb: 16,
    disk_gb: 8,
};
pub const LANES: [Lane; 2] = [VOICE_LANE, PICTURES_LANE];

/// What the gate measures. Read once per check; nothing here is cached
/// because free disk changes under us.
struct Machine {
    macos: bool,
    apple_silicon: bool,
    memory_gb: u64,
    disk_gb: u64,
}

impl Machine {
    fn this() -> Self {
        Machine {
            macos: cfg!(target_os = "macos"),
            apple_silicon: cfg!(all(target_os = "macos", target_arch = "aarch64")),
            memory_gb: unified_memory_gb(),
            disk_gb: disk_free_gb(&runtime_dir()),
        }
    }
}

/// May this machine carry `lane`? `Err` carries the reason in plain
/// words, shown to the user as-is.
pub fn lane_gate(lane: &Lane) -> Result<(), String> {
    gate(lane, &Machine::this())
}

fn gate(lane: &Lane, m: &Machine) -> Result<(), String> {
    if !m.macos {
        return Err("needs a Mac".into());
    }
    if !m.apple_silicon {
        return Err("needs an Apple Silicon Mac".into());
    }
    if m.memory_gb < lane.min_memory_gb {
        return Err(format!(
            "needs {} GB of memory, this Mac has {} GB",
            lane.min_memory_gb, m.memory_gb
        ));
    }
    if m.disk_gb < lane.disk_gb {
        return Err(format!(
            "needs {} GB free on disk, {} GB free now",
            lane.disk_gb, m.disk_gb
        ));
    }
    Ok(())
}

/// Whether this machine can ever carry `lane` — platform and memory only,
/// which do not change while the engine runs. Cached; used on the hot
/// path that decides which tools the model is offered. Free disk is the
/// install stage's concern, not this one's.
pub fn lane_supported(lane: &Lane) -> bool {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<&'static str, bool>>,
    > = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(v) = cache.lock().ok().and_then(|m| m.get(lane.name).copied()) {
        return v;
    }
    let m = Machine {
        macos: cfg!(target_os = "macos"),
        apple_silicon: cfg!(all(target_os = "macos", target_arch = "aarch64")),
        memory_gb: unified_memory_gb(),
        disk_gb: u64::MAX,
    };
    let ok = gate(lane, &m).is_ok();
    if let Ok(mut map) = cache.lock() {
        map.insert(lane.name, ok);
    }
    ok
}

/// Every lane with its verdict — what Settings and the progress topic show.
pub fn lanes_status() -> serde_json::Value {
    let m = Machine::this();
    let lanes: Vec<serde_json::Value> = LANES
        .iter()
        .map(|lane| {
            let verdict = gate(lane, &m);
            serde_json::json!({
                "name": lane.name,
                "min_memory_gb": lane.min_memory_gb,
                "disk_gb": lane.disk_gb,
                "available": verdict.is_ok(),
                "reason": verdict.err(),
            })
        })
        .collect();
    serde_json::json!({
        "memory_gb": m.memory_gb,
        "disk_free_gb": m.disk_gb,
        "lanes": lanes,
    })
}

/// Installed unified memory in whole GB; 0 when it cannot be read, which
/// fails the gate closed rather than open.
fn unified_memory_gb() -> u64 {
    if !cfg!(target_os = "macos") {
        return 0;
    }
    std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse::<u64>()
                .ok()
        })
        .map(|b| b / GB)
        .unwrap_or(0)
}

const GB: u64 = 1024 * 1024 * 1024;

/// Free disk in whole GB for the volume holding `dir`. On macOS this is
/// the purgeable-aware figure Finder shows (APFS frees purgeable space on
/// demand); `df` excludes it and under-reports by tens of GB on a Mac with
/// big caches. Unreadable free space must not block a stage, so the
/// fallback is "plenty".
fn disk_free_gb(dir: &Path) -> u64 {
    std::fs::create_dir_all(dir).ok();
    foundation_free_bytes(dir)
        .or_else(|| df_free_bytes(dir))
        .map(|b| b / GB)
        .unwrap_or(u64::MAX)
}

fn foundation_free_bytes(dir: &Path) -> Option<u64> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let path = serde_json::to_string(&dir.to_string_lossy()).ok()?;
    let script = format!(
        "ObjC.import('Foundation'); \
         const u = $.NSURL.fileURLWithPath({path}); \
         const k = $.NSURLVolumeAvailableCapacityForImportantUsageKey; \
         const v = u.resourceValuesForKeysError($([k]), null); \
         v.objectForKey(k).longLongValue"
    );
    let out = std::process::Command::new("osascript")
        .args(["-l", "JavaScript", "-e", &script])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn df_free_bytes(dir: &Path) -> Option<u64> {
    let out = std::process::Command::new("df")
        .arg("-Pk")
        .arg(dir)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let avail_kb: u64 = text
        .lines()
        .nth(1)?
        .split_whitespace()
        .nth(3)?
        .parse()
        .ok()?;
    Some(avail_kb * 1024)
}

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
pub type Progress = std::sync::Arc<dyn Fn(&serde_json::Value) + Send + Sync>;

/// The progress sink the server wires at boot, kept so an on-demand
/// install started from a tool call reports on the same topic.
static PROGRESS_SINK: std::sync::OnceLock<Progress> = std::sync::OnceLock::new();

pub fn set_progress_sink(progress: Progress) {
    let _ = PROGRESS_SINK.set(progress);
}

fn progress_sink() -> Progress {
    PROGRESS_SINK
        .get()
        .cloned()
        .unwrap_or_else(|| std::sync::Arc::new(|_| {}))
}

/// Is the machine quiet enough to start a download? The server answers
/// from its own facts (no top-level run, no chat turn for a few minutes);
/// an on-demand install passes [`always_quiet`].
pub type Quiet = std::sync::Arc<dyn Fn() -> bool + Send + Sync>;

/// How often a waiting stage asks [`Quiet`] again.
const QUIET_POLL: std::time::Duration = std::time::Duration::from_secs(30);

pub fn always_quiet() -> Quiet {
    std::sync::Arc::new(|| true)
}

/// Return once `quiet` says so, asking every `poll`. A download never
/// starts mid-run; one already started is not paused.
pub async fn wait_for_quiet(quiet: &Quiet, poll: std::time::Duration) {
    while !quiet() {
        tokio::time::sleep(poll).await;
    }
}

/// One large download at a time, engine-wide: every stage here and every
/// other managed download (Kokoro's weights) holds this while it runs.
/// Held per stage, not per prewarm, so an on-demand install can run
/// between boot stages that are waiting for quiet.
pub async fn download_slot() -> tokio::sync::MutexGuard<'static, ()> {
    static SLOT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    SLOT.lock().await
}

/// Boot runs every stage the lane gate admits — voice (venv + model,
/// ~2.1 GB) and pictures (venv + ~4.3 GB model) — so neither waits for
/// its first use. Python and the tools venv start at once (small, and
/// skills call them on a fresh install's first turn); the lane stages
/// wait for `quiet`. `progress` receives each stage payload; the server
/// wires it to the live topic bus + retained store.
pub async fn prewarm(progress: Progress, quiet: Quiet) {
    prewarm_lanes(true, true, progress, quiet).await
}

async fn prewarm_lanes(with_tts: bool, with_pictures: bool, progress: Progress, quiet: Quiet) {
    if !cfg!(target_os = "macos") {
        tracing::info!("[runtime] prewarm skipped: unsupported platform");
        return;
    }
    export_python_env();

    for attempt in 0..3u32 {
        match prewarm_stages(with_tts, with_pictures, &progress, &quiet).await {
            Ok(()) => return,
            Err(e) if format!("{e:#}").contains("low disk") => {
                // Not retryable on a timer — the disk won't free itself.
                // Next boot re-checks; no 10/20/40-minute wedge.
                tracing::warn!("[runtime] prewarm stopped: {e:#}");
                return;
            }
            Err(e) => {
                tracing::warn!("[runtime] prewarm attempt {} failed: {e:#}", attempt + 1);
                tokio::time::sleep(std::time::Duration::from_secs(600 << attempt)).await;
            }
        }
    }
    tracing::warn!("[runtime] prewarm giving up until next boot");
}

/// Publish the managed interpreter to every child lane at once: skill
/// scripts on any spawn path read `${LINGGEN_PY:-python3}`. Only set when
/// the interpreter is actually there — a dangling value would defeat the
/// `:-python3` fallback on machines that still need it.
fn export_python_env() {
    if python_ready() {
        std::env::set_var("LINGGEN_PY", python_bin());
    }
}

/// Are the picture venv and model both on disk? Cheap; the tool asks on
/// every call.
pub fn pictures_ready() -> bool {
    env_bin("pictures", "python3").exists() && stamp_matches(&picture_model_stamp(), PICTURE_MODEL)
}

/// Are the voice venv and model both on disk? The sidecar spawns only then.
pub fn voice_ready() -> bool {
    env_bin("tts", "python3").exists() && stamp_matches(&tts_model_stamp(), TTS_MODEL)
}

fn picture_model_stamp() -> PathBuf {
    runtime_dir().join("pictures-model.stamp")
}

fn tts_model_stamp() -> PathBuf {
    runtime_dir().join("tts-model.stamp")
}

/// Run the picture stages now, off the caller's path. Boot already queues
/// them for the first quiet moment; this is for a GenerateImage call that
/// beats it (or a boot prewarm that gave up). The person asked, so it does
/// not wait for quiet — only for the download slot.
pub fn ensure_pictures_in_background() {
    tokio::spawn(prewarm_lanes(false, true, progress_sink(), always_quiet()));
}

/// Which lanes a stage set may run: each goes through the gate and a
/// closed lane is logged with its reason.
fn lane_open(lane: &Lane, wanted: bool) -> bool {
    if !wanted {
        return false;
    }
    match lane_gate(lane) {
        Ok(()) => true,
        Err(reason) => {
            tracing::info!("[runtime] {} lane off: {reason}", lane.name);
            false
        }
    }
}

/// A lane stage is a large download and waits for quiet; python and the
/// tools venv are small and needed on a fresh install's first turn.
fn waits_for_quiet(stage: &str) -> bool {
    !matches!(stage, "python" | "tools")
}

/// Is this stage already done on disk? A done stage skips the quiet wait.
fn stage_ready(stage: &str) -> bool {
    match stage {
        "python" => python_ready(),
        "tts-model" => stamp_matches(&tts_model_stamp(), TTS_MODEL),
        "pictures-model" => stamp_matches(&picture_model_stamp(), PICTURE_MODEL),
        venv => VENVS
            .iter()
            .any(|(name, spec)| *name == venv && env_ready(name, spec)),
    }
}

/// A model stage whose snapshot is already in the shared cache has nothing
/// to download — it only verifies and stamps — so it need not wait for
/// quiet. This is what an install from before the stamps looks like.
fn model_cached(stage: &str) -> bool {
    let repo = match stage {
        "tts-model" => TTS_MODEL,
        "pictures-model" => PICTURE_MODEL,
        _ => return false,
    };
    hub_snapshot_present(&hf_home().join("hub"), repo)
}

fn hub_snapshot_present(hub: &Path, repo: &str) -> bool {
    let snapshots = hub
        .join(format!("models--{}", repo.replace('/', "--")))
        .join("snapshots");
    std::fs::read_dir(snapshots).is_ok_and(|mut d| d.next().is_some())
}

async fn prewarm_stages(
    with_tts: bool,
    with_pictures: bool,
    progress: &Progress,
    quiet: &Quiet,
) -> Result<()> {
    // Each lane goes through the lane gate (Apple Silicon, memory, disk);
    // a Mac that fails it keeps the runtime + tools and simply never grows
    // that lane. The reason is logged and rides on the progress payload so
    // Settings can say why.
    let tts_on = lane_open(&VOICE_LANE, with_tts);
    let pictures_on = lane_open(&PICTURES_LANE, with_pictures);
    let stages: &[(&str, bool)] = &[
        ("python", true),
        ("tools", true),
        ("tts", tts_on),
        ("tts-model", tts_on),
        ("pictures", pictures_on),
        ("pictures-model", pictures_on),
    ];
    let total = stages.iter().filter(|(_, on)| *on).count();
    let mut done = 0;

    for (stage, enabled) in stages {
        if !enabled {
            continue;
        }
        if waits_for_quiet(stage) && !stage_ready(stage) && !model_cached(stage) {
            publish_progress(progress, done, total, stage, Phase::WaitingForQuiet);
            wait_for_quiet(quiet, QUIET_POLL).await;
        }
        publish_progress(progress, done, total, stage, Phase::Running);
        run_stage(stage).await?;
        done += 1;
        let phase = if done == total {
            Phase::Finished
        } else {
            Phase::Running
        };
        publish_progress(progress, done, total, stage, phase);
    }
    tracing::info!("[runtime] prewarm complete");
    Ok(())
}

/// Run one stage inside the download slot. Every `ensure_*` re-checks its
/// own stamp first, so a stage another prewarm finished meanwhile is free.
async fn run_stage(stage: &str) -> Result<()> {
    let _slot = download_slot().await;
    match stage {
        "python" => {
            ensure_python().await?;
            export_python_env();
        }
        "tools" | "tts" | "pictures" => {
            ensure_venv(stage).await?;
        }
        "tts-model" => ensure_tts_model().await?,
        "pictures-model" => ensure_picture_model().await?,
        _ => unreachable!(),
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    WaitingForQuiet,
    Running,
    Finished,
}

fn publish_progress(progress: &Progress, done: usize, total: usize, current: &str, phase: Phase) {
    progress(&serde_json::json!({
            "app": "runtime",
            "task_id": "runtime-prewarm",
            "label": "Preparing runtime",
            "done": done, "total": total, "current": current,
            "finished": phase == Phase::Finished,
            // The next stage is queued for a quiet moment, not stuck.
            "waiting_for_quiet": phase == Phase::WaitingForQuiet,
            "machine": lanes_status(),
            "at": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
    }));
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

/// PyPI mirrors a failed install retries through, in order — for regions
/// where pypi.org is blocked or crawling (mainland China).
const PYPI_MIRRORS: &[&str] = &[
    "https://pypi.tuna.tsinghua.edu.cn/simple",
    "https://mirrors.aliyun.com/pypi/simple/",
];

/// The pip invocations to try: `args` unchanged first, then once per
/// mirror with `-i <mirror>` — only for installs, and never when the
/// caller already named an index.
fn pip_attempts(args: &[&str]) -> Vec<Vec<String>> {
    let first: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let install = args.first() == Some(&"install");
    let names_index = args
        .iter()
        .any(|a| *a == "-i" || a.starts_with("--index-url") || a.starts_with("--extra-index-url"));
    let mut out = vec![first.clone()];
    if install && !names_index {
        for m in PYPI_MIRRORS {
            let mut a = first.clone();
            a.insert(1, "-i".into());
            a.insert(2, m.to_string());
            out.push(a);
        }
    }
    out
}

async fn pip(env: &str, args: &[&str]) -> Result<String> {
    let mut last = None;
    for (i, attempt) in pip_attempts(args).iter().enumerate() {
        if i > 0 {
            tracing::warn!("[runtime] pip retrying through {}", attempt[2]);
        }
        match run_ok(
            tokio::process::Command::new(env_bin(env, "pip")).args(attempt),
            "pip",
        )
        .await
        {
            Ok(out) => return Ok(out),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("pip: nothing to run")))
}

/// Warm the TTS model into the shared HF cache (the same one Kokoro uses),
/// so the first spoken line never downloads, and stamp it so a later boot
/// skips both the load and the quiet wait. huggingface_hub resumes
/// partial downloads on its own; `HF_ENDPOINT` is the user's, else
/// hf-mirror.com when huggingface.co is unreachable (see `hf_mirror`).
async fn ensure_tts_model() -> Result<()> {
    if stamp_matches(&tts_model_stamp(), TTS_MODEL) {
        return Ok(());
    }
    check_disk_headroom(&runtime_dir(), 6)?;
    let code = format!("from mlx_audio.tts.utils import load_model; load_model({TTS_MODEL:?})");
    run_ok(
        tokio::process::Command::new(env_bin("tts", "python3"))
            .arg("-c")
            .arg(&code)
            .env("HF_HOME", hf_home())
            .env("HF_ENDPOINT", crate::hf_mirror::endpoint().await),
        "warm tts model",
    )
    .await?;
    std::fs::write(tts_model_stamp(), TTS_MODEL)?;
    tracing::info!("[runtime] tts model ready");
    Ok(())
}

/// Warm the picture model into the same HF cache, then stamp it so
/// [`pictures_ready`] is a file test. snapshot_download resumes partial
/// pulls on its own; `HF_ENDPOINT` as in [`ensure_tts_model`].
async fn ensure_picture_model() -> Result<()> {
    if stamp_matches(&picture_model_stamp(), PICTURE_MODEL) {
        return Ok(());
    }
    check_disk_headroom(&runtime_dir(), PICTURES_LANE.disk_gb)?;
    let code = format!(
        "from huggingface_hub import snapshot_download; snapshot_download({PICTURE_MODEL:?})"
    );
    run_ok(
        tokio::process::Command::new(env_bin("pictures", "python3"))
            .arg("-c")
            .arg(&code)
            .env("HF_HOME", hf_home())
            .env("HF_ENDPOINT", crate::hf_mirror::endpoint().await),
        "warm picture model",
    )
    .await?;
    std::fs::write(picture_model_stamp(), PICTURE_MODEL)?;
    tracing::info!("[runtime] picture model ready");
    Ok(())
}

/// The one Hugging Face cache every managed model shares.
pub fn hf_home() -> PathBuf {
    crate::paths::linggen_home().join("models/hf-hub")
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
    let urls = crate::mirror::sources(&format!(
        "https://github.com/linggen/linggen-releases/releases/download/{PY_RELEASE_TAG}/{asset}"
    ));
    let part = dest.with_extension("part");
    let mut last = None;
    for url in &urls {
        match download_resumable(url, &part).await {
            Ok(()) => {
                let got = file_sha256(&part)?;
                if got != sha256 {
                    // A corrupt body from one source is exactly what the
                    // other source is for — clear the .part and move on.
                    let _ = std::fs::remove_file(&part);
                    tracing::warn!("[runtime] sha256 mismatch for {asset} from {url}");
                    last = Some(anyhow::anyhow!("sha256 mismatch for {asset}: got {got}"));
                    continue;
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
    let client = reqwest::Client::builder()
        .connect_timeout(crate::mirror::CONNECT_TIMEOUT)
        .build()?;
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

pub(crate) fn file_sha256(path: &Path) -> Result<String> {
    let mut hasher = sha2::Sha256::new();
    let mut f = std::fs::File::open(path)?;
    std::io::copy(&mut f, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Refuse to start a stage without `need_gb` of free disk — a full disk
/// mid-unpack is worse than a missing runtime.
pub(crate) fn check_disk_headroom(dir: &Path, need_gb: u64) -> Result<()> {
    let free = disk_free_gb(dir);
    if free < need_gb {
        bail!("low disk: {free}GB free, need {need_gb}GB");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn mac(memory_gb: u64, disk_gb: u64) -> Machine {
        Machine {
            macos: true,
            apple_silicon: true,
            memory_gb,
            disk_gb,
        }
    }

    #[test]
    fn lane_opens_on_a_capable_mac() {
        assert_eq!(gate(&VOICE_LANE, &mac(16, 50)), Ok(()));
        assert_eq!(gate(&PICTURES_LANE, &mac(16, 8)), Ok(()));
    }

    #[test]
    fn lane_closes_off_mac_and_off_apple_silicon() {
        let linux = Machine {
            macos: false,
            apple_silicon: false,
            memory_gb: 64,
            disk_gb: 500,
        };
        assert_eq!(gate(&PICTURES_LANE, &linux), Err("needs a Mac".into()));
        let intel = Machine {
            macos: true,
            apple_silicon: false,
            memory_gb: 64,
            disk_gb: 500,
        };
        assert_eq!(
            gate(&VOICE_LANE, &intel),
            Err("needs an Apple Silicon Mac".into())
        );
    }

    #[test]
    fn lane_closes_on_small_memory_then_disk() {
        assert_eq!(
            gate(&VOICE_LANE, &mac(8, 100)),
            Err("needs 16 GB of memory, this Mac has 8 GB".into())
        );
        assert_eq!(
            gate(&PICTURES_LANE, &mac(32, 7)),
            Err("needs 8 GB free on disk, 7 GB free now".into())
        );
    }

    #[test]
    fn unreadable_memory_fails_closed() {
        assert!(gate(&VOICE_LANE, &mac(0, 100)).is_err());
    }

    #[test]
    fn pip_install_retries_through_the_mirrors_after_the_plain_try() {
        let got = pip_attempts(&["install", "-q", "--upgrade", "--pre", "yt-dlp"]);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], ["install", "-q", "--upgrade", "--pre", "yt-dlp"]);
        assert_eq!(
            got[1],
            [
                "install",
                "-i",
                "https://pypi.tuna.tsinghua.edu.cn/simple",
                "-q",
                "--upgrade",
                "--pre",
                "yt-dlp"
            ]
        );
        assert_eq!(got[2][2], "https://mirrors.aliyun.com/pypi/simple/");
    }

    #[test]
    fn pip_keeps_a_named_index_and_non_installs_alone() {
        assert_eq!(
            pip_attempts(&["install", "-i", "https://x/simple", "a"]).len(),
            1
        );
        assert_eq!(pip_attempts(&["list"]).len(), 1);
    }

    /// A predicate that says "busy" `busy` times, then "quiet", counting asks.
    fn quiet_after(busy: usize) -> (Quiet, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let asks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = asks.clone();
        let quiet: Quiet = std::sync::Arc::new(move || {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= busy
        });
        (quiet, asks)
    }

    #[tokio::test]
    async fn a_download_waits_until_the_machine_is_quiet() {
        let (quiet, asks) = quiet_after(3);
        wait_for_quiet(&quiet, std::time::Duration::from_millis(1)).await;
        // Three busy answers, then the quiet one that let it through.
        assert_eq!(asks.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn a_quiet_machine_starts_at_once() {
        let (quiet, asks) = quiet_after(0);
        let started = std::time::Instant::now();
        wait_for_quiet(&quiet, std::time::Duration::from_secs(60)).await;
        assert_eq!(asks.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert!(always_quiet()());
    }

    #[tokio::test]
    async fn a_busy_machine_never_starts() {
        let (quiet, _) = quiet_after(usize::MAX);
        let wait = wait_for_quiet(&quiet, std::time::Duration::from_millis(1));
        let r = tokio::time::timeout(std::time::Duration::from_millis(50), wait).await;
        assert!(r.is_err(), "a download started while the machine was busy");
    }

    #[test]
    fn only_the_lane_stages_wait_for_quiet() {
        for stage in ["tts", "tts-model", "pictures", "pictures-model"] {
            assert!(waits_for_quiet(stage), "{stage} should wait");
        }
        for stage in ["python", "tools"] {
            assert!(!waits_for_quiet(stage), "{stage} should not wait");
        }
    }

    #[test]
    fn a_cached_snapshot_is_recognised_and_an_empty_one_is_not() {
        let hub = tempfile::tempdir().unwrap();
        let repo = "org/model-4bit";
        assert!(!hub_snapshot_present(hub.path(), repo));
        let snaps = hub.path().join("models--org--model-4bit/snapshots");
        std::fs::create_dir_all(&snaps).unwrap();
        assert!(!hub_snapshot_present(hub.path(), repo));
        std::fs::create_dir_all(snaps.join("abc123")).unwrap();
        assert!(hub_snapshot_present(hub.path(), repo));
        assert!(!model_cached("tts"), "a venv stage is never a cached model");
    }

    #[tokio::test]
    async fn the_download_slot_admits_one_at_a_time() {
        let first = download_slot().await;
        let second = tokio::time::timeout(std::time::Duration::from_millis(20), download_slot());
        assert!(second.await.is_err(), "two downloads held the slot at once");
        drop(first);
        let _again = download_slot().await;
    }

    #[test]
    fn lanes_status_names_every_lane() {
        let v = lanes_status();
        let names: Vec<&str> = v["lanes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["voice", "pictures"]);
    }
}
