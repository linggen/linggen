//! Generic device sync for skills — the engine's answer to "my app needs to
//! serve files to the phone."
//!
//! A skill's UI only exists while its panel is open, so it can't answer a
//! paired device on its own. Rather than grow a bespoke module per app, a skill
//! **declares** a directory in its frontmatter (`sync:`) and the engine serves
//! it through routes keyed by skill name. Nothing here knows what the files
//! mean — no track, no photo, no app name. See `doc/skill-spec.md` § Device
//! sync, and the kernel-boundary rule in `doc/product-spec.md`.
//!
//! Read-only by design: ingest (a device writing into the Mac) carries
//! delete-safety semantics a declaration can't express.

mod ledger;
mod listing;

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::util::LockExt;

use crate::engine::skill::SyncConfig;
use crate::server::ServerState;

/// A skill's resolved sync surface: its declaration plus the absolute base dir.
struct Surface {
    skill: String,
    cfg: SyncConfig,
    base: PathBuf,
}

/// Skill names reach us from the URL and end up in a ledger filename. Only
/// installed skills get this far, but keep the filename provably safe anyway.
fn safe_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

async fn surface(state: &Arc<ServerState>, skill: &str) -> Result<Surface, Response> {
    let not_found = || (StatusCode::NOT_FOUND, "no sync for that skill").into_response();
    if !safe_skill_name(skill) {
        return Err(not_found());
    }
    let cfg = state
        .skills
        .get_skill(skill)
        .await
        .and_then(|s| s.sync)
        .ok_or_else(not_found)?;
    let base = crate::util::resolve_path(std::path::Path::new(&cfg.dir));
    Ok(Surface {
        skill: skill.to_string(),
        cfg,
        base,
    })
}

impl Surface {
    /// Resolve a declared subdir key to a directory. `None` = the base dir.
    /// An undeclared key is an error, never a path.
    fn dir_for(&self, key: Option<&str>) -> Option<PathBuf> {
        match key {
            None => Some(self.base.clone()),
            Some(k) => self.cfg.subdirs.get(k).map(|sub| self.base.join(sub)),
        }
    }
}

fn has_ext(name: &str, exts: &[String]) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, e)| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

fn stem(name: &str) -> &str {
    name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name)
}

/// GET /api/skill-sync/{skill}/items — every primary file with its size and
/// whichever declared companions actually exist on disk.
pub(crate) async fn get_items(
    State(state): State<Arc<ServerState>>,
    Path(skill): Path<String>,
) -> Response {
    let s = match surface(&state, &skill).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    // A skill installed (or a dir created) after startup gets its watcher
    // the first time a device asks.
    arm_watcher(&state, &s.cfg);
    let items = tokio::task::spawn_blocking(move || build_items(&s, listing::list))
        .await
        .unwrap_or_default();
    Json(serde_json::json!({ "items": items })).into_response()
}

/// The `/items` rows. `list` yields a directory's files with their sizes.
fn build_items(
    s: &Surface,
    list: impl Fn(&std::path::Path) -> Arc<listing::Listing>,
) -> Vec<serde_json::Value> {
    let names = list(&s.base);
    // One listing per distinct companion directory, not per companion.
    let mut pools: HashMap<Option<String>, Arc<listing::Listing>> = HashMap::new();
    pools.insert(None, names.clone());
    for c in &s.cfg.companions {
        if !pools.contains_key(&c.subdir) {
            let pool = s
                .dir_for(c.subdir.as_deref())
                .map(|d| list(&d))
                .unwrap_or_default();
            pools.insert(c.subdir.clone(), pool);
        }
    }

    // `names` is ordered, so the rows come out sorted by name.
    names
        .iter()
        .filter(|(n, _)| has_ext(n, &s.cfg.items))
        .map(|(name, size)| item_row(s, name, *size, &pools))
        .collect()
}

fn item_row(
    s: &Surface,
    name: &str,
    size: u64,
    pools: &HashMap<Option<String>, Arc<listing::Listing>>,
) -> serde_json::Value {
    let base_stem = stem(name);
    let mut item = serde_json::Map::new();
    item.insert("name".into(), name.into());
    item.insert("size".into(), size.into());
    for c in &s.cfg.companions {
        let pool = pools.get(&c.subdir);
        let prefix = format!("{base_stem}{}", c.suffix.as_deref().unwrap_or(""));
        let found = c.exts.iter().find_map(|e| {
            let f = format!("{prefix}.{e}");
            pool.and_then(|p| p.get(&f)).map(|size| (f, *size))
        });
        // The size travels with the name. A companion was presence-only, so
        // a device that already had one never fetched it again — a corrected
        // companion on the Mac could not reach the phone (2026-09-18).
        if let Some((_, size)) = &found {
            item.insert(format!("{}_size", c.name), (*size).into());
        }
        item.insert(c.name.clone(), found.map(|(f, _)| f).into());
    }
    serde_json::Value::Object(item)
}

