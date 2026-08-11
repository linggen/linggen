//! Wireless media sync — Linggen Mobile's paired Photos backup.
//!
//! Contract: `linggen-mobile/doc/shifu.md`. The load-bearing ones are manifest
//! (what does the Mac need / already hold), reconcile (the phone's whole roll,
//! which prunes what it deleted), backup, and verify (which uploads are now
//! safe to delete on-phone). Bytes arrive on the media channel; `ingest`, the
//! multipart route, predates that and currently has no caller.
//!
//! Files land in the apple-shifu Media pipeline's own staging + archive, so the
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
    crate::paths::global_skills_dir()
        .join("apple-shifu")
        .join("data")
        .join("media")
}

pub(crate) fn staging_dir() -> PathBuf {
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

/// Mac-requested phone deletions the phone hasn't executed yet.
fn delete_queue_path() -> PathBuf {
    data_dir().join("phone-delete-queue.json")
}

// There were file watchers here announcing `media/delete-requested` and
// `media/verdicts-updated`. They are gone deliberately.
//
// A watcher earns its place when the directory is written by the outside world
// — `~/Music/DJ` gets files from Finder, a download, or the agent, and nothing
// else would notice. This directory is written only by our own API, so the
// watcher was machinery for discovering something we had just done ourselves.
//
// The phone gets the queue two ways that already cover every real path: it
// reads `deleteRequested` off every manifest reply, so tapping Sync delivers
// it, and it fetches once whenever a route to the Mac appears — app start,
// resume, reconnect. The watcher only added the case where the phone was
// foreground *and* connected at the instant of the click, and iOS suspends the
// app within seconds of backgrounding anyway.
//
// `verdicts-updated` was weaker still: the phone's handler only repainted, and
// verdicts arrive on the manifest reply, so it redrew data it did not yet have.

/// One queued deletion. `device` is the paired-device row the photo came from,
/// so a household with several phones asks the right one.
///
/// A localId is a PhotoKit identifier, which means something only on the phone
/// that minted it — a queue without `device` is ambiguous the moment a second
/// phone pairs. `None` means "we don't know which phone", which is how rows
/// queued before this existed, and USB-era rows, still behave: offered to
/// everyone, exactly as they were — until a reconcile finds the photo on some
/// phone's roll and adopts the entry for it (presence is proof of ownership).
#[derive(Clone, Debug)]
struct DeleteEntry {
    local_id: String,
    device: Option<String>,
}

impl DeleteEntry {
    /// Is this entry this phone's business?
    fn concerns(&self, device: Option<&str>) -> bool {
        match (&self.device, device) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(owner), Some(asking)) => owner == asking,
        }
    }
}

