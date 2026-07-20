//! DJ library sync for Linggen Mobile — the phone becomes DJ's native sync
//! target (replacing the old VLC push). Read-only over the paired channel:
//! list `~/Music/DJ`, then fetch tracks + `.lrc` lyric sidecars + covers by
//! name. Both routes sit behind the LAN gate like everything else.

use axum::{
    extract::Query,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::path::PathBuf;

const AUDIO_EXTS: &[&str] = &["mp3", "m4a", "flac", "wav", "ogg", "aac"];
const COVER_EXTS: &[&str] = &["webp", "jpg", "jpeg", "png"];

fn dj_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("Music").join("DJ")
}

fn is_ext(name: &str, exts: &[&str]) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, e)| exts.contains(&e.to_lowercase().as_str()))
}

/// GET /api/dj/library — every audio file with its sidecar availability.
pub(crate) async fn get_library() -> impl IntoResponse {
    let dir = dj_dir();
    let mut tracks = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        for name in names.iter().filter(|n| is_ext(n, AUDIO_EXTS)) {
            let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
            let lrc = format!("{stem}.lrc");
            let cover = COVER_EXTS
                .iter()
                .map(|e| format!("{stem}.{e}"))
                .find(|c| names.contains(c));
            let size = std::fs::metadata(dir.join(name)).map(|m| m.len()).unwrap_or(0);
            tracks.push(serde_json::json!({
                "name": name,
                "size": size,
                "lrc": names.contains(&lrc).then_some(lrc),
                "cover": cover,
            }));
        }
    }
    tracks.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Json(serde_json::json!({ "tracks": tracks }))
}

#[derive(Deserialize)]
pub(crate) struct FileQuery {
    name: String,
}

/// GET /api/dj/file?name=… — serve one library file (audio / .lrc / cover).
/// Plain file names only; anything path-like is rejected.
pub(crate) async fn get_file(Query(q): Query<FileQuery>) -> Response {
    if q.name.contains('/') || q.name.contains('\\') || q.name.starts_with('.') {
        return (StatusCode::BAD_REQUEST, "bad name").into_response();
    }
    let path = dj_dir().join(&q.name);
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mime = match q.name.rsplit_once('.').map(|(_, e)| e.to_lowercase()) {
                Some(e) if e == "mp3" => "audio/mpeg",
                Some(e) if e == "m4a" || e == "aac" => "audio/mp4",
                Some(e) if e == "flac" => "audio/flac",
                Some(e) if e == "wav" => "audio/wav",
                Some(e) if e == "ogg" => "audio/ogg",
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
