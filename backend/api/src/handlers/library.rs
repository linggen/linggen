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

#[derive(Deserialize)]
pub struct DownloadSkillRequest {
    pub url: String,
    pub skill: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
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
                scan_dir(&path, packs, library_path);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                // Support markdown and script files
                let is_supported = matches!(
                    ext,
                    "md" | "py" | "js" | "ts" | "jsx" | "tsx" | "sh" | "bash" | "zsh"
                );

                if is_supported {
                    info!("DEBUG: found supported file {:?}", path);
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        // For markdown files, try to extract frontmatter metadata
                        // For script files, create basic metadata
                        let mut meta = if ext == "md" {
                            extract_all_meta_from_content(&content)
                                .unwrap_or_else(|| serde_json::json!({}))
                        } else {
                            serde_json::json!({})
                        };

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

                        if let Some(obj) = meta.as_object_mut() {
                            obj.insert("id".to_string(), serde_json::json!(rel_path));
                            obj.insert("read_only".to_string(), serde_json::json!(false));
                            obj.insert("file_type".to_string(), serde_json::json!(ext));

                            let filename = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("unknown")
                                .to_string();
                            obj.insert("filename".to_string(), serde_json::json!(filename));

                            if !obj.contains_key("name") {
                                // For script files, use filename with extension
                                let display_name = if ext == "md" {
                                    filename.clone()
                                } else {
                                    format!("{}.{}", filename, ext)
                                };
                                obj.insert("name".to_string(), serde_json::json!(display_name));
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
}

pub async fn create_folder(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateFolderRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if req.name.contains("..") || req.name.starts_with('/') {
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
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid folder name".to_string()));
    }

    // Resolve paths - assume user is renaming something they can see
    let library_root = &state.library_path;
    let old_path = library_root.join(&req.old_name);

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
    let folder_path = library_root.join(&folder_name);

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

    std::fs::write(&pack_path, &req.content).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save pack: {}", e),
        )
    })?;

    info!("Saved library pack: {} at {:?}", pack_id, pack_path);

    Ok(Json(serde_json::json!({ "success": true })))
}

pub async fn download_skill(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DownloadSkillRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {

    // Validate skill name
    if req.skill.contains("..") || req.skill.contains('/') || req.skill.contains('\\') {
        return Err((StatusCode::BAD_REQUEST, "Invalid skill name".to_string()));
    }

    // Parse GitHub URL
    let url = req.url.trim().trim_end_matches(".git").trim_end_matches('/');
    let (owner, repo) = parse_github_url(url)?;

    info!(
        "Downloading skill {} from {}/{} (ref: {})",
        req.skill, owner, repo, req.git_ref
    );

    // Download zipball from GitHub
    let zip_url = format!(
        "https://codeload.github.com/{}/{}/zip/{}",
        owner, repo, req.git_ref
    );

    let client = reqwest::Client::new();
    let response = client
        .get(&zip_url)
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to download from GitHub: {}", e),
            )
        })?;

    if !response.status().is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("GitHub returned status: {}", response.status()),
        ));
    }

    let bytes = response.bytes().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read download: {}", e),
        )
    })?;

    // Extract skill from zip
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to open zip archive: {}", e),
        )
    })?;

    // Find skill directory in zip
    let mut skill_root_in_zip = None;
    for i in 0..archive.len() {
        let file = archive.by_index(i).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read zip entry: {}", e),
            )
        })?;
        let name = file.name();

        // Look for SKILL.md inside a directory named skill_name
        if (name.ends_with("/SKILL.md") || name.ends_with("/skill.md"))
            && name.contains(&format!("/{}/", req.skill))
        {
            let path = std::path::Path::new(name);
            if let Some(parent) = path.parent() {
                skill_root_in_zip = Some(parent.to_path_buf());
                break;
            }
        }
    }

    let skill_root = skill_root_in_zip.ok_or((
        StatusCode::NOT_FOUND,
        format!(
            "Could not find skill '{}' in repository. Make sure it contains a SKILL.md file.",
            req.skill
        ),
    ))?;

    // Create target directory: library/skills/{skill_name}
    let target_dir = state
        .library_path
        .join("skills")
        .join(&req.skill);

    // Remove existing if present
    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to remove existing skill: {}", e),
            )
        })?;
    }

    std::fs::create_dir_all(&target_dir).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create skill directory: {}", e),
        )
    })?;

    // Extract files
    let skill_root_str = skill_root.to_str().unwrap();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read zip entry: {}", e),
            )
        })?;
        let name = file.name().to_string();

        if name.starts_with(skill_root_str) && !file.is_dir() {
            let rel_path = &name[skill_root_str.len()..].trim_start_matches('/');
            if rel_path.is_empty() {
                continue;
            }

            // Security check
            if rel_path.contains("..") || rel_path.starts_with('/') {
                continue;
            }

            let dest_path = target_dir.join(rel_path);
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to create directory: {}", e),
                    )
                })?;
            }

            let mut outfile = std::fs::File::create(&dest_path).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to create file: {}", e),
                )
            })?;
            std::io::copy(&mut file, &mut outfile).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to write file: {}", e),
                )
            })?;
        }
    }

    info!("Skill {} downloaded to {:?}", req.skill, target_dir);

    Ok(Json(serde_json::json!({
        "success": true,
        "skill": req.skill,
        "path": target_dir.to_string_lossy()
    })))
}

fn parse_github_url(url: &str) -> Result<(String, String), (StatusCode, String)> {
    let stripped = url.trim_start_matches("https://github.com/");
    let parts: Vec<&str> = stripped.split('/').collect();
    if parts.len() >= 2 {
        return Ok((parts[0].to_string(), parts[1].to_string()));
    }
    Err((
        StatusCode::BAD_REQUEST,
        format!("Could not parse GitHub repository from '{}'", url),
    ))
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
