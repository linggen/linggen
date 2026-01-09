use ax_extract::extract_all_meta_from_content;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::info;

use crate::handlers::AppState;

#[derive(Deserialize)]
pub struct ApplyPackRequest {
    pub project_id: String,
}

#[derive(Deserialize)]
pub struct SavePackRequest {
    pub content: String,
}

#[derive(Deserialize)]
pub struct CreateFolderRequest {
    pub name: String,
}

#[derive(Deserialize)]
pub struct RenameFolderRequest {
    pub old_name: String,
    pub new_name: String,
}

#[derive(Deserialize)]
pub struct CreatePackRequest {
    pub folder: String,
    pub name: String,
}

#[derive(Deserialize)]
pub struct RenamePackRequest {
    pub pack_id: String,
    pub new_name: String,
}

pub async fn list_folders(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let library_root = &state.library_path;
    let mut folders: Vec<String> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(library_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    // Hide dot folders; optionally hide legacy "packs" folder.
                    if name.starts_with('.') || name == "packs" {
                        continue;
                    }
                    folders.push(name.to_string());
                }
            }
        }
    }

    folders.sort();
    folders.dedup();

    Json(serde_json::json!({ "folders": folders }))
}

pub async fn list_packs(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let library_root = &state.library_path;
    let mut packs = Vec::new();
    let mut seen_ids = HashSet::new();

    fn scan_dir(
        dir: &std::path::Path,
        root: &std::path::Path,
        packs: &mut Vec<serde_json::Value>,
        seen_ids: &mut HashSet<String>,
    ) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    scan_dir(&path, root, packs, seen_ids);
                } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Some(mut meta) = extract_all_meta_from_content(&content) {
                            if let Some(id) = meta.get("id").and_then(|v| v.as_str()) {
                                if seen_ids.contains(id) {
                                    continue;
                                }
                                seen_ids.insert(id.to_string());
                            }

                            if let Some(obj) = meta.as_object_mut() {
                                // Add file metadata
                                if let Ok(metadata) = std::fs::metadata(&path) {
                                    if let Ok(created) = metadata.created() {
                                        obj.insert(
                                            "created_at".to_string(),
                                            serde_json::json!(
                                                chrono::DateTime::<chrono::Utc>::from(created)
                                            ),
                                        );
                                    }
                                    if let Ok(modified) = metadata.modified() {
                                        obj.insert(
                                            "updated_at".to_string(),
                                            serde_json::json!(
                                                chrono::DateTime::<chrono::Utc>::from(modified)
                                            ),
                                        );
                                    }
                                }

                                // Add folder info to metadata if possible
                                if let Some(parent_name) = path
                                    .parent()
                                    .and_then(|p| p.file_name())
                                    .and_then(|n| n.to_str())
                                {
                                    // If the parent is the root itself, we don't add a folder name (or use "general")
                                    if path.parent() != Some(root) {
                                        obj.insert(
                                            "folder".to_string(),
                                            serde_json::json!(parent_name),
                                        );
                                    }
                                }
                            }
                            packs.push(meta);
                        }
                    }
                }
            }
        }
    }

    scan_dir(library_root, library_root, &mut packs, &mut seen_ids);

    Json(serde_json::json!({ "packs": packs }))
}

pub async fn create_folder(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateFolderRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if req.name.contains(std::path::MAIN_SEPARATOR) || req.name.contains("..") {
        return Err((StatusCode::BAD_REQUEST, "Invalid folder name".to_string()));
    }
    let folder_path = state.library_path.join(&req.name);
    if folder_path.exists() {
        return Err((StatusCode::CONFLICT, "Folder already exists".to_string()));
    }
    std::fs::create_dir_all(&folder_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create folder: {}", e),
        )
    })?;
    Ok(StatusCode::OK)
}

pub async fn rename_folder(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RenameFolderRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if req.old_name.contains(std::path::MAIN_SEPARATOR)
        || req.new_name.contains(std::path::MAIN_SEPARATOR)
        || req.old_name.contains("..")
        || req.new_name.contains("..")
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid folder name".to_string()));
    }
    let old_path = state.library_path.join(&req.old_name);
    let new_path = state.library_path.join(&req.new_name);

    if !old_path.exists() {
        return Err((StatusCode::NOT_FOUND, "Folder not found".to_string()));
    }
    if new_path.exists() {
        return Err((
            StatusCode::CONFLICT,
            "New folder name already exists".to_string(),
        ));
    }

    std::fs::rename(old_path, new_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to rename folder: {}", e),
        )
    })?;
    Ok(StatusCode::OK)
}

