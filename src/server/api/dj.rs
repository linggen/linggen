//! DJ library sync for Linggen Mobile — the phone becomes DJ's native sync
//! target (replacing the old VLC push). Read-only over the paired channel:
//! list `~/Music/DJ`, then fetch tracks + `.lrc` lyric sidecars + covers by
//! name. Both routes sit behind the LAN gate like everything else.

use axum::{
    extract::Query,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

const AUDIO_EXTS: &[&str] = &["mp3", "m4a", "flac", "wav", "ogg", "aac"];
const COVER_EXTS: &[&str] = &["webp", "jpg", "jpeg", "png"];

fn dj_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("Music").join("DJ")
}

/// Karaoke sources (instrumental mp3 + karaoke video) live in a hidden
/// `.karaoke/` subdir so the library scan never adopts them as tracks. The
/// phone reaches them via `/api/dj/file?dir=karaoke&name=…`.
fn karaoke_dir() -> PathBuf {
    dj_dir().join(".karaoke")
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

fn is_ext(name: &str, exts: &[&str]) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, e)| exts.contains(&e.to_lowercase().as_str()))
}

/// Announce library changes so paired phones sync on push instead of polling.
pub(crate) fn spawn_library_watcher(state: std::sync::Arc<crate::server::ServerState>) {
    super::topic::watch_dir(
        state,
        dj_dir(),
        "dj",
        "library-changed",
        std::time::Duration::from_secs(2),
        None,
    );
}

/// GET /api/dj/library — every audio file with its sidecar availability
/// (`.lrc`, cover) plus the karaoke sources in `.karaoke/` (instrumental mp3
/// and karaoke video), keyed off the track's stem + " (Karaoke)".
pub(crate) async fn get_library() -> impl IntoResponse {
    let dir = dj_dir();
    let names = read_names(&dir);
    let knames = read_names(&karaoke_dir());
    let mut tracks = Vec::new();
    for name in names.iter().filter(|n| is_ext(n, AUDIO_EXTS)) {
        let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
        let lrc = format!("{stem}.lrc");
        let cover = COVER_EXTS
            .iter()
            .map(|e| format!("{stem}.{e}"))
            .find(|c| names.contains(c));
        let karaoke_audio = format!("{stem} (Karaoke).mp3");
        let karaoke_video = format!("{stem} (Karaoke).mp4");
        let size = std::fs::metadata(dir.join(name)).map(|m| m.len()).unwrap_or(0);
        tracks.push(serde_json::json!({
            "name": name,
            "size": size,
            "lrc": names.contains(&lrc).then_some(lrc),
            "cover": cover,
            "karaoke_audio": knames.contains(&karaoke_audio).then_some(karaoke_audio),
            "karaoke_video": knames.contains(&karaoke_video).then_some(karaoke_video),
        }));
    }
    tracks.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Json(serde_json::json!({ "tracks": tracks }))
}

#[derive(Deserialize)]
pub(crate) struct FileQuery {
    name: String,
    /// `karaoke` serves from the `.karaoke/` subdir; absent = the library root.
    #[serde(default)]
    dir: Option<String>,
}

/// Per-device sync ledger (`~/.linggen/dj-sync.json`): which library files each
/// paired device has fetched. Written on every `/api/dj/file` hit from a paired
/// device; the DJ skill reads it back via `/api/dj/devices` to show true
/// per-phone coverage. Keyed by `PairedDevice.id`, so revoking a device orphans
/// (not corrupts) its row.
#[derive(serde::Serialize, Deserialize, Default, Clone)]
struct DeviceSync {
    files: Vec<String>,
    last_fetch: i64,
}

fn sync_path() -> PathBuf {
    crate::paths::linggen_home().join("dj-sync.json")
}

/// Serializes read-modify-write of the ledger; the phone syncs sequentially,
/// but nothing enforces that.
static SYNC_LOCK: Mutex<()> = Mutex::new(());

fn load_sync() -> HashMap<String, DeviceSync> {
    std::fs::read_to_string(sync_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// The same token sources the LAN gate accepts (header / bearer / cookie),
/// resolved to the paired device that owns the token. Loopback callers — the
/// Mac's own UI and skills — carry no token and resolve to None.
fn device_from_headers(headers: &HeaderMap) -> Option<super::pair::PairedDevice> {
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
        })?;
    super::pair::load_devices().into_iter().find(|d| d.secret == token)
}

