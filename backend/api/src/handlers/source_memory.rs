use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info};

use super::AppState;

/// Information about a memory markdown file
#[derive(Debug, Serialize)]
pub struct MemoryFileInfo {
    pub path: String,
    pub name: String,
    pub modified_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListMemoryFilesResponse {
    pub files: Vec<MemoryFileInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryFileContent {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct SaveMemoryFileRequest {
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct RenameMemoryFileRequest {
    pub old_path: String,
    pub new_path: String,
}

fn get_memory_dir(source_path: &str) -> PathBuf {
    PathBuf::from(source_path).join(".linggen").join("memory")
}

fn try_extract_title_from_frontmatter(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    if !content.starts_with("---") {
        return None;
    }
    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        return None;
    }
    let frontmatter = parts[1];
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("title:") {
            let mut t = rest.trim().to_string();
            if (t.starts_with('\"') && t.ends_with('\"'))
                || (t.starts_with('\'') && t.ends_with('\''))
            {
                t = t[1..t.len() - 1].to_string();
            }
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

pub async fn list_memory_files(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
) -> Result<Json<ListMemoryFilesResponse>, (StatusCode, String)> {
    let source = state.metadata_store.get_source(&source_id).map_err(|e| {
        error!("Failed to get source {}: {}", source_id, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get source: {}", e),
        )
    })?;
    let source = source.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Source {} not found", source_id),
        )
    })?;

    let mem_dir = get_memory_dir(&source.path);
    if !mem_dir.exists() {
        return Ok(Json(ListMemoryFilesResponse { files: vec![] }));
    }

    let mut files = Vec::new();

    fn collect(
        dir: &PathBuf,
        base: &PathBuf,
        out: &mut Vec<MemoryFileInfo>,
    ) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect(&path, base, out)?;
            } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                let relative_path = path
                    .strip_prefix(base)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| path.file_name().unwrap().to_string_lossy().to_string());

                let name = try_extract_title_from_frontmatter(&path).unwrap_or_else(|| {
                    path.file_stem()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| relative_path.clone())
                });

                let modified_at = path.metadata().and_then(|m| m.modified()).ok().map(|t| {
                    let datetime: chrono::DateTime<chrono::Utc> = t.into();
                    datetime.to_rfc3339()
                });

                out.push(MemoryFileInfo {
                    path: relative_path,
                    name,
                    modified_at,
                });
            }
        }
        Ok(())
    }

    if let Err(e) = collect(&mem_dir, &mem_dir, &mut files) {
        error!("Failed to list memory files: {}", e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list memory files: {}", e),
        ));
    }

    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(ListMemoryFilesResponse { files }))
}

pub async fn get_memory_file(
    State(state): State<Arc<AppState>>,
    Path((source_id, file_path)): Path<(String, String)>,
) -> Result<Json<MemoryFileContent>, (StatusCode, String)> {
    let source = state.metadata_store.get_source(&source_id).map_err(|e| {
        error!("Failed to get source {}: {}", source_id, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get source: {}", e),
        )
    })?;
    let source = source.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Source {} not found", source_id),
        )
    })?;

    let mem_dir = get_memory_dir(&source.path);
    let file = mem_dir.join(&file_path);

    let canonical_base = mem_dir.canonicalize().unwrap_or(mem_dir.clone());
    if let Ok(canonical_file) = file.canonicalize() {
        if !canonical_file.starts_with(&canonical_base) {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid memory file path".to_string(),
            ));
        }
    }

    if !file.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Memory file {} not found", file_path),
        ));
    }

    let content = std::fs::read_to_string(&file).map_err(|e| {
        error!("Failed to read memory file {}: {}", file_path, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read memory file: {}", e),
        )
    })?;

    Ok(Json(MemoryFileContent {
        path: file_path,
        content,
    }))
}

