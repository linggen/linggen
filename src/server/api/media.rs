//! Wireless media sync — Linggen Mobile's paired Photos backup.
//!
//! Contract: `linggen-mobile/doc/media-sync-protocol.md`. Three routes:
//! manifest (what does the Mac need / already hold), ingest (one original per
//! multipart POST), verify (which uploads are now safe to delete on-phone).
//!
//! Files land in the mac-shifu Media pipeline's own staging + archive, so the
//! Mac review UI and the phone share one source of truth:
//! - staging rows append to `data/media/manifest.jsonl` with a `wireless/…`
//!   path (the USB pull's ghost-reconcile skips non-`/` paths);
//! - ingest archives immediately to `~/Pictures/iPhone Backup` with a re-hash,
//!   appending `data/media/archive.jsonl` — the same ledger the pipeline's
//!   `remove` leg trusts. `verified` == "this sha is in that ledger", so the
//!   phone's delete gate is exactly the USB flow's delete gate.

use axum::{
    extract::Multipart,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

/// Serializes ledger/manifest mutations across concurrent ingests.
static MEDIA_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Bumped on every ingest; a scheduled scan only fires if it is still the
/// newest generation after the quiesce window (i.e. uploads went quiet).
static SCAN_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// At most one pipeline scan at a time.
static SCAN_RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// How long uploads must go quiet before the post-sync scan fires.
const SCAN_QUIESCE: std::time::Duration = std::time::Duration::from_secs(20);

/// Manifest rows written by this module use this path prefix instead of an
/// AFC phone path; it both marks them for verify lookups and exempts them
/// from the USB pull's ghost-reconcile (which only prunes `/…` paths).
const WIRELESS_PREFIX: &str = "wireless/";

fn data_dir() -> PathBuf {
    crate::paths::global_skills_dir().join("mac-shifu").join("data").join("media")
}

fn staging_dir() -> PathBuf {
    data_dir().join("staging")
}

fn manifest_path() -> PathBuf {
    data_dir().join("manifest.jsonl")
}

fn ledger_path() -> PathBuf {
    data_dir().join("archive.jsonl")
}

fn flags_path() -> PathBuf {
    data_dir().join("flags.json")
}

/// Mac Shifu's scan verdicts (blurry/dark/…), keyed by content hash. The
/// phone borrows these instead of re-implementing image analysis in Dart —
/// standalone gets the cheap detectors, paired gets the Mac's brains.
fn load_verdicts() -> HashMap<String, Vec<String>> {
    let Ok(text) = std::fs::read_to_string(flags_path()) else {
        return HashMap::new();
    };
    let Ok(doc) = serde_json::from_str::<Value>(&text) else {
        return HashMap::new();
    };
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for item in doc.get("items").and_then(|i| i.as_array()).into_iter().flatten() {
        let Some(sha) = item.get("sha256").and_then(|s| s.as_str()) else {
            continue;
        };
        let flags: Vec<String> = item
            .get("flags")
            .and_then(|f| f.as_array())
            .into_iter()
            .flatten()
            .filter_map(|f| f.as_str().map(str::to_string))
            .collect();
        if !flags.is_empty() {
            // Byte-identical copies share a sha — union their flags.
            out.entry(sha.to_string()).or_default().extend(flags);
        }
    }
    for flags in out.values_mut() {
        flags.sort();
        flags.dedup();
    }
    out
}

fn backup_root() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("Pictures").join("iPhone Backup")
}

// ---------------------------------------------------------------------------
// Shared jsonl helpers
// ---------------------------------------------------------------------------

fn load_jsonl(path: &Path) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn sha_set(rows: &[Value]) -> HashSet<String> {
    rows.iter()
        .filter_map(|r| Some(r.get("sha256")?.as_str()?.to_string()))
        .collect()
}

fn append_jsonl(path: &Path, row: &Value) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{row}")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut f, &mut hasher)?;
    Ok(hex(&hasher.finalize()))
}

fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({"error": msg.into()}))).into_response()
}

