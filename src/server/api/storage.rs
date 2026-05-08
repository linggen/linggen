use crate::server::ServerState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Extensions treated as binary — refuse to serve their content.
const BINARY_EXTENSIONS: &[&str] = &[".redb", ".db", ".sqlite", ".sqlite3", ".bin", ".exe", ".dll", ".so", ".dylib"];

/// Dotfile directory names we auto-detect under the user's home.
const DOTFILE_DIRS: &[(&str, &str)] = &[
    (".linggen", "Linggen"),
    (".claude", "Claude"),
    (".codex", "Codex"),
];

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct StorageRoot {
    label: String,
    path: String,
}

#[derive(Serialize)]
struct StorageEntry {
    name: String,
    path: String,
    is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    children_count: Option<usize>,
}

#[derive(Serialize)]
struct TreeResponse {
    entries: Vec<StorageEntry>,
}

#[derive(Serialize)]
struct FileReadResponse {
    content: String,
    size: u64,
    modified: u64,
}

#[derive(Deserialize)]
pub(crate) struct TreeQuery {
    root: String,
    path: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct FileQuery {
    root: String,
    path: String,
}

#[derive(Deserialize)]
pub(crate) struct FileWriteBody {
    root: String,
    path: String,
    content: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the set of allowed root directories (those that actually exist).
fn allowed_roots() -> Vec<(String, PathBuf)> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    DOTFILE_DIRS
        .iter()
        .filter_map(|(dir, label)| {
            let p = home.join(dir);
            if p.is_dir() {
                Some((label.to_string(), p))
            } else {
                None
            }
        })
        .collect()
}

/// Validate that `root` is one of the allowed dotfile dirs and return its
/// canonical path. Returns `None` if invalid.
fn validate_root(root: &str) -> Option<PathBuf> {
    let root_path = PathBuf::from(root);
    let canonical = root_path.canonicalize().ok()?;
    let roots = allowed_roots();
    if roots.iter().any(|(_, p)| p.canonicalize().ok().as_ref() == Some(&canonical)) {
        Some(canonical)
    } else {
        None
    }
}

/// Resolve a relative `path` within `root`, rejecting traversal and symlink
/// escape. Returns the full canonical path on success.
fn safe_resolve(root: &Path, rel_path: &str) -> Result<PathBuf, StatusCode> {
    if rel_path.contains("..") {
        return Err(StatusCode::BAD_REQUEST);
    }
    let full = root.join(rel_path);
    // Canonicalize only if it already exists; for writes we allow new paths
    // but still verify prefix.
    let resolved = full.canonicalize().unwrap_or_else(|_| full.clone());
    if !resolved.starts_with(root) {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(resolved)
}

fn is_binary(path: &Path) -> bool {
    let s = path.to_string_lossy().to_ascii_lowercase();
    BINARY_EXTENSIONS.iter().any(|ext| s.ends_with(ext))
}

fn modified_secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/storage/roots` — list existing dotfile root directories.
pub(crate) async fn storage_roots(
    State(_state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    let roots: Vec<StorageRoot> = allowed_roots()
        .into_iter()
        .map(|(label, path)| StorageRoot {
            label,
            path: path.to_string_lossy().to_string(),
        })
        .collect();
    Json(roots).into_response()
}

/// `GET /api/storage/tree?root=...&path=...` — list directory entries.
pub(crate) async fn storage_tree(
    State(_state): State<Arc<ServerState>>,
    Query(query): Query<TreeQuery>,
) -> impl IntoResponse {
    let Some(root) = validate_root(&query.root) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let rel = query.path.as_deref().unwrap_or("");
    let dir = match safe_resolve(&root, rel) {
        Ok(p) => p,
        Err(status) => return status.into_response(),
    };
    if !dir.is_dir() {
        return StatusCode::NOT_FOUND.into_response();
    }

    let read_dir = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let mut entries: Vec<StorageEntry> = Vec::new();
    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let meta = entry.metadata().ok();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let entry_rel = if rel.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", rel, name)
        };

        if is_dir {
            let children_count = std::fs::read_dir(entry.path())
                .map(|rd| rd.count())
                .ok();
            entries.push(StorageEntry {
                name,
                path: entry_rel,
                is_dir: true,
                size: None,
                modified: meta.as_ref().map(modified_secs),
                children_count,
            });
        } else {
            entries.push(StorageEntry {
                name,
                path: entry_rel,
                is_dir: false,
                size: meta.as_ref().map(|m| m.len()),
                modified: meta.as_ref().map(modified_secs),
                children_count: None,
            });
        }
    }

    // Sort: dirs first, then alphabetical
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Json(TreeResponse { entries }).into_response()
}

/// `GET /api/storage/file?root=...&path=...` — read a file's content.
pub(crate) async fn storage_read_file(
    State(_state): State<Arc<ServerState>>,
    Query(query): Query<FileQuery>,
) -> impl IntoResponse {
    let Some(root) = validate_root(&query.root) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let full = match safe_resolve(&root, &query.path) {
        Ok(p) => p,
        Err(status) => return status.into_response(),
    };
    if full.is_dir() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if is_binary(&full) {
        return (StatusCode::UNSUPPORTED_MEDIA_TYPE, "Binary file").into_response();
    }

    let meta = match std::fs::metadata(&full) {
        Ok(m) => m,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let content = match std::fs::read_to_string(&full) {
        Ok(c) => c,
        Err(_) => return StatusCode::UNPROCESSABLE_ENTITY.into_response(),
    };

    Json(FileReadResponse {
        size: meta.len(),
        modified: modified_secs(&meta),
        content,
    })
    .into_response()
}

/// `PUT /api/storage/file` — write (create/update) a file.
pub(crate) async fn storage_write_file(
    State(_state): State<Arc<ServerState>>,
    Json(body): Json<FileWriteBody>,
) -> impl IntoResponse {
    let Some(root) = validate_root(&body.root) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let full = match safe_resolve(&root, &body.path) {
        Ok(p) => p,
        Err(status) => return status.into_response(),
    };
    if let Some(parent) = full.parent() {
        if let Err(_) = std::fs::create_dir_all(parent) {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    match std::fs::write(&full, &body.content) {
        Ok(_) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// `DELETE /api/storage/file?root=...&path=...` — delete a single file.
pub(crate) async fn storage_delete_file(
    State(_state): State<Arc<ServerState>>,
    Query(query): Query<FileQuery>,
) -> impl IntoResponse {
    let Some(root) = validate_root(&query.root) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let full = match safe_resolve(&root, &query.path) {
        Ok(p) => p,
        Err(status) => return status.into_response(),
    };
    if full.is_dir() {
        return (StatusCode::BAD_REQUEST, "Cannot delete directories").into_response();
    }
    match std::fs::remove_file(&full) {
        Ok(_) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