pub async fn save_memory_file(
    State(state): State<Arc<AppState>>,
    Path((source_id, file_path)): Path<(String, String)>,
    Json(req): Json<SaveMemoryFileRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    info!("Saving memory file {} for source {}", file_path, source_id);
    let source = state.metadata_store.get_source(&source_id).map_err(|e| {
        error!("Failed to get source {}: {}", source_id, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get source: {}", e),
        )
    })?;
    let source = source.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Source {} not found", source_id),
        )
    })?;

    let mem_dir = get_memory_dir(&source.path);
    let file = mem_dir.join(&file_path);

    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            error!("Failed to create memory directory: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create directory: {}", e),
            )
        })?;
    }

    let canonical_base = mem_dir.canonicalize().unwrap_or(mem_dir.clone());
    if let Some(parent) = file.parent() {
        if let Ok(canonical_parent) = parent.canonicalize() {
            if !canonical_parent.starts_with(&canonical_base) {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Invalid memory file path".to_string(),
                ));
            }
        }
    }

    std::fs::write(&file, req.content).map_err(|e| {
        error!("Failed to write memory file {}: {}", file_path, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write memory file: {}", e),
        )
    })?;

    Ok(StatusCode::OK)
}

pub async fn delete_memory_file(
    State(state): State<Arc<AppState>>,
    Path((source_id, file_path)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    info!(
        "Deleting memory file {} for source {}",
        file_path, source_id
    );
    let source = state.metadata_store.get_source(&source_id).map_err(|e| {
        error!("Failed to get source {}: {}", source_id, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get source: {}", e),
        )
    })?;
    let source = source.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Source {} not found", source_id),
        )
    })?;

    let mem_dir = get_memory_dir(&source.path);
    let file = mem_dir.join(&file_path);

    let canonical_base = mem_dir.canonicalize().unwrap_or(mem_dir.clone());
    if let Ok(canonical_file) = file.canonicalize() {
        if !canonical_file.starts_with(&canonical_base) {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid memory file path".to_string(),
            ));
        }
    }

    if !file.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Memory file {} not found", file_path),
        ));
    }

    std::fs::remove_file(&file).map_err(|e| {
        error!("Failed to delete memory file {}: {}", file_path, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete memory file: {}", e),
        )
    })?;

    Ok(StatusCode::OK)
}

pub async fn rename_memory_file(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    Json(req): Json<RenameMemoryFileRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    info!(
        "Renaming memory file from {} to {} for source {}",
        req.old_path, req.new_path, source_id
    );
    let source = state.metadata_store.get_source(&source_id).map_err(|e| {
        error!("Failed to get source {}: {}", source_id, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get source: {}", e),
        )
    })?;
    let source = source.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Source {} not found", source_id),
        )
    })?;

    let mem_dir = get_memory_dir(&source.path);
    let old_file = mem_dir.join(&req.old_path);
    let new_file = mem_dir.join(&req.new_path);
    let canonical_base = mem_dir.canonicalize().unwrap_or(mem_dir.clone());

    if let Ok(canonical_old) = old_file.canonicalize() {
        if !canonical_old.starts_with(&canonical_base) {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid old memory path".to_string(),
            ));
        }
    }

    if let Some(parent) = new_file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            error!("Failed to create parent directory: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create directory: {}", e),
            )
        })?;
        let canonical_parent = parent.canonicalize().unwrap_or(parent.to_path_buf());
        if !canonical_parent.starts_with(&canonical_base) {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid new memory path".to_string(),
            ));
        }
    }

    if !old_file.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Old memory file {} not found", req.old_path),
        ));
    }

    if new_file.exists() {
        return Err((
            StatusCode::CONFLICT,
            format!("New memory path {} already exists", req.new_path),
        ));
    }

    std::fs::rename(&old_file, &new_file).map_err(|e| {
        error!("Failed to rename memory file: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to rename memory file: {}", e),
        )
    })?;

    Ok(StatusCode::OK)
}
