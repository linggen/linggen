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

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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

fn read_names(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
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
    let names = read_names(&s.base);

    // One listing per distinct companion directory, not per companion.
    let mut listings: HashMap<Option<String>, Vec<String>> = HashMap::new();
    listings.insert(None, names.clone());
    for c in &s.cfg.companions {
        listings.entry(c.subdir.clone()).or_insert_with(|| {
            s.dir_for(c.subdir.as_deref())
                .map(|d| read_names(&d))
                .unwrap_or_default()
        });
    }

    let mut items = Vec::new();
    for name in names.iter().filter(|n| has_ext(n, &s.cfg.items)) {
        let base_stem = stem(name);
        let mut item = serde_json::Map::new();
        item.insert("name".into(), name.clone().into());
        item.insert(
            "size".into(),
            std::fs::metadata(s.base.join(name))
                .map(|m| m.len())
                .unwrap_or(0)
                .into(),
        );
        for c in &s.cfg.companions {
            let pool = listings.get(&c.subdir).map(Vec::as_slice).unwrap_or(&[]);
            let prefix = format!("{base_stem}{}", c.suffix.as_deref().unwrap_or(""));
            let found = c
                .exts
                .iter()
                .map(|e| format!("{prefix}.{e}"))
                .find(|f| pool.contains(f));
            item.insert(c.name.clone(), found.into());
        }
        items.push(serde_json::Value::Object(item));
    }
    items.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Json(serde_json::json!({ "items": items })).into_response()
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
    let Ok(bytes) = tokio::fs::read(base.join(&q.name)).await else {
        return (StatusCode::NOT_FOUND, "no such file").into_response();
    };
    // Only base-dir fetches count toward coverage — companions are extras.
    if q.dir.is_none() {
        if let Some(device) = device_from_headers(&headers) {
            record_fetch(&s.skill, &device.id, &q.name);
        }
    }
    ([(header::CONTENT_TYPE, mime_for(&q.name))], bytes).into_response()
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

// ── Per-device sync ledger ───────────────────────────────────────────────────
//
// `~/.linggen/sync/<skill>.json`: which of the skill's items each paired device
// holds. Written on every base-dir fetch; the skill's own UI reads it back via
// `/devices` to show true per-phone coverage. Keyed by `PairedDevice.id`, so
// revoking a device orphans (not corrupts) its row.

#[derive(serde::Serialize, Deserialize, Default, Clone)]
struct DeviceSync {
    files: Vec<String>,
    last_fetch: i64,
}

fn ledger_path(skill: &str) -> PathBuf {
    let dir = crate::paths::linggen_home().join("sync");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{skill}.json"))
}

/// Serializes read-modify-write of the ledgers; devices sync sequentially, but
/// nothing enforces that, and several skills can be syncing at once.
static SYNC_LOCK: Mutex<()> = Mutex::new(());

fn load_ledger(skill: &str) -> HashMap<String, DeviceSync> {
    std::fs::read_to_string(ledger_path(skill))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_ledger(skill: &str, ledger: &HashMap<String, DeviceSync>) -> bool {
    serde_json::to_string_pretty(ledger)
        .map(|t| std::fs::write(ledger_path(skill), t).is_ok())
        .unwrap_or(false)
}

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

fn record_fetch(skill: &str, device_id: &str, name: &str) {
    let _guard = SYNC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut ledger = load_ledger(skill);
    let entry = ledger.entry(device_id.to_string()).or_default();
    if !entry.files.iter().any(|f| f == name) {
        entry.files.push(name.to_string());
    }
    entry.last_fetch = chrono::Utc::now().timestamp();
    save_ledger(skill, &ledger);
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
    let _guard = SYNC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut ledger = load_ledger(&s.skill);
    let entry = ledger.entry(device.id).or_default();
    entry.files = body.files;
    entry.last_fetch = chrono::Utc::now().timestamp();
    if !save_ledger(&s.skill, &ledger) {
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
    let ledger = load_ledger(&s.skill);
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
        let Some(cfg) = skill.sync else { continue };
        let Some(topic) = cfg.topic else { continue };
        let dir = crate::util::resolve_path(std::path::Path::new(&cfg.dir));
        super::topic::watch_dir(
            state.clone(),
            dir,
            topic,
            "library-changed".to_string(),
            std::time::Duration::from_secs(2),
            None,
        );
    }
}