fn record_fetch(device_id: &str, name: &str) {
    let _guard = SYNC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut sync = load_sync();
    let entry = sync.entry(device_id.to_string()).or_default();
    if !entry.files.iter().any(|f| f == name) {
        entry.files.push(name.to_string());
    }
    entry.last_fetch = chrono::Utc::now().timestamp();
    if let Ok(text) = serde_json::to_string_pretty(&sync) {
        let _ = std::fs::write(sync_path(), text);
    }
}

#[derive(Deserialize)]
pub(crate) struct HaveBody {
    files: Vec<String>,
}

/// POST /api/dj/have — the phone reports its full on-device inventory after a
/// sync, replacing its ledger row. Fetch-recording alone can't get there:
/// files synced before the ledger existed are never re-fetched, and files
/// deleted on the phone would stay marked as synced.
pub(crate) async fn post_have(headers: HeaderMap, Json(body): Json<HaveBody>) -> Response {
    let Some(device) = device_from_headers(&headers) else {
        return (StatusCode::UNAUTHORIZED, "paired devices only").into_response();
    };
    let _guard = SYNC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut sync = load_sync();
    let entry = sync.entry(device.id).or_default();
    entry.files = body.files;
    entry.last_fetch = chrono::Utc::now().timestamp();
    match serde_json::to_string_pretty(&sync).map(|t| std::fs::write(sync_path(), t)) {
        Ok(Ok(())) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "persist failed").into_response(),
    }
}

/// GET /api/dj/devices — paired devices joined with their sync ledger. The DJ
/// skill compares `files` against the library to render per-phone coverage.
pub(crate) async fn get_devices() -> impl IntoResponse {
    let sync = load_sync();
    let devices: Vec<serde_json::Value> = super::pair::load_devices()
        .iter()
        .map(|d| {
            let s = sync.get(&d.id).cloned().unwrap_or_default();
            serde_json::json!({
                "id": d.id,
                "name": d.name,
                "files": s.files,
                "last_fetch": (s.last_fetch > 0).then_some(s.last_fetch),
            })
        })
        .collect();
    Json(serde_json::json!({ "devices": devices }))
}

/// GET /api/dj/file?name=…[&dir=karaoke] — serve one file. Plain file names
/// only (anything path-like is rejected); `dir=karaoke` picks the `.karaoke/`
/// subdir. The subdir is chosen by the param, never by a path inside `name`.
pub(crate) async fn get_file(headers: HeaderMap, Query(q): Query<FileQuery>) -> Response {
    if q.name.contains('/') || q.name.contains('\\') || q.name.starts_with('.') {
        return (StatusCode::BAD_REQUEST, "bad name").into_response();
    }
    let base = match q.dir.as_deref() {
        None => dj_dir(),
        Some("karaoke") => karaoke_dir(),
        Some(_) => return (StatusCode::BAD_REQUEST, "bad dir").into_response(),
    };
    let path = base.join(&q.name);
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            // Only library-root fetches count toward per-phone coverage; karaoke
            // sources are extras, not part of the tracked library set.
            if q.dir.is_none() {
                if let Some(device) = device_from_headers(&headers) {
                    record_fetch(&device.id, &q.name);
                }
            }
            let mime = match q.name.rsplit_once('.').map(|(_, e)| e.to_lowercase()) {
                Some(e) if e == "mp3" => "audio/mpeg",
                Some(e) if e == "m4a" || e == "aac" => "audio/mp4",
                Some(e) if e == "flac" => "audio/flac",
                Some(e) if e == "wav" => "audio/wav",
                Some(e) if e == "ogg" => "audio/ogg",
                Some(e) if e == "mp4" => "video/mp4",
                Some(e) if e == "lrc" => "text/plain; charset=utf-8",
                Some(e) if e == "webp" => "image/webp",
                Some(e) if e == "jpg" || e == "jpeg" => "image/jpeg",
                Some(e) if e == "png" => "image/png",
                _ => "application/octet-stream",
            };
            ([(header::CONTENT_TYPE, mime)], bytes).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "no such track").into_response(),
    }
}
