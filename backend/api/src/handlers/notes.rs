use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info};

use crate::AppState;

/// Information about a design note
#[derive(Debug, Serialize)]
pub struct NoteInfo {
    pub path: String,
    pub name: String,
    pub modified_at: Option<String>,
}

/// Response for listing notes
#[derive(Debug, Serialize)]
pub struct ListNotesResponse {
    pub notes: Vec<NoteInfo>,
}

/// Content of a note
#[derive(Debug, Serialize, Deserialize)]
pub struct NoteContent {
    pub path: String,
    pub content: String,
    pub linked_node: Option<String>,
}

/// Request to save a note
#[derive(Debug, Deserialize)]
pub struct SaveNoteRequest {
    pub content: String,
    pub linked_node: Option<String>,
}

/// Get the .linggen/notes directory for a source
fn get_notes_dir(source_path: &str) -> PathBuf {
    PathBuf::from(source_path).join(".linggen").join("notes")
}

/// List all design notes for a source
pub async fn list_notes(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
) -> Result<Json<ListNotesResponse>, (StatusCode, String)> {
    // Get the source to find its path
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

    let notes_dir = get_notes_dir(&source.path);

    if !notes_dir.exists() {
        return Ok(Json(ListNotesResponse { notes: vec![] }));
    }

    let mut notes = Vec::new();

    fn collect_notes(
        dir: &PathBuf,
        base: &PathBuf,
        notes: &mut Vec<NoteInfo>,
    ) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                collect_notes(&path, base, notes)?;
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

                notes.push(NoteInfo {
                    path: relative_path,
                    name,
                    modified_at,
                });
            }
        }
        Ok(())
    }

    if let Err(e) = collect_notes(&notes_dir, &notes_dir, &mut notes) {
        error!("Failed to list notes: {}", e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list notes: {}", e),
        ));
    }

    // Sort by name
    notes.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Json(ListNotesResponse { notes }))
}

/// Get a specific note's content
pub async fn get_note(
    State(state): State<Arc<AppState>>,
    Path((source_id, note_path)): Path<(String, String)>,
) -> Result<Json<NoteContent>, (StatusCode, String)> {
    // Get the source to find its path
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

    let notes_dir = get_notes_dir(&source.path);
    let note_file = notes_dir.join(&note_path);

    // Security check: ensure the path is within notes_dir
    let canonical_notes = notes_dir.canonicalize().unwrap_or(notes_dir.clone());
    if let Ok(canonical_note) = note_file.canonicalize() {
        if !canonical_note.starts_with(&canonical_notes) {
            return Err((StatusCode::BAD_REQUEST, "Invalid note path".to_string()));
        }
    }

    if !note_file.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Note {} not found", note_path),
        ));
    }

    let content = std::fs::read_to_string(&note_file).map_err(|e| {
        error!("Failed to read note {}: {}", note_path, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read note: {}", e),
        )
    })?;

    // Try to extract linked_node from frontmatter (simple parsing)
    let linked_node = extract_linked_node(&content);

    Ok(Json(NoteContent {
        path: note_path,
        content,
        linked_node,
    }))
}

/// Save a note
pub async fn save_note(
    State(state): State<Arc<AppState>>,
    Path((source_id, note_path)): Path<(String, String)>,
    Json(req): Json<SaveNoteRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    info!("Saving note {} for source {}", note_path, source_id);

    // Get the source to find its path
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

    let notes_dir = get_notes_dir(&source.path);
    let note_file = notes_dir.join(&note_path);

    // Create parent directories if needed
    if let Some(parent) = note_file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            error!("Failed to create notes directory: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create directory: {}", e),
            )
        })?;
    }

    // Security check after creating dirs
    let canonical_notes = notes_dir.canonicalize().unwrap_or(notes_dir.clone());
    if let Some(parent) = note_file.parent() {
        if let Ok(canonical_parent) = parent.canonicalize() {
            if !canonical_parent.starts_with(&canonical_notes) {
                return Err((StatusCode::BAD_REQUEST, "Invalid note path".to_string()));
            }
        }
    }

    // Prepare content with frontmatter if linked_node is provided
    let content = if let Some(ref linked_node) = req.linked_node {
        if !req.content.starts_with("---") {
            format!("---\nlinked_node: {}\n---\n\n{}", linked_node, req.content)
        } else {
            req.content.clone()
        }
    } else {
        req.content.clone()
    };

    std::fs::write(&note_file, content).map_err(|e| {
        error!("Failed to write note {}: {}", note_path, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write note: {}", e),
        )
    })?;

    info!("Note {} saved successfully", note_path);
    Ok(StatusCode::OK)
}

