use ax_extract::extract_all_meta_from_content;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono;
use serde::Deserialize;
use std::sync::Arc;
use tracing::info;

use crate::handlers::AppState;

fn find_pack_by_id(root: &std::path::Path, target_id: &str) -> Option<std::path::PathBuf> {
    // The target_id is now the relative path from the library root
    let pack_path = root.join(target_id);

    // Security check: ensure the resolved path is still within the library root
    if let Ok(canonical_root) = root.canonicalize() {
        if let Ok(canonical_path) = pack_path.canonicalize() {
            if !canonical_path.starts_with(&canonical_root) {
                return None;
            }
        }
    }

    if pack_path.exists() && pack_path.is_file() {
        Some(pack_path)
    } else {
        None
    }
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

fn scan_folders_recursive(
    dir: &std::path::Path,
    root: &std::path::Path,
    prefix: &str,
    folders: &mut Vec<String>,
) {
    let root_can = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    info!(
        "DEBUG: scan_folders_recursive in {:?} (root_can: {:?})",
        dir, root_can
    );
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.') || name == "packs" {
                        continue;
                    }

                    // Prevent nested 'official' folders from appearing in the tree
                    if name == "official" {
                        let dir_can = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
                        let is_root_official = dir_can == root_can
                            || dir_can.to_string_lossy() == root_can.to_string_lossy();

                        if !is_root_official {
                            info!(
                                "DEBUG: skipping nested official folder in tree at {:?} (dir_can: {:?}, root_can: {:?})",
                                path, dir_can, root_can
                            );
                            continue;
                        }
                    }

                    let rel_path = match path.strip_prefix(&root_can) {
                        Ok(p) => p.to_string_lossy().to_string(),
                        Err(_) => {
                            // Try canonicalizing path too
                            let path_can = path.canonicalize().unwrap_or_else(|_| path.clone());
                            path_can
                                .strip_prefix(&root_can)
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_else(|_| name.to_string())
                        }
                    };

                    info!("DEBUG: adding folder: {}{}", prefix, rel_path);
                    folders.push(format!("{}{}", prefix, rel_path));
                    scan_folders_recursive(&path, root, prefix, folders);
                }
            }
        }
    }
}

pub async fn list_library(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let library_root = &state.library_path;
    let library_root_can = library_root
        .canonicalize()
        .unwrap_or_else(|_| library_root.to_path_buf());
    info!(
        "DEBUG: list_library scanning root: {:?} (can: {:?})",
        library_root, library_root_can
    );

    let mut folders = Vec::new();
    scan_folders_recursive(&library_root_can, &library_root_can, "", &mut folders);
    folders.sort();
    folders.dedup();

    let mut packs = Vec::new();
    scan_dir(&library_root_can, &mut packs, &library_root_can);

    Json(serde_json::json!({
        "folders": folders,
        "packs": packs
    }))
}

fn scan_dir(
    dir: &std::path::Path,
    packs: &mut Vec<serde_json::Value>,
    library_path: &std::path::Path,
) {
    let library_path_can = library_path
        .canonicalize()
        .unwrap_or_else(|_| library_path.to_path_buf());
    info!(
        "DEBUG: scan_dir in {:?} (library_path_can: {:?})",
        dir, library_path_can
    );
    if !dir.exists() {
        info!("DEBUG: dir does not exist: {:?}", dir);
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            info!("DEBUG: found entry {:?}", path);
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.') {
                        continue;
                    }
                    if name == "official" {
                        let dir_can = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
                        let is_root_official = dir_can == library_path_can
                            || dir_can.to_string_lossy() == library_path_can.to_string_lossy();

                        if !is_root_official {
                            info!("DEBUG: skipping nested official folder at {:?} (dir_can: {:?}, lib_can: {:?})", path, dir_can, library_path_can);
                            continue;
                        }
                    }
                }
                scan_dir(&path, packs, library_path);
            } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                info!("DEBUG: found markdown file {:?}", path);
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let mut meta = extract_all_meta_from_content(&content)
                        .unwrap_or_else(|| serde_json::json!({}));

                    // Use relative path from the library root as the ID
                    let rel_path = match path.strip_prefix(&library_path_can) {
                        Ok(p) => p.to_string_lossy().to_string(),
                        Err(_) => {
                            let path_can = path.canonicalize().unwrap_or_else(|_| path.clone());
                            path_can
                                .strip_prefix(&library_path_can)
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_else(|_| {
                                    path.file_name().unwrap().to_string_lossy().to_string()
                                })
                        }
                    };

                    let is_official = rel_path.starts_with("official/");

                    if let Some(obj) = meta.as_object_mut() {
                        obj.insert("id".to_string(), serde_json::json!(rel_path));
                        obj.insert("read_only".to_string(), serde_json::json!(is_official));

                        let filename = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("unknown")
                            .to_string();
                        obj.insert("filename".to_string(), serde_json::json!(filename));

                        if !obj.contains_key("name") {
                            obj.insert("name".to_string(), serde_json::json!(filename));
                        }

                        // Add file metadata
                        if let Ok(metadata) = std::fs::metadata(&path) {
                            if let Ok(created) = metadata.created() {
                                obj.insert(
                                    "created_at".to_string(),
                                    serde_json::json!(chrono::DateTime::<chrono::Utc>::from(
                                        created
                                    )),
                                );
                            }
                            if let Ok(modified) = metadata.modified() {
                                obj.insert(
                                    "updated_at".to_string(),
                                    serde_json::json!(chrono::DateTime::<chrono::Utc>::from(
                                        modified
                                    )),
                                );
                            }
                        }

                        // Add full relative folder path info
                        if let Some(parent) = path.parent() {
                            let rel_folder = match parent.strip_prefix(&library_path_can) {
                                Ok(p) => p.to_string_lossy().to_string(),
                                Err(_) => {
                                    let parent_can = parent
                                        .canonicalize()
                                        .unwrap_or_else(|_| parent.to_path_buf());
                                    parent_can
                                        .strip_prefix(&library_path_can)
                                        .map(|p| p.to_string_lossy().to_string())
                                        .unwrap_or_else(|_| "".to_string())
                                }
                            };

                            if !rel_folder.is_empty() {
                                obj.insert("folder".to_string(), serde_json::json!(rel_folder));
                            }
                        }
                    }
                    packs.push(meta);
                }
            }
        }
    }
}