fn load_delete_queue() -> Vec<DeleteEntry> {
    let Some(v) = std::fs::read_to_string(delete_queue_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    else {
        return Vec::new();
    };
    // The old shape was a bare `{"localIds": [...]}` with no device. Read it as
    // unscoped entries so an upgrade never drops a pending deletion.
    if let Some(ids) = v.get("localIds").and_then(Value::as_array) {
        return ids
            .iter()
            .filter_map(Value::as_str)
            .map(|id| DeleteEntry {
                local_id: id.to_string(),
                device: None,
            })
            .collect();
    }
    v.get("queue")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    Some(DeleteEntry {
                        local_id: r.get("localId")?.as_str()?.to_string(),
                        device: r.get("device").and_then(Value::as_str).map(String::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn save_delete_queue(entries: &[DeleteEntry]) -> std::io::Result<()> {
    let rows: Vec<Value> = entries
        .iter()
        .map(|e| json!({ "localId": e.local_id, "device": e.device }))
        .collect();
    std::fs::write(delete_queue_path(), json!({ "queue": rows }).to_string())
}

/// Stamp `by.device` on unclaimed wireless rows whose localId this roll
/// contains. Only the device is asserted — the account stays absent, which
/// every reader already treats as "unknown", never "nobody".
fn adopt_unclaimed_rows(rows: &mut [Value], on_phone: &HashSet<&str>, me: &str) -> bool {
    let mut adopted = false;
    for r in rows.iter_mut() {
        let unclaimed = r
            .get("by")
            .and_then(|b| b.get("device"))
            .and_then(Value::as_str)
            .is_none();
        let mine = r
            .get("path")
            .and_then(Value::as_str)
            .and_then(|p| p.strip_prefix(WIRELESS_PREFIX))
            .is_some_and(|local_id| on_phone.contains(local_id));
        if !unclaimed || !mine {
            continue;
        }
        match r.get_mut("by") {
            Some(Value::Object(by)) => {
                by.insert("device".to_string(), json!(me));
            }
            _ => r["by"] = json!({ "device": me }),
        }
        adopted = true;
    }
    adopted
}

/// Which phone holds this photo, per the manifest row written when it arrived.
fn owner_of(rows: &[Value], local_id: &str) -> Option<String> {
    let wire_path = format!("{WIRELESS_PREFIX}{local_id}");
    rows.iter()
        .find(|r| r.get("path").and_then(Value::as_str) == Some(wire_path.as_str()))
        .and_then(|r| r.get("by")?.get("device")?.as_str().map(String::from))
}

/// Apple Shifu's scan verdicts (blurry/dark/…), keyed by content hash. The
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
    for item in doc
        .get("items")
        .and_then(|i| i.as_array())
        .into_iter()
        .flatten()
    {
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
    dirs::home_dir()
        .unwrap_or_default()
        .join("Pictures")
        .join("iPhone Backup")
}

// ---------------------------------------------------------------------------
// Shared jsonl helpers
// ---------------------------------------------------------------------------

fn load_jsonl(path: &Path) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn sha_set(rows: &[Value]) -> HashSet<String> {
    rows.iter()
        .filter_map(|r| Some(r.get("sha256")?.as_str()?.to_string()))
        .collect()
}

fn rewrite_jsonl(path: &Path, rows: &[&Value]) -> std::io::Result<()> {
    let tmp = path.with_extension("jsonl.tmp");
    let mut f = std::fs::File::create(&tmp)?;
    for r in rows {
        writeln!(f, "{r}")?;
    }
    std::fs::rename(&tmp, path)
}

fn append_jsonl(path: &Path, row: &Value) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
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
pub(crate) async fn manifest_handler(
    headers: axum::http::HeaderMap,
    Json(body): Json<ManifestBody>,
) -> Response {
    let asking = crate::server::api::pair::caller_device(&headers);
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
    // Mac-requested deletions ride every manifest reply; the phone executes
    // them via PhotoKit (system confirm) and reconcile clears the queue. Only
    // this phone's entries — another phone's localIds mean nothing here.
    let delete_requested: Vec<String> = load_delete_queue()
        .into_iter()
        .filter(|e| e.concerns(asking.as_deref()))
        .map(|e| e.local_id)
        .collect();
    Json(json!({
        "needed": needed,
        "verified": verified,
        "verdicts": verdicts,
        "deleteRequested": delete_requested,
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// POST /api/media/request-delete · GET /api/media/pending-deletes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestDeleteBody {
    local_ids: Vec<String>,
    /// true = remove these ids from the queue (the Unqueue undo).
    #[serde(default)]
    cancel: bool,
}

/// The Mac review queues photos for on-phone deletion. The queue is intent,
/// not action: nothing is touched until the phone's user confirms the
/// PhotoKit dialog; cache rows stay until reconcile sees the photo gone.
/// `cancel: true` withdraws ids — queued intent stays reversible.
pub(crate) async fn request_delete_handler(Json(body): Json<RequestDeleteBody>) -> Response {
    let _guard = MEDIA_LOCK.lock().await;
    let mut queue = load_delete_queue();
    if body.cancel {
        queue.retain(|e| !body.local_ids.contains(&e.local_id));
    } else {
        // Which phone to ask is a property of the photo, not of whoever is
        // clicking on the Mac — read it off the row written when it arrived.
        let rows = load_jsonl(&manifest_path());
        for id in body.local_ids {
            if queue.iter().any(|e| e.local_id == id) {
                continue;
            }
            let device = owner_of(&rows, &id);
            queue.push(DeleteEntry {
                local_id: id,
                device,
            });
        }
    }
    let n = queue.len();
    if let Err(e) = save_delete_queue(&queue) {
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("queue write: {e}"),
        );
    }
    tracing::info!("[media] phone-delete queue now {n} ids");
    Json(json!({"queued": n})).into_response()
}

/// The queue as the caller may see it. A phone reads it whenever a route to
/// the Mac appears — app start, resume, reconnect — and gets its own entries
/// plus unclaimed ones. A caller with no device identity is the Mac's own
/// Media page (loopback, no tunnel stamp): it owns the queue and sees all of
/// it — scoping it like a phone made every queued badge vanish on reload.
pub(crate) async fn pending_deletes_handler(headers: axum::http::HeaderMap) -> Response {
    let asking = crate::server::api::pair::caller_device(&headers);
    let ids: Vec<String> = load_delete_queue()
        .into_iter()
        .filter(|e| asking.is_none() || e.concerns(asking.as_deref()))
        .map(|e| e.local_id)
        .collect();
    Json(json!({ "localIds": ids })).into_response()
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
        (
            load_jsonl(&manifest_path()),
            sha_set(&load_jsonl(&ledger_path())),
        )
    })
    .await;
    let Ok((rows, archived)) = loaded else {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "verify load failed");
    };
    // localId → sha via the wireless manifest rows (last row wins, like the
    // pipeline's own by-path compaction).
    let mut sha_by_local_id: HashMap<&str, &str> = HashMap::new();
    for r in &rows {
        let (Some(path), Some(sha)) = (
            r.get("path").and_then(Value::as_str),
            r.get("sha256").and_then(Value::as_str),
        ) else {
            continue;
        };
        if let Some(local_id) = path.strip_prefix(WIRELESS_PREFIX) {
            sha_by_local_id.insert(local_id, sha);
        }
    }
    let verified: Vec<&String> = body
        .local_ids
        .iter()
        .filter(|id| {
            sha_by_local_id
                .get(id.as_str())
                .is_some_and(|sha| archived.contains(*sha))
        })
        .collect();
    Json(json!({"verified": verified})).into_response()
}

// ---------------------------------------------------------------------------
// POST /api/media/reconcile
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReconcileBody {
    all_local_ids: Vec<String>,
}

/// The mirror's delete leg (phone → Mac): after a sync the phone posts its
/// complete roll; wireless rows whose asset no longer exists on the phone are
/// pruned, along with staged files no surviving row references. The archive
/// is never touched — backup copies outlive phone deletions.
pub(crate) async fn reconcile_handler(
    headers: axum::http::HeaderMap,
    Json(body): Json<ReconcileBody>,
) -> Response {
    let asking = crate::server::api::pair::caller_device(&headers);
    // An empty roll is indistinguishable from a client that failed to index
    // (e.g. Photos permission revoked) — never treat it as "delete everything".
    if body.all_local_ids.is_empty() {
        return Json(json!({"pruned": 0})).into_response();
    }
    let _guard = MEDIA_LOCK.lock().await;
    let pruned = tokio::task::spawn_blocking(move || {
        reconcile_wireless(&body.all_local_ids, asking.as_deref())
    })
    .await;
    match pruned {
        Ok(Ok(n)) => {
            if n > 0 {
                tracing::info!("[media] reconcile pruned {n} wireless rows");
                schedule_wireless_scan(); // flags must drop the pruned items
            }
            Json(json!({"pruned": n})).into_response()
        }
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("reconcile task: {e}"),
        ),
    }
}

fn reconcile_wireless(all: &[String], asking: Option<&str>) -> anyhow::Result<usize> {
    let on_phone: HashSet<&str> = all.iter().map(String::as_str).collect();
    // A roll speaks only for its own phone. With one device row paired nothing
    // is ambiguous, so a row nobody claims must be that phone's — with two
    // (and a long-kept sim row counts), it genuinely could be either, and
    // guessing would delete someone's photos.
    let alone = crate::server::api::pair::paired_device_count() <= 1;
    let speaks_for = |owner: Option<&str>| match (owner, asking) {
        (Some(o), Some(me)) => o == me,
        (Some(_), None) => false,
        (None, _) => alone,
    };

    // ADOPTION, before any pruning: an unclaimed row whose localId appears in
    // this roll is this phone's — localIds are phone-minted, so presence is
    // proof of ownership where absence stays ambiguous. Claiming on evidence
    // is what lets pre-attribution rows drain even on a Mac whose device list
    // never gets down to one (a kept sim row pins `alone` false forever).
    let mut queue = load_delete_queue();
    let mut adopted = false;
    if let Some(me) = asking {
        for e in queue.iter_mut() {
            if e.device.is_none() && on_phone.contains(e.local_id.as_str()) {
                e.device = Some(me.to_string());
                adopted = true;
            }
        }
    }

    // Queue entries whose photo is no longer on the phone are done (executed,
    // or deleted by hand) — the roll report is the ack, no protocol needed.
    let live: Vec<DeleteEntry> = queue
        .iter()
        .filter(|e| !speaks_for(e.device.as_deref()) || on_phone.contains(e.local_id.as_str()))
        .cloned()
        .collect();
    if adopted || live.len() != queue.len() {
        let _ = save_delete_queue(&live);
    }
    let mut rows = load_jsonl(&manifest_path());
    let rows_adopted = match asking {
        Some(me) => adopt_unclaimed_rows(&mut rows, &on_phone, me),
        None => false,
    };
    let gone = |r: &Value| {
        if !speaks_for(
            r.get("by")
                .and_then(|b| b.get("device"))
                .and_then(Value::as_str),
        ) {
            return false;
        }
        r.get("path")
            .and_then(Value::as_str)
            .and_then(|p| p.strip_prefix(WIRELESS_PREFIX))
            .is_some_and(|local_id| !on_phone.contains(local_id))
    };
    let (pruned, kept): (Vec<&Value>, Vec<&Value>) = rows.iter().partition(|r| gone(r));
    if pruned.is_empty() {
        if rows_adopted {
            rewrite_jsonl(&manifest_path(), &kept)?;
        }
        return Ok(0);
    }
    let referenced: HashSet<&str> = kept
        .iter()
        .filter_map(|r| r.get("staged").and_then(Value::as_str))
        .collect();
    for r in &pruned {
        if let Some(staged) = r.get("staged").and_then(Value::as_str) {
            if !referenced.contains(staged) {
                let _ = std::fs::remove_file(staging_dir().join(staged));
            }
        }
    }
    rewrite_jsonl(&manifest_path(), &kept)?;
    Ok(pruned.len())
}

// ---------------------------------------------------------------------------
// POST /api/media/backup
// ---------------------------------------------------------------------------

/// The explicit backup step, phone-triggered: copy every staged wireless
/// original into `~/Pictures/iPhone Backup` with a re-hash verify and append
/// the ledger — the same gate the USB flow's Back up all writes. Idempotent.
pub(crate) async fn backup_handler() -> Response {
    let _guard = MEDIA_LOCK.lock().await;
    let done = tokio::task::spawn_blocking(backup_wireless).await;
    match done {
        Ok(Ok((archived, failed))) => {
            if archived > 0 {
                tracing::info!("[media] phone backup archived {archived} originals");
            }
            Json(json!({"archived": archived, "failed": failed})).into_response()
        }
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("backup task: {e}"),
        ),
    }
}

fn backup_wireless() -> anyhow::Result<(usize, usize)> {
    let rows = load_jsonl(&manifest_path());
    let archived_shas = sha_set(&load_jsonl(&ledger_path()));
    let mut done = HashSet::new();
    let (mut archived, mut failed) = (0usize, 0usize);
    for r in &rows {
        let is_wireless = r
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|p| p.starts_with(WIRELESS_PREFIX));
        let (Some(sha), Some(staged)) = (
            r.get("sha256").and_then(Value::as_str),
            r.get("staged").and_then(Value::as_str),
        ) else {
            continue;
        };
        if !is_wireless || archived_shas.contains(sha) || !done.insert(sha.to_string()) {
            continue;
        }
        if !staging_dir().join(staged).exists() {
            continue;
        }
        // staged rel is `wireless/<sha12>-<original filename>`
        let filename = staged
            .strip_prefix(WIRELESS_PREFIX)
            .and_then(|n| n.get(13..))
            .unwrap_or(staged);
        let created_ms = r.get("mtime").and_then(Value::as_i64).map(|s| s * 1000);
        let size = r.get("size").and_then(Value::as_u64).unwrap_or(0);
        // Attribution rides from the manifest row written at upload time —
        // backup is a Mac-side action, so "who is connected now" is the wrong
        // answer and may be nobody.
        let by = r.get("by").cloned();
        match ensure_archived(sha, created_ms, filename, staged, size, by, &archived_shas) {
            Ok(()) => archived += 1,
            Err(e) => {
                tracing::warn!("[media] backup failed for {staged}: {e}");
                failed += 1;
            }
        }
    }
    Ok((archived, failed))
}

// ---------------------------------------------------------------------------
// POST /api/media/ingest
// ---------------------------------------------------------------------------

/// One original per request: stream the `file` part to a staging temp while
/// hashing, reject on digest mismatch, then stage + archive + ledger it.
/// Idempotent at every step — a retry after any partial failure converges.
pub(crate) async fn ingest_handler(
    headers: axum::http::HeaderMap,
    mut multipart: Multipart,
) -> Response {
    // The channel is the live upload path; this route survives for the USB-era
    // flow and for curl. Attribute it the only way an HTTP caller can be known.
    let by = crate::server::api::pair::actor_for_headers(&headers);
    let mut local_id: Option<String> = None;
    let mut declared_sha: Option<String> = None;
    let mut created_ms: Option<i64> = None;
    let mut filename: Option<String> = None;
    let mut received: Option<(PathBuf, String, u64)> = None; // tmp, computed sha, size

    let staging = staging_dir();
    if let Err(e) = tokio::fs::create_dir_all(&staging).await {
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("staging dir: {e}"),
        );
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
                    Err(e) => {
                        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("receive: {e}"))
                    }
                }
            }
            _ => {}
        }
    }

    let (Some(local_id), Some(declared_sha), Some((tmp, computed_sha, size))) =
        (local_id, declared_sha, received)
    else {
        discard(&None);
        return err(
            StatusCode::BAD_REQUEST,
            "need localId, sha256 and file parts",
        );
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
        finalize_ingest(
            &local_id,
            &computed_sha,
            created_ms,
            &filename,
            &tmp,
            size,
            by,
        )
    })
    .await;
    match finalized {
        Ok(Ok(())) => {
            schedule_wireless_scan();
            Json(json!({"ok": true})).into_response()
        }
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("ingest task: {e}"),
        ),
    }
}