pub async fn delete_folder(
    State(state): State<Arc<AppState>>,
    Path(folder_name): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if folder_name.contains(std::path::MAIN_SEPARATOR) || folder_name.contains("..") {
        return Err((StatusCode::BAD_REQUEST, "Invalid folder name".to_string()));
    }
    let folder_path = state.library_path.join(&folder_name);
    if !folder_path.exists() {
        return Err((StatusCode::NOT_FOUND, "Folder not found".to_string()));
    }

    std::fs::remove_dir_all(folder_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete folder: {}", e),
        )
    })?;
    Ok(StatusCode::OK)
}

pub async fn create_pack(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreatePackRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if req.folder.contains(std::path::MAIN_SEPARATOR)
        || req.folder.contains("..")
        || req.name.contains(std::path::MAIN_SEPARATOR)
        || req.name.contains("..")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid folder or file name".to_string(),
        ));
    }
    let folder_path = state.library_path.join(&req.folder);
    if !folder_path.exists() {
        std::fs::create_dir_all(&folder_path).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create folder: {}", e),
            )
        })?;
    }

    let mut file_name = req.name.clone();
    if !file_name.ends_with(".md") {
        file_name.push_str(".md");
    }
    let file_path = folder_path.join(&file_name);

    if file_path.exists() {
        return Err((StatusCode::CONFLICT, "Pack already exists".to_string()));
    }

    let pack_id = req.name.to_lowercase().replace(' ', "-");
    let content = format!(
        "---\nid: {}\nname: {}\ndescription: New library pack\nscope: Personal\nversion: 1.0.0\nauthor: User\ntags: []\n---\n\n# {}\n\nStart writing...",
        pack_id, req.name, req.name
    );

    std::fs::write(&file_path, content).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create pack: {}", e),
        )
    })?;

    Ok(Json(
        serde_json::json!({ "id": pack_id, "path": file_path.to_string_lossy() }),
    ))
}

pub async fn rename_pack(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RenamePackRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if req.new_name.contains(std::path::MAIN_SEPARATOR) || req.new_name.contains("..") {
        return Err((StatusCode::BAD_REQUEST, "Invalid pack name".to_string()));
    }
    let library_root = &state.library_path;

    fn find_pack(dir: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(p) = find_pack(&path, id) {
                        return Some(p);
                    }
                } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Some(meta) = extract_all_meta_from_content(&content) {
                            if meta.get("id").and_then(|v| v.as_str()) == Some(id) {
                                return Some(path);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    let pack_path = find_pack(library_root, &req.pack_id)
        .ok_or((StatusCode::NOT_FOUND, "Pack not found".to_string()))?;

    let display_name = req
        .new_name
        .trim()
        .trim_end_matches(".md")
        .trim()
        .to_string();

    let mut new_file_name = display_name.clone();
    if !new_file_name.ends_with(".md") {
        new_file_name.push_str(".md");
    }
    let new_path = pack_path.parent().unwrap().join(&new_file_name);

    if new_path.exists() {
        return Err((
            StatusCode::CONFLICT,
            "New pack name already exists".to_string(),
        ));
    }

    let content = std::fs::read_to_string(&pack_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read pack: {}", e),
        )
    })?;

    fn update_frontmatter_name(content: &str, new_name: &str) -> String {
        if !content.starts_with("---") {
            return content.to_string();
        }

        let mut parts = content.splitn(3, "---");
        parts.next(); // skip empty before first ---
        let frontmatter = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("");

        let mut replaced = false;
        let mut out = Vec::new();
        for line in frontmatter.lines() {
            if !replaced && line.trim_start().starts_with("name:") {
                out.push(format!("name: {}", new_name));
                replaced = true;
            } else {
                out.push(line.to_string());
            }
        }
        if !replaced {
            out.push(format!("name: {}", new_name));
        }

        format!("---\n{}\n---{}", out.join("\n"), rest)
    }

    let updated = update_frontmatter_name(&content, &display_name);

    std::fs::rename(&pack_path, &new_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to rename pack: {}", e),
        )
    })?;

    std::fs::write(&new_path, updated).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to update pack content after rename: {}", e),
        )
    })?;

    Ok(StatusCode::OK)
}