#[derive(Deserialize)]
pub(crate) struct FileQuery {
    name: String,
    /// A key declared in `sync.subdirs`; absent = the base dir.
    #[serde(default)]
    dir: Option<String>,
}

/// GET /api/skill-sync/{skill}/file?name=…[&dir=…] — serve one file. Plain
/// names only (anything path-like is rejected); the directory is chosen by the
/// declared `dir` key, never by a path inside `name`.
pub(crate) async fn get_file(
    State(state): State<Arc<ServerState>>,
    Path(skill): Path<String>,
    headers: HeaderMap,
    Query(q): Query<FileQuery>,
) -> Response {
    let s = match surface(&state, &skill).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    if q.name.contains('/') || q.name.contains('\\') || q.name.starts_with('.') {
        return (StatusCode::BAD_REQUEST, "bad name").into_response();
    }
    let Some(base) = s.dir_for(q.dir.as_deref()) else {
        return (StatusCode::BAD_REQUEST, "bad dir").into_response();
    };
    let Some((file, len)) = open_file(&base.join(&q.name)).await else {
        return (StatusCode::NOT_FOUND, "no such file").into_response();
    };
    // Only base-dir fetches count toward coverage — companions are extras.
    if q.dir.is_none() {
        if let Some(device) = device_from_headers(&headers) {
            ledger::record_fetch(&s.skill, &device.id, &q.name);
        }
    }
    // Streamed, not read whole: an item can be hundreds of MB.
    let body = Body::from_stream(file_chunks(file));
    (
        [
            (header::CONTENT_TYPE, mime_for(&q.name).to_string()),
            (header::CONTENT_LENGTH, len.to_string()),
        ],
        body,
    )
        .into_response()
}

/// Read size per chunk of a streamed file.
const STREAM_CHUNK: usize = 64 * 1024;

/// A file as a stream of chunks, ending after the first read error.
fn file_chunks(
    file: tokio::fs::File,
) -> impl futures_util::Stream<Item = std::io::Result<axum::body::Bytes>> {
    use tokio::io::AsyncReadExt;
    futures_util::stream::unfold(Some(file), |file| async move {
        let mut file = file?;
        let mut buf = vec![0u8; STREAM_CHUNK];
        match file.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok(buf.into()), Some(file)))
            }
            Err(e) => Some((Err(e), None)),
        }
    })
}

/// Open a regular file for streaming, with its length.
async fn open_file(path: &std::path::Path) -> Option<(tokio::fs::File, u64)> {
    let file = tokio::fs::File::open(path).await.ok()?;
    let meta = file.metadata().await.ok()?;
    meta.is_file().then(|| (file, meta.len()))
}

fn mime_for(name: &str) -> &'static str {
    match name
        .rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
        .unwrap_or_default()
        .as_str()
    {
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "txt" | "lrc" | "vtt" | "srt" => "text/plain; charset=utf-8",
        "json" => "application/json",
        "webp" => "image/webp",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "heic" => "image/heic",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

// ── Per-device sync ledger (see `ledger`) ────────────────────────────────────

/// Which paired device is asking. Loopback callers — the Mac's own UI and
/// skills — are nobody's device and resolve to None.
///
/// Two ways in, because there are two ways a phone reaches this daemon. Over
/// HTTP it carries its own token (header / bearer / cookie), the same sources
/// the LAN gate accepts. Over the WebRTC tunnel it carries no token at all:
/// the peer already authenticated the channel and tags each request with
/// `x-linggen-actor-device`, an id rather than a secret.
///
/// Only the token half authorizes. The tunnel header attributes — it names who
/// is already inside, and anything able to write it can reach loopback and is
/// the Mac's owner anyway. Reading it here is the difference between a ledger
/// that knows what a phone carries and one frozen at the day the transport
/// became WebRTC-only: every `/have` a phone has posted since then was
/// answered 401 and dropped.
fn device_from_headers(headers: &HeaderMap) -> Option<super::pair::PairedDevice> {
    let devices = super::pair::load_devices();
    let token = headers
        .get("x-linggen-device")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
        })
        .or_else(|| {
            headers
                .get(header::COOKIE)
                .and_then(|v| v.to_str().ok())
                .and_then(|c| {
                    c.split(';')
                        .map(str::trim)
                        .find_map(|kv| kv.strip_prefix("linggen_device="))
                })
        });
    if let Some(token) = token {
        return devices.into_iter().find(|d| d.secret == token);
    }
    let id = headers
        .get(super::pair::ACTOR_DEVICE_HEADER)
        .and_then(|v| v.to_str().ok())?;
    devices.into_iter().find(|d| d.id == id)
}