// ---------------------------------------------------------------------------
// POST /api/media/manifest
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct ManifestBody {
    assets: Vec<ManifestAsset>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestAsset {
    local_id: String,
    sha256: String,
}

/// needed = not staged, not archived → upload. verified = hash-present in the
/// archive ledger → safe to delete on the phone. In staging but not archived →
/// absent from both (a later verify catches it once the archive copy lands).
pub(crate) async fn manifest_handler(Json(body): Json<ManifestBody>) -> Response {
    let loaded = tokio::task::spawn_blocking(|| {
        (
            sha_set(&load_jsonl(&manifest_path())),
            sha_set(&load_jsonl(&ledger_path())),
            load_verdicts(),
        )
    })
    .await;
    let Ok((staged, archived, all_verdicts)) = loaded else {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "manifest load failed");
    };
    let mut needed = Vec::new();
    let mut verified = Vec::new();
    let mut verdicts = serde_json::Map::new();
    for a in &body.assets {
        if archived.contains(&a.sha256) {
            verified.push(a.local_id.clone());
        } else if !staged.contains(&a.sha256) {
            needed.push(a.local_id.clone());
        }
        if let Some(flags) = all_verdicts.get(&a.sha256) {
            verdicts.insert(a.local_id.clone(), json!(flags));
        }
    }
    Json(json!({"needed": needed, "verified": verified, "verdicts": verdicts}))
        .into_response()
}

// ---------------------------------------------------------------------------
// POST /api/media/verify
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VerifyBody {
    local_ids: Vec<String>,
}

pub(crate) async fn verify_handler(Json(body): Json<VerifyBody>) -> Response {
    let loaded = tokio::task::spawn_blocking(|| {
        (load_jsonl(&manifest_path()), sha_set(&load_jsonl(&ledger_path())))
    })
    .await;
    let Ok((rows, archived)) = loaded else {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "verify load failed");
    };
    // localId → sha via the wireless manifest rows (last row wins, like the
    // pipeline's own by-path compaction).
    let mut sha_by_local_id: HashMap<&str, &str> = HashMap::new();
    for r in &rows {
        let (Some(path), Some(sha)) = (r.get("path").and_then(Value::as_str), r.get("sha256").and_then(Value::as_str)) else {
            continue;
        };
        if let Some(local_id) = path.strip_prefix(WIRELESS_PREFIX) {
            sha_by_local_id.insert(local_id, sha);
        }
    }
    let verified: Vec<&String> = body
        .local_ids
        .iter()
        .filter(|id| sha_by_local_id.get(id.as_str()).is_some_and(|sha| archived.contains(*sha)))
        .collect();
    Json(json!({"verified": verified})).into_response()
}

// ---------------------------------------------------------------------------
// POST /api/media/ingest
// ---------------------------------------------------------------------------

/// One original per request: stream the `file` part to a staging temp while
/// hashing, reject on digest mismatch, then stage + archive + ledger it.
/// Idempotent at every step — a retry after any partial failure converges.
pub(crate) async fn ingest_handler(mut multipart: Multipart) -> Response {
    let mut local_id: Option<String> = None;
    let mut declared_sha: Option<String> = None;
    let mut created_ms: Option<i64> = None;
    let mut filename: Option<String> = None;
    let mut received: Option<(PathBuf, String, u64)> = None; // tmp, computed sha, size

    let staging = staging_dir();
    if let Err(e) = tokio::fs::create_dir_all(&staging).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("staging dir: {e}"));
    }
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => {
                discard(&received);
                return err(StatusCode::BAD_REQUEST, format!("multipart: {e}"));
            }
        };
        match field.name().unwrap_or_default() {
            "localId" => local_id = field.text().await.ok(),
            "sha256" => declared_sha = field.text().await.ok().map(|s| s.to_lowercase()),
            "createdEpochMs" => created_ms = field.text().await.ok().and_then(|s| s.parse().ok()),
            "file" => {
                filename = field.file_name().map(sanitize_filename);
                match stream_to_tmp(field, &staging).await {
                    Ok(r) => received = Some(r),
                    Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("receive: {e}")),
                }
            }
            _ => {}
        }
    }

    let (Some(local_id), Some(declared_sha), Some((tmp, computed_sha, size))) =
        (local_id, declared_sha, received)
    else {
        discard(&None);
        return err(StatusCode::BAD_REQUEST, "need localId, sha256 and file parts");
    };
    if computed_sha != declared_sha {
        let _ = std::fs::remove_file(&tmp);
        return err(
            StatusCode::BAD_REQUEST,
            format!("sha256 mismatch: declared {declared_sha}, received {computed_sha}"),
        );
    }

    let filename = filename.unwrap_or_else(|| format!("{}.bin", &computed_sha[..12]));
    let _guard = MEDIA_LOCK.lock().await;
    let finalized = tokio::task::spawn_blocking(move || {
        finalize_ingest(&local_id, &computed_sha, created_ms, &filename, &tmp, size)
    })
    .await;
    match finalized {
        Ok(Ok(())) => {
            schedule_wireless_scan();
            Json(json!({"ok": true})).into_response()
        }
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("ingest task: {e}")),
    }
}