/// Delete a note
pub async fn delete_note(
    State(state): State<Arc<AppState>>,
    Path((source_id, note_path)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    info!("Deleting note {} for source {}", note_path, source_id);

    // Get the source to find its path
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

    let notes_dir = get_notes_dir(&source.path);
    let note_file = notes_dir.join(&note_path);

    // Security check
    let canonical_notes = notes_dir.canonicalize().unwrap_or(notes_dir.clone());
    if let Ok(canonical_note) = note_file.canonicalize() {
        if !canonical_note.starts_with(&canonical_notes) {
            return Err((StatusCode::BAD_REQUEST, "Invalid note path".to_string()));
        }
    }

    if !note_file.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Note {} not found", note_path),
        ));
    }

    std::fs::remove_file(&note_file).map_err(|e| {
        error!("Failed to delete note {}: {}", note_path, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete note: {}", e),
        )
    })?;

    info!("Note {} deleted successfully", note_path);
    Ok(StatusCode::OK)
}

/// Extract linked_node from YAML frontmatter
fn extract_linked_node(content: &str) -> Option<String> {
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
        if line.starts_with("linked_node:") {
            return Some(line.trim_start_matches("linked_node:").trim().to_string());
        }
    }

    None
}

/// Request to rename a note
#[derive(Debug, Deserialize)]
pub struct RenameNoteRequest {
    pub old_path: String,
    pub new_path: String,
}

/// Rename a note
pub async fn rename_note(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    Json(req): Json<RenameNoteRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    info!(
        "Renaming note from {} to {} for source {}",
        req.old_path, req.new_path, source_id
    );

    // Get the source to find its path
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

    let notes_dir = get_notes_dir(&source.path);
    let old_file = notes_dir.join(&req.old_path);
    let new_file = notes_dir.join(&req.new_path);

    // Security check
    let canonical_notes = notes_dir.canonicalize().unwrap_or(notes_dir.clone());

    // Check old file
    if let Ok(canonical_old) = old_file.canonicalize() {
        if !canonical_old.starts_with(&canonical_notes) {
            return Err((StatusCode::BAD_REQUEST, "Invalid old note path".to_string()));
        }
    }

    // Check new file parent
    if let Some(parent) = new_file.parent() {
        // Create parent if needed
        std::fs::create_dir_all(parent).map_err(|e| {
            error!("Failed to create parent directory for new path: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create directory: {}", e),
            )
        })?;

        let canonical_parent = parent.canonicalize().unwrap_or(parent.to_path_buf());
        if !canonical_parent.starts_with(&canonical_notes) {
            return Err((StatusCode::BAD_REQUEST, "Invalid new note path".to_string()));
        }
    }

    if !old_file.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Old note {} not found", req.old_path),
        ));
    }

    if new_file.exists() {
        let is_same_file = if let (Ok(old_canon), Ok(new_canon)) =
            (old_file.canonicalize(), new_file.canonicalize())
        {
            old_canon == new_canon
        } else {
            false
        };

        if !is_same_file {
            return Err((
                StatusCode::CONFLICT,
                format!("New note path {} already exists", req.new_path),
            ));
        }
    }

    std::fs::rename(&old_file, &new_file).map_err(|e| {
        error!("Failed to rename note: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to rename note: {}", e),
        )
    })?;

    info!(
        "Note renamed successfully from {} to {}",
        req.old_path, req.new_path
    );
    Ok(StatusCode::OK)
}