#[derive(Deserialize)]
pub(crate) struct HaveBody {
    files: Vec<String>,
}

/// POST /api/skill-sync/{skill}/have — the device reports its full on-device
/// inventory after a sync, replacing its ledger row. Fetch-recording alone
/// can't get there: files synced before the ledger existed are never
/// re-fetched, and files deleted on the device would stay marked as synced.
pub(crate) async fn post_have(
    State(state): State<Arc<ServerState>>,
    Path(skill): Path<String>,
    headers: HeaderMap,
    Json(body): Json<HaveBody>,
) -> Response {
    let s = match surface(&state, &skill).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let Some(device) = device_from_headers(&headers) else {
        return (StatusCode::UNAUTHORIZED, "paired devices only").into_response();
    };
    let now = chrono::Utc::now().timestamp();
    let saved = tokio::task::spawn_blocking(move || {
        ledger::STORE.replace(&s.skill, &device.id, body.files, now)
    })
    .await
    .unwrap_or(false);
    if !saved {
        return (StatusCode::INTERNAL_SERVER_ERROR, "persist failed").into_response();
    }
    Json(serde_json::json!({ "status": "ok" })).into_response()
}

/// GET /api/skill-sync/{skill}/devices — paired devices joined with their sync
/// ledger. The skill's own UI compares `files` against `/items` to render
/// per-device coverage.
pub(crate) async fn get_devices(
    State(state): State<Arc<ServerState>>,
    Path(skill): Path<String>,
) -> Response {
    let s = match surface(&state, &skill).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let ledger = ledger::STORE.snapshot(&s.skill);
    // Whether the device is here RIGHT NOW, which the ledger cannot say — it
    // records what was last fetched, not who is holding a channel. A page that
    // pushes work at a device has to tell the difference to describe what it
    // just did: something a connected device starts on immediately reads very
    // differently from something waiting for one to wake up.
    let present = crate::perception::devices::present_ids();
    let devices: Vec<serde_json::Value> = super::pair::load_devices()
        .iter()
        .map(|d| {
            let row = ledger.get(&d.id).cloned().unwrap_or_default();
            serde_json::json!({
                "id": d.id,
                "name": d.name,
                "files": row.files,
                "last_fetch": (row.last_fetch > 0).then_some(row.last_fetch),
                "present": present.contains(&d.id),
            })
        })
        .collect();
    Json(serde_json::json!({ "devices": devices })).into_response()
}

/// Start a change-watcher for every installed skill that declared a sync
/// `topic`, so paired devices are pushed to instead of polling.
pub(crate) async fn spawn_watchers(state: Arc<ServerState>) {
    for skill in state.skills.list_skills().await {
        if let Some(cfg) = skill.sync {
            arm_watcher(&state, &cfg);
        }
    }
}

/// `(dir, topic)` pairs with a live watcher.
static ARMED: Mutex<Option<HashSet<(PathBuf, String)>>> = Mutex::new(None);

/// Watch a skill's sync dir if it declared a topic and nothing watches it yet.
/// The dir is the skill's own declaration, so a missing one is created —
/// otherwise a fresh install armed nothing and devices never heard a change
/// until the daemon restarted. Idempotent; cheap after the first call.
fn arm_watcher(state: &Arc<ServerState>, cfg: &SyncConfig) {
    let Some(topic) = cfg.topic.clone() else {
        return;
    };
    let dir = crate::util::resolve_path(std::path::Path::new(&cfg.dir));
    let key = (dir.clone(), topic.clone());
    let mut armed = ARMED.lock_ok();
    let armed = armed.get_or_insert_with(HashSet::new);
    if armed.contains(&key) {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("[skill-sync] cannot create {}: {e}", dir.display());
        return;
    }
    let armed_ok = super::topic::watch_dir(
        state.clone(),
        dir.clone(),
        topic,
        "library-changed".to_string(),
        std::time::Duration::from_secs(2),
        Some(counts_as_change),
    );
    if armed_ok {
        listing::mark_watched(&dir);
        armed.insert(key);
    }
}

/// Whether a filesystem event under a sync dir is a real change. In-progress
/// downloads and temp files churn for minutes and would ring the topic over
/// and over; the rename that finishes them is what counts. A real change also
/// drops the cached listings.
fn counts_as_change(path: &std::path::Path) -> bool {
    if is_scratch(path) {
        return false;
    }
    listing::invalidate();
    true
}

/// Dotfiles and the usual partial-download / temp-file names.
fn is_scratch(path: &std::path::Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    const SCRATCH_EXTS: &[&str] = &["part", "tmp", "temp", "crdownload", "download", "ytdl"];
    name.starts_with('.')
        || lower.contains(".part-")
        || lower
            .rsplit_once('.')
            .is_some_and(|(_, ext)| SCRATCH_EXTS.contains(&ext))
}

#[cfg(test)]
mod tests;