/// Wireless syncs analyze themselves: once ingests go quiet for
/// [`SCAN_QUIESCE`], run the Media pipeline's `scan` (analyzers over staging —
/// no phone involved) so synced photos get dupe/blurry/dark verdicts without
/// a Media-tab visit, and the phone sees them on its next manifest call.
fn schedule_wireless_scan() {
    use std::sync::atomic::Ordering;
    let gen = SCAN_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    tokio::spawn(async move {
        tokio::time::sleep(SCAN_QUIESCE).await;
        if SCAN_GEN.load(Ordering::SeqCst) != gen {
            return; // a newer ingest re-armed the timer
        }
        let _running = SCAN_RUNNING.lock().await;
        if SCAN_GEN.load(Ordering::SeqCst) != gen {
            return; // more uploads landed while a previous scan ran
        }
        run_media_scan().await;
    });
}

/// Invoke the mac-shifu Media pipeline's `scan` with its own venv python.
/// Silently a no-op until the user has run the Media tab's one-time setup —
/// without the venv there are no analyzers to run.
async fn run_media_scan() {
    let py = data_dir().join("venv").join("bin").join("python");
    let pipeline = crate::paths::global_skills_dir()
        .join("mac-shifu")
        .join("scripts")
        .join("media")
        .join("media_pipeline.py");
    if !py.exists() || !pipeline.exists() {
        return;
    }
    tracing::info!("[media] wireless sync quiesced — running pipeline scan");
    match tokio::process::Command::new(&py).arg(&pipeline).arg("scan").output().await {
        Ok(out) if out.status.success() => {
            tracing::info!("[media] post-sync scan done");
        }
        Ok(out) => tracing::warn!(
            "[media] post-sync scan failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(e) => tracing::warn!("[media] post-sync scan spawn failed: {e}"),
    }
}

fn discard(received: &Option<(PathBuf, String, u64)>) {
    if let Some((tmp, _, _)) = received {
        let _ = std::fs::remove_file(tmp);
    }
}

fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim_matches('.').to_string();
    if trimmed.is_empty() { "asset".to_string() } else { trimmed }
}