pub async fn delete_pack(
    State(state): State<Arc<AppState>>,
    Path(pack_id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let library_root = &state.library_path;

    fn find_pack(dir: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(p) = find_pack(&path, id) {
                        return Some(p);
                    }
                } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Some(meta) = extract_all_meta_from_content(&content) {
                            if meta.get("id").and_then(|v| v.as_str()) == Some(id) {
                                return Some(path);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    let pack_path = find_pack(library_root, &pack_id)
        .ok_or((StatusCode::NOT_FOUND, "Pack not found".to_string()))?;

    std::fs::remove_file(pack_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete pack: {}", e),
        )
    })?;
    Ok(StatusCode::OK)
}

pub async fn apply_pack(
    State(state): State<Arc<AppState>>,
    Path(pack_id): Path<String>,
    Json(req): Json<ApplyPackRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // 1. Find the pack file
    let library_root = &state.library_path;

    fn find_pack(dir: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(p) = find_pack(&path, id) {
                        return Some(p);
                    }
                } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Some(meta) = extract_all_meta_from_content(&content) {
                            if meta.get("id").and_then(|v| v.as_str()) == Some(id) {
                                return Some(path);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    let pack_path = match find_pack(library_root, &pack_id) {
        Some(p) => p,
        None => return Err((StatusCode::NOT_FOUND, "Pack not found".to_string())),
    };

    // 2. Find the project
    let projects = state
        .metadata_store
        .get_sources()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let project = projects
        .into_iter()
        .find(|p| p.id == req.project_id)
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;

    // 3. Determine destination directory
    let project_root = std::path::PathBuf::from(&project.path);
    let linggen_dir = project_root.join(".linggen");

    // Check if the pack was in a folder like 'skills' or 'policies'
    let folder_name = pack_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("general");
    let dest_dir = linggen_dir.join(folder_name);

    if !dest_dir.exists() {
        std::fs::create_dir_all(&dest_dir).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create destination directory: {}", e),
            )
        })?;
    }

    let file_name = pack_path.file_name().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Invalid pack filename".to_string(),
    ))?;
    let dest_path = dest_dir.join(file_name);

    // 4. Copy the file
    std::fs::copy(&pack_path, &dest_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to copy pack: {}", e),
        )
    })?;

    info!(
        "Applied pack {} to project {} at {:?}",
        pack_id, project.name, dest_path
    );

    Ok(Json(serde_json::json!({
        "success": true,
        "destination": dest_path.to_string_lossy()
    })))
}

pub async fn get_pack(
    State(state): State<Arc<AppState>>,
    Path(pack_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let library_root = &state.library_path;

    fn find_pack(dir: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(p) = find_pack(&path, id) {
                        return Some(p);
                    }
                } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Some(meta) = extract_all_meta_from_content(&content) {
                            if meta.get("id").and_then(|v| v.as_str()) == Some(id) {
                                return Some(path);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    let pack_path = find_pack(library_root, &pack_id)
        .ok_or((StatusCode::NOT_FOUND, "Pack not found".to_string()))?;

    let content = std::fs::read_to_string(&pack_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read pack: {}", e),
        )
    })?;

    Ok(Json(serde_json::json!({
        "path": pack_path.to_string_lossy(),
        "content": content
    })))
}

pub async fn save_pack(
    State(state): State<Arc<AppState>>,
    Path(pack_id): Path<String>,
    Json(req): Json<SavePackRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let library_root = &state.library_path;

    fn find_pack(dir: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(p) = find_pack(&path, id) {
                        return Some(p);
                    }
                } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Some(meta) = extract_all_meta_from_content(&content) {
                            if meta.get("id").and_then(|v| v.as_str()) == Some(id) {
                                return Some(path);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    let pack_path = find_pack(library_root, &pack_id)
        .ok_or((StatusCode::NOT_FOUND, "Pack not found".to_string()))?;

    std::fs::write(&pack_path, &req.content).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save pack: {}", e),
        )
    })?;

    info!("Saved library pack: {} at {:?}", pack_id, pack_path);

    Ok(Json(serde_json::json!({ "success": true })))
}

mod ax_extract {
    pub fn extract_all_meta_from_content(content: &str) -> Option<serde_json::Value> {
        if !content.starts_with("---") {
            return None;
        }

        let mut parts = content.splitn(3, "---");
        parts.next(); // skip empty before first ---
        let frontmatter = parts.next()?;

        // Parse YAML into a generic serde_yaml::Value
        let yaml_val: serde_yaml::Value = serde_yaml::from_str(frontmatter).ok()?;

        // Convert YAML Value to JSON Value for storage in LanceDB/Metadata
        serde_json::to_value(yaml_val).ok()
    }
}