pub async fn create_folder(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateFolderRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if req.name.contains("..")
        || req.name.starts_with('/')
        || req.name == "official"
        || req.name.starts_with("official/")
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid folder name".to_string()));
    }
    // Create folders directly in library root (user managed)
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
    if req.old_name.contains("..")
        || req.new_name.contains("..")
        || req.old_name.starts_with('/')
        || req.new_name.starts_with('/')
        || req.new_name == "official"
        || req.new_name.starts_with("official/")
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid folder name".to_string()));
    }

    // Resolve paths - assume user is renaming something they can see
    let library_root = &state.library_path;
    let old_path = if req.old_name.starts_with("official/") {
        return Err((
            StatusCode::FORBIDDEN,
            "Cannot rename official folders".to_string(),
        ));
    } else {
        library_root.join(&req.old_name)
    };

    // Determine new path - preserve parent directory of the old path
    let parent = old_path.parent().unwrap_or(library_root);
    let new_path = parent.join(&req.new_name);

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
    if folder_name.contains("..") || folder_name.starts_with('/') {
        return Err((StatusCode::BAD_REQUEST, "Invalid folder name".to_string()));
    }

    let library_root = &state.library_path;
    let folder_path = if folder_name.starts_with("official/") {
        return Err((
            StatusCode::FORBIDDEN,
            "Cannot delete official folders".to_string(),
        ));
    } else {
        library_root.join(&folder_name)
    };

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
    if req.folder.contains("..")
        || req.name.contains(std::path::MAIN_SEPARATOR)
        || req.name.contains("..")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid folder or file name".to_string(),
        ));
    }
    // Create pack in the specified folder relative to library root
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

    let content = format!(
        "---\nname: {}\ndescription: New library pack\nscope: Personal\nversion: 1.0.0\nauthor: User\ntags: []\n---\n\n# {}\n\nStart writing...",
        req.name, req.name
    );

    std::fs::write(&file_path, content).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create pack: {}", e),
        )
    })?;

    let rel_path = file_path
        .strip_prefix(&state.library_path)
        .unwrap_or(&file_path)
        .to_string_lossy()
        .to_string();

    Ok(Json(
        serde_json::json!({ "id": rel_path, "path": file_path.to_string_lossy() }),
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

    let pack_path = find_pack_by_id(library_root, &req.pack_id)
        .ok_or((StatusCode::NOT_FOUND, "Pack not found".to_string()))?;

    // If it's official, we don't allow renaming it directly (must save/edit first to move to user space)
    if pack_path.starts_with(library_root.join("official")) {
        return Err((
            StatusCode::FORBIDDEN,
            "Cannot rename official packs. Edit and save it first to create your own version."
                .to_string(),
        ));
    }

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
            let trimmed = line.trim_start();
            if trimmed.starts_with("id:") {
                // Skip ID line to remove it
                continue;
            }
            if !replaced && trimmed.starts_with("name:") {
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

    let pack_path = find_pack_by_id(library_root, &pack_id)
        .ok_or((StatusCode::NOT_FOUND, "Pack not found".to_string()))?;

    // Cannot delete official packs
    if pack_path.starts_with(library_root.join("official")) {
        return Err((
            StatusCode::FORBIDDEN,
            "Cannot delete official packs.".to_string(),
        ));
    }

    std::fs::remove_file(pack_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete pack: {}", e),
        )
    })?;
    Ok(StatusCode::OK)
}

pub async fn get_pack(
    State(state): State<Arc<AppState>>,
    Path(pack_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let library_root = &state.library_path;

    let pack_path = find_pack_by_id(library_root, &pack_id)
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

    let pack_path = find_pack_by_id(library_root, &pack_id)
        .ok_or((StatusCode::NOT_FOUND, "Pack not found".to_string()))?;

    // Check if the file is in official
    if pack_path.starts_with(library_root.join("official")) {
        return Err((
            StatusCode::FORBIDDEN,
            "Official templates are read-only. Please create your own pack to save changes."
                .to_string(),
        ));
    }

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