async fn stream_to_tmp(
    mut field: axum::extract::multipart::Field<'_>,
    staging: &Path,
) -> anyhow::Result<(PathBuf, String, u64)> {
    let tmp = staging.join(format!(".ingest-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = tokio::fs::File::create(&tmp).await?;
    let mut hasher = Sha256::new();
    let mut size: u64 = 0;
    loop {
        let chunk = match field.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                drop(file);
                let _ = std::fs::remove_file(&tmp);
                return Err(e.into());
            }
        };
        hasher.update(&chunk);
        size += chunk.len() as u64;
        if let Err(e) = file.write_all(&chunk).await {
            drop(file);
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
    }
    file.flush().await?;
    Ok((tmp, hex(&hasher.finalize()), size))
}

/// Stage + archive + ledger, under MEDIA_LOCK. Each step no-ops if a previous
/// (possibly partial) run already did it, keyed by content hash.
fn finalize_ingest(
    local_id: &str,
    sha: &str,
    created_ms: Option<i64>,
    filename: &str,
    tmp: &Path,
    size: u64,
) -> anyhow::Result<()> {
    let rows = load_jsonl(&manifest_path());
    let staged_rel = ensure_staged(&rows, sha, filename, tmp)?;
    ensure_archived(sha, created_ms, filename, &staged_rel, size)?;
    ensure_wireless_row(&rows, local_id, sha, created_ms, &staged_rel, size)?;
    Ok(())
}

/// Land the temp file in staging unless this content is already staged; either
/// way return a staged rel path holding the bytes (archive copies from it).
fn ensure_staged(rows: &[Value], sha: &str, filename: &str, tmp: &Path) -> anyhow::Result<String> {
    let existing = rows.iter().find_map(|r| {
        let staged = r.get("staged")?.as_str()?;
        (r.get("sha256")?.as_str()? == sha && staging_dir().join(staged).exists())
            .then(|| staged.to_string())
    });
    if let Some(rel) = existing {
        let _ = std::fs::remove_file(tmp);
        return Ok(rel);
    }
    let rel = format!("{WIRELESS_PREFIX}{}-{filename}", &sha[..12]);
    let dest = staging_dir().join(&rel);
    std::fs::create_dir_all(dest.parent().unwrap_or(&staging_dir()))?;
    std::fs::rename(tmp, &dest)?;
    Ok(rel)
}

/// Copy the staged file into the archive root with a re-hash verify, then
/// append the ledger row the Media pipeline's remove leg trusts.
fn ensure_archived(
    sha: &str,
    created_ms: Option<i64>,
    filename: &str,
    staged_rel: &str,
    size: u64,
) -> anyhow::Result<()> {
    let ledger = ledger_path();
    if sha_set(&load_jsonl(&ledger)).contains(sha) {
        return Ok(());
    }
    let created = created_ms
        .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms))
        .map(|dt| dt.with_timezone(&chrono::Local))
        .unwrap_or_else(chrono::Local::now);
    let dest_dir = backup_root()
        .join(chrono::Local::now().format("%Y-%m-%d").to_string())
        .join(created.format("%Y").to_string())
        .join(created.format("%m").to_string());
    std::fs::create_dir_all(&dest_dir)?;
    let dest = unique_dest(&dest_dir, filename, sha)?;
    if !dest.exists() {
        std::fs::copy(staging_dir().join(staged_rel), &dest)?;
        if sha256_file(&dest)? != sha {
            let _ = std::fs::remove_file(&dest);
            anyhow::bail!("archive copy failed hash verify");
        }
    }
    append_jsonl(
        &ledger,
        &json!({"sha256": sha, "dest": dest.to_string_lossy(), "size": size, "at": now_iso()}),
    )?;
    Ok(())
}

/// First free name in the archive dir for this content: reuse an existing file
/// only when it already holds these exact bytes, else suffix -1, -2, …
fn unique_dest(dir: &Path, filename: &str, sha: &str) -> anyhow::Result<PathBuf> {
    let (stem, ext) = match filename.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (filename.to_string(), String::new()),
    };
    for n in 0..1000 {
        let name = if n == 0 { format!("{stem}{ext}") } else { format!("{stem}-{n}{ext}") };
        let candidate = dir.join(name);
        if !candidate.exists() || sha256_file(&candidate)? == sha {
            return Ok(candidate);
        }
    }
    anyhow::bail!("no free archive name for {filename}")
}

/// Verify maps localId → sha through a `wireless/…` manifest row; make sure
/// one exists for this asset (the staged bytes may sit under a USB row).
fn ensure_wireless_row(
    rows: &[Value],
    local_id: &str,
    sha: &str,
    created_ms: Option<i64>,
    staged_rel: &str,
    size: u64,
) -> anyhow::Result<()> {
    let wire_path = format!("{WIRELESS_PREFIX}{local_id}");
    let already = rows.iter().any(|r| {
        r.get("path").and_then(Value::as_str) == Some(wire_path.as_str())
            && r.get("sha256").and_then(Value::as_str) == Some(sha)
    });
    if already {
        return Ok(());
    }
    let mtime = created_ms.map(|ms| ms / 1000).unwrap_or_else(|| chrono::Local::now().timestamp());
    append_jsonl(
        &manifest_path(),
        &json!({"path": wire_path, "size": size, "mtime": mtime, "sha256": sha, "staged": staged_rel}),
    )?;
    Ok(())
}
