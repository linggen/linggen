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

#[derive(Debug, Serialize)]
pub struct PromptInfo {
    pub path: String,
    pub name: String,
    pub modified_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListPromptsResponse {
    pub prompts: Vec<PromptInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PromptContent {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct SavePromptRequest {
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct RenamePromptRequest {
    pub old_path: String,
    pub new_path: String,
}

fn get_prompts_dir(source_path: &str) -> PathBuf {
    PathBuf::from(source_path).join(".linggen").join("prompts")
}

pub async fn list_prompts(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
) -> Result<Json<ListPromptsResponse>, (StatusCode, String)> {
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

    let dir = get_prompts_dir(&source.path);
    if !dir.exists() {
        return Ok(Json(ListPromptsResponse { prompts: vec![] }));
    }

    let mut prompts = Vec::new();

    fn collect(dir: &PathBuf, base: &PathBuf, out: &mut Vec<PromptInfo>) -> std::io::Result<()> {
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

                let name = path
                    .file_stem()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| relative_path.clone());

                let modified_at = path.metadata().and_then(|m| m.modified()).ok().map(|t| {
                    let datetime: chrono::DateTime<chrono::Utc> = t.into();
                    datetime.to_rfc3339()
                });

                out.push(PromptInfo {
                    path: relative_path,
                    name,
                    modified_at,
                });
            }
        }
        Ok(())
    }

    if let Err(e) = collect(&dir, &dir, &mut prompts) {
        error!("Failed to list prompts: {}", e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list prompts: {}", e),
        ));
    }

    prompts.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(ListPromptsResponse { prompts }))
}

pub async fn get_prompt(
    State(state): State<Arc<AppState>>,
    Path((source_id, prompt_path)): Path<(String, String)>,
) -> Result<Json<PromptContent>, (StatusCode, String)> {
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

    let dir = get_prompts_dir(&source.path);
    let file = dir.join(&prompt_path);

    let canonical_base = dir.canonicalize().unwrap_or(dir.clone());
    if let Ok(canonical_file) = file.canonicalize() {
        if !canonical_file.starts_with(&canonical_base) {
            return Err((StatusCode::BAD_REQUEST, "Invalid prompt path".to_string()));
        }
    }

    if !file.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Prompt {} not found", prompt_path),
        ));
    }

    let content = std::fs::read_to_string(&file).map_err(|e| {
        error!("Failed to read prompt {}: {}", prompt_path, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read prompt: {}", e),
        )
    })?;

    Ok(Json(PromptContent {
        path: prompt_path,
        content,
    }))
}

pub async fn save_prompt(
    State(state): State<Arc<AppState>>,
    Path((source_id, prompt_path)): Path<(String, String)>,
    Json(req): Json<SavePromptRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    info!("Saving prompt {} for source {}", prompt_path, source_id);
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

    let dir = get_prompts_dir(&source.path);
    let file = dir.join(&prompt_path);

    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            error!("Failed to create prompts directory: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create directory: {}", e),
            )
        })?;
    }

    let canonical_base = dir.canonicalize().unwrap_or(dir.clone());
    if let Some(parent) = file.parent() {
        if let Ok(canonical_parent) = parent.canonicalize() {
            if !canonical_parent.starts_with(&canonical_base) {
                return Err((StatusCode::BAD_REQUEST, "Invalid prompt path".to_string()));
            }
        }
    }

    std::fs::write(&file, &req.content).map_err(|e| {
        error!("Failed to write prompt {}: {}", prompt_path, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write prompt: {}", e),
        )
    })?;

    // Index the file in the internal index (async, don't block on errors)
    let internal_index_store = state.internal_index_store.clone();
    let embedding_model = state.embedding_model.clone();
    let chunker = state.chunker.clone();
    let source_id_clone = source_id.clone();
    let file_clone = file.clone();
    let prompt_path_with_kind = format!("prompts/{}", prompt_path);

    tokio::spawn(async move {
        if let Err(e) = crate::internal_indexer::index_internal_file(
            &internal_index_store,
            &embedding_model,
            &chunker,
            &source_id_clone,
            &file_clone,
            &prompt_path_with_kind,
        )
        .await
        {
            tracing::warn!("Failed to index prompt file in internal index: {}", e);
        }
    });

    Ok(StatusCode::OK)
}

pub async fn delete_prompt(
    State(state): State<Arc<AppState>>,
    Path((source_id, prompt_path)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    info!("Deleting prompt {} for source {}", prompt_path, source_id);
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

    let dir = get_prompts_dir(&source.path);
    let file = dir.join(&prompt_path);

    let canonical_base = dir.canonicalize().unwrap_or(dir.clone());
    if let Ok(canonical_file) = file.canonicalize() {
        if !canonical_file.starts_with(&canonical_base) {
            return Err((StatusCode::BAD_REQUEST, "Invalid prompt path".to_string()));
        }
    }

    if !file.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Prompt {} not found", prompt_path),
        ));
    }

    std::fs::remove_file(&file).map_err(|e| {
        error!("Failed to delete prompt {}: {}", prompt_path, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete prompt: {}", e),
        )
    })?;

    // Remove from internal index (async, don't block on errors)
    let internal_index_store = state.internal_index_store.clone();
    let source_id_clone = source_id.clone();
    let prompt_path_with_kind = format!("prompts/{}", prompt_path);

    tokio::spawn(async move {
        if let Err(e) = crate::internal_indexer::remove_internal_file(
            &internal_index_store,
            &source_id_clone,
            "prompt",
            &prompt_path_with_kind,
        )
        .await
        {
            tracing::warn!("Failed to remove prompt file from internal index: {}", e);
        }
    });

    Ok(StatusCode::OK)
}

pub async fn rename_prompt(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    Json(req): Json<RenamePromptRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    info!(
        "Renaming prompt from {} to {} for source {}",
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

    let dir = get_prompts_dir(&source.path);
    let old_file = dir.join(&req.old_path);
    let new_file = dir.join(&req.new_path);
    let canonical_base = dir.canonicalize().unwrap_or(dir.clone());

    if let Ok(canonical_old) = old_file.canonicalize() {
        if !canonical_old.starts_with(&canonical_base) {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid old prompt path".to_string(),
            ));
        }
    }

    if let Some(parent) = new_file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            error!("Failed to create parent directory for new path: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create directory: {}", e),
            )
        })?;
        let canonical_parent = parent.canonicalize().unwrap_or(parent.to_path_buf());
        if !canonical_parent.starts_with(&canonical_base) {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid new prompt path".to_string(),
            ));
        }
    }

    if !old_file.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Old prompt {} not found", req.old_path),
        ));
    }
    if new_file.exists() {
        return Err((
            StatusCode::CONFLICT,
            format!("New prompt path {} already exists", req.new_path),
        ));
    }

    std::fs::rename(&old_file, &new_file).map_err(|e| {
        error!("Failed to rename prompt: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to rename prompt: {}", e),
        )
    })?;
    Ok(StatusCode::OK)
}