/// Wireless syncs analyze themselves: once ingests go quiet for
/// [`SCAN_QUIESCE`], run the Media pipeline's `scan` (analyzers over staging —
/// no phone involved) so synced photos get dupe/blurry/dark verdicts without
/// a Media-tab visit, and the phone sees them on its next manifest call.
pub(crate) fn schedule_wireless_scan() {
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

/// Invoke the apple-shifu Media pipeline's `scan` with its own venv python.
/// Silently a no-op until the user has run the Media tab's one-time setup —
/// without the venv there are no analyzers to run.
async fn run_media_scan() {
    let py = data_dir().join("venv").join("bin").join("python");
    let pipeline = crate::paths::global_skills_dir()
        .join("apple-shifu")
        .join("scripts")
        .join("media")
        .join("media_pipeline.py");
    if !py.exists() || !pipeline.exists() {
        return;
    }
    tracing::info!("[media] wireless sync quiesced — running pipeline scan");
    match tokio::process::Command::new(&py)
        .arg(&pipeline)
        .arg("scan")
        .output()
        .await
    {
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

pub(crate) fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('.').to_string();
    if trimmed.is_empty() {
        "asset".to_string()
    } else {
        trimmed
    }
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

/// Stage + manifest row, under MEDIA_LOCK. Each step no-ops if a previous
/// (possibly partial) run already did it, keyed by content hash.
///
/// Sync is a MIRROR, not a backup: ingest deliberately does NOT archive.
/// The archive copy (+ ledger row, the phone's delete gate) happens only in
/// the explicit backup step (`backup_handler` / the Mac's Back up all).
pub(crate) fn finalize_ingest(
    local_id: &str,
    sha: &str,
    created_ms: Option<i64>,
    filename: &str,
    tmp: &Path,
    size: u64,
    by: Option<crate::server::api::pair::Actor>,
) -> anyhow::Result<()> {
    let rows = load_jsonl(&manifest_path());
    let staged_rel = ensure_staged(&rows, sha, filename, tmp)?;
    ensure_wireless_row(&rows, local_id, sha, created_ms, &staged_rel, size, by)?;
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
/// Copy one staged original into the archive and record it.
///
/// `known` is the caller's already-loaded set of archived shas. It is required
/// rather than optional because rebuilding it here re-read and re-parsed the
/// whole ledger once per file: a 1,000-photo backup against a 3,000-row ledger
/// did three million redundant line parses.
fn ensure_archived(
    sha: &str,
    created_ms: Option<i64>,
    filename: &str,
    staged_rel: &str,
    size: u64,
    by: Option<Value>,
    known: &HashSet<String>,
) -> anyhow::Result<()> {
    let ledger = ledger_path();
    if known.contains(sha) {
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
    let mut row =
        json!({"sha256": sha, "dest": dest.to_string_lossy(), "size": size, "at": now_iso()});
    // Content dedupes by sha, so the first phone to get a copy archived owns
    // the row. There is one file; it came from someone.
    if let Some(a) = by {
        stamp_xattr(&dest, &a);
        row["by"] = a;
    }
    append_jsonl(&ledger, &row)?;
    Ok(())
}

/// Mirror the ledger's `by` onto the file itself, so Finder and Spotlight can
/// answer "whose is this" without Linggen running.
///
/// An export, never the record: extended attributes do not survive a zip, a
/// non-native filesystem, or most upload paths, and the archive already writes
/// to external disks. The ledger stays the truth; this is a convenience that is
/// allowed to be missing. Deliberately does not touch the bytes — rewriting the
/// file would change its sha256, which is the delete gate.
#[cfg(unix)]
fn stamp_xattr(dest: &Path, by: &Value) {
    let out = std::process::Command::new("xattr")
        .args(["-w", "com.linggen.by", &by.to_string()])
        .arg(dest)
        .output();
    if let Ok(o) = out {
        if !o.status.success() {
            tracing::debug!("[media] xattr stamp skipped for {}", dest.display());
        }
    }
}

#[cfg(not(unix))]
fn stamp_xattr(_dest: &Path, _by: &Value) {}

/// First free name in the archive dir for this content: reuse an existing file
/// only when it already holds these exact bytes, else suffix -1, -2, …
fn unique_dest(dir: &Path, filename: &str, sha: &str) -> anyhow::Result<PathBuf> {
    let (stem, ext) = match filename.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (filename.to_string(), String::new()),
    };
    for n in 0..1000 {
        let name = if n == 0 {
            format!("{stem}{ext}")
        } else {
            format!("{stem}-{n}{ext}")
        };
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
    by: Option<crate::server::api::pair::Actor>,
) -> anyhow::Result<()> {
    let wire_path = format!("{WIRELESS_PREFIX}{local_id}");
    let already = rows.iter().any(|r| {
        r.get("path").and_then(Value::as_str) == Some(wire_path.as_str())
            && r.get("sha256").and_then(Value::as_str) == Some(sha)
    });
    if already {
        return Ok(());
    }
    let mtime = created_ms
        .map(|ms| ms / 1000)
        .unwrap_or_else(|| chrono::Local::now().timestamp());
    let mut row = json!({"path": wire_path, "size": size, "mtime": mtime, "sha256": sha, "staged": staged_rel});
    // Whose phone sent it. Absent for an anonymous peer and for the USB pull —
    // a row with no `by` means "we don't know", never "nobody".
    if let Some(a) = by {
        row["by"] = serde_json::to_value(a).unwrap_or(Value::Null);
    }
    append_jsonl(&manifest_path(), &row)?;
    Ok(())
}
