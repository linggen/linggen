//! File upload handler for Uploads source type
//!
//! Accepts files via multipart form, extracts text, chunks, embeds, and stores in LanceDB.

use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    Json,
};
use ingestion::extract_text;
use linggen_core::Chunk;
use serde::Serialize;
use std::io::Write;
use std::sync::Arc;
use tempfile::NamedTempFile;
use uuid::Uuid;

use super::index::AppState;

#[derive(Serialize)]
pub struct UploadResponse {
    pub success: bool,
    pub source_id: String,
    pub filename: String,
    pub chunks_created: usize,
}

#[derive(Serialize)]
pub struct UploadError {
    pub error: String,
}

/// Upload a file to an Uploads source
///
/// Expects multipart form with:
/// - `source_id`: The uploads source ID
/// - `file`: The file to upload
pub async fn upload_file(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, (StatusCode, Json<UploadError>)> {
    let mut source_id: Option<String> = None;
    let mut file_data: Option<(String, Vec<u8>)> = None;

    // Parse multipart form
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(UploadError {
                error: format!("Failed to read multipart field: {}", e),
            }),
        )
    })? {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "source_id" => {
                source_id = Some(field.text().await.map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(UploadError {
                            error: format!("Failed to read source_id: {}", e),
                        }),
                    )
                })?);
            }
            "file" => {
                let filename = field
                    .file_name()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                let data = field.bytes().await.map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(UploadError {
                            error: format!("Failed to read file data: {}", e),
                        }),
                    )
                })?;
                file_data = Some((filename, data.to_vec()));
            }
            _ => {}
        }
    }

    // Validate required fields
    let source_id = source_id.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(UploadError {
                error: "Missing source_id field".to_string(),
            }),
        )
    })?;

    let (filename, data) = file_data.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(UploadError {
                error: "Missing file field".to_string(),
            }),
        )
    })?;

    // Verify source exists and is an Uploads type
    let source = state
        .metadata_store
        .get_source(&source_id)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(UploadError {
                    error: format!("Failed to get source: {}", e),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(UploadError {
                    error: format!("Source not found: {}", source_id),
                }),
            )
        })?;

    if !matches!(source.source_type, linggen_core::SourceType::Uploads) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(UploadError {
                error: "Source is not an Uploads type".to_string(),
            }),
        ));
    }

    tracing::info!(
        "Uploading file '{}' ({} bytes) to source '{}'",
        filename,
        data.len(),
        source_id
    );

    // Write to temp file so we can use extract_text
    let extension = std::path::Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("txt");

    let mut temp_file = NamedTempFile::with_suffix(&format!(".{}", extension)).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(UploadError {
                error: format!("Failed to create temp file: {}", e),
            }),
        )
    })?;

    temp_file.write_all(&data).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(UploadError {
                error: format!("Failed to write temp file: {}", e),
            }),
        )
    })?;

    // Extract text from file
    let content = extract_text(temp_file.path()).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(UploadError {
                error: format!(
                    "Failed to extract text from file '{}'. Unsupported format or empty file.",
                    filename
                ),
            }),
        )
    })?;

    if content.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(UploadError {
                error: "File contains no extractable text".to_string(),
            }),
        ));
    }

    tracing::info!("Extracted {} characters from '{}'", content.len(), filename);

    // Chunk the content
    let chunks_text = state.chunker.chunk(&content);
    tracing::info!("Created {} chunks", chunks_text.len());

    if chunks_text.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(UploadError {
                error: "File produced no chunks".to_string(),
            }),
        ));
    }

    // Generate embeddings
    let chunk_refs: Vec<&str> = chunks_text.iter().map(|s| s.as_str()).collect();

    let model_guard = state.embedding_model.read().await;
    let model = model_guard.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        Json(UploadError {
            error: "Embedding model is initializing. Please try again in a few seconds."
                .to_string(),
        }),
    ))?;

    let embeddings = model.embed_batch(&chunk_refs).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(UploadError {
                error: format!("Failed to generate embeddings: {}", e),
            }),
        )
    })?;

    // Create chunks for storage
    let chunks: Vec<Chunk> = chunks_text
        .iter()
        .zip(embeddings.iter())
        .map(|(text, embedding)| Chunk {
            id: Uuid::new_v4(),
            source_id: source_id.clone(),
            document_id: filename.clone(),
            content: text.clone(),
            embedding: Some(embedding.clone()),
            metadata: serde_json::json!({
                "file_path": filename,
                "uploaded": true,
            }),
        })
        .collect();

    let chunks_created = chunks.len();

    // If this filename already exists for this source, remove old chunks first
    let previous_chunks = state
        .vector_store
        .delete_document_from_source(&source_id, &filename)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(UploadError {
                    error: format!("Failed to delete existing chunks for '{}': {}", filename, e),
                }),
            )
        })?;

    // Store new chunks in LanceDB
    state.vector_store.add(chunks).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(UploadError {
                error: format!("Failed to store chunks: {}", e),
            }),
        )
    })?;

    tracing::info!(
        "Successfully uploaded '{}' to source '{}': {} chunks created",
        filename,
        source_id,
        chunks_created
    );

    // Update source stats
    if let Ok(Some(mut source)) = state.metadata_store.get_source(&source_id) {
        // Adjust chunk count: remove previous chunks for this file (if any), then add new ones
        let current_chunks = source.chunk_count.unwrap_or(0);
        let adjusted_chunks = current_chunks
            .saturating_sub(previous_chunks)
            .saturating_add(chunks_created);
        source.chunk_count = Some(adjusted_chunks);

        // Only increment file count if this is a brand new file for this source
        let current_files = source.file_count.unwrap_or(0);
        source.file_count = Some(if previous_chunks > 0 {
            current_files
        } else {
            current_files + 1
        });

        // For size, avoid double-counting on re-upload: only add size for brand new files
        let current_size = source.total_size_bytes.unwrap_or(0);
        source.total_size_bytes = Some(if previous_chunks > 0 {
            current_size
        } else {
            current_size + data.len()
        });

        let _ = state.metadata_store.update_source(&source);
    }

    Ok(Json(UploadResponse {
        success: true,
        source_id,
        filename,
        chunks_created,
    }))
}

// --- List and Delete files for uploads sources ---

use serde::Deserialize;

#[derive(Deserialize)]
pub struct ListFilesRequest {
    pub source_id: String,
}

#[derive(Serialize)]
pub struct FileInfo {
    pub filename: String,
    pub chunk_count: usize,
}

#[derive(Serialize)]
pub struct ListFilesResponse {
    pub source_id: String,
    pub files: Vec<FileInfo>,
}

/// List all uploaded files for a source
pub async fn list_uploaded_files(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ListFilesRequest>,
) -> Result<Json<ListFilesResponse>, (StatusCode, Json<UploadError>)> {
    // Verify source exists
    let _source = state
        .metadata_store
        .get_source(&req.source_id)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(UploadError {
                    error: format!("Failed to get source: {}", e),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(UploadError {
                    error: format!("Source not found: {}", req.source_id),
                }),
            )
        })?;

    // List documents from vector store
    let documents = state
        .vector_store
        .list_documents(&req.source_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(UploadError {
                    error: format!("Failed to list files: {}", e),
                }),
            )
        })?;

    let files: Vec<FileInfo> = documents
        .into_iter()
        .map(|doc| FileInfo {
            filename: doc.document_id,
            chunk_count: doc.chunk_count,
        })
        .collect();

    Ok(Json(ListFilesResponse {
        source_id: req.source_id,
        files,
    }))
}

#[derive(Deserialize)]
pub struct DeleteFileRequest {
    pub source_id: String,
    pub filename: String,
}

#[derive(Serialize)]
pub struct DeleteFileResponse {
    pub success: bool,
    pub source_id: String,
    pub filename: String,
    pub chunks_deleted: usize,
}

/// Delete a specific file and its chunks from an uploads source
pub async fn delete_uploaded_file(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DeleteFileRequest>,
) -> Result<Json<DeleteFileResponse>, (StatusCode, Json<UploadError>)> {
    // Verify source exists
    let _source = state
        .metadata_store
        .get_source(&req.source_id)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(UploadError {
                    error: format!("Failed to get source: {}", e),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(UploadError {
                    error: format!("Source not found: {}", req.source_id),
                }),
            )
        })?;

    tracing::info!(
        "Deleting file '{}' from source '{}'",
        req.filename,
        req.source_id
    );

    // Delete from vector store
    let chunks_deleted = state
        .vector_store
        .delete_document_from_source(&req.source_id, &req.filename)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(UploadError {
                    error: format!("Failed to delete file: {}", e),
                }),
            )
        })?;

    // Update source stats
    if let Ok(Some(mut source)) = state.metadata_store.get_source(&req.source_id) {
        source.chunk_count = Some(
            source
                .chunk_count
                .unwrap_or(0)
                .saturating_sub(chunks_deleted),
        );
        source.file_count = Some(source.file_count.unwrap_or(0).saturating_sub(1));
        let _ = state.metadata_store.update_source(&source);
    }

    tracing::info!(
        "Deleted {} chunks for file '{}' from source '{}'",
        chunks_deleted,
        req.filename,
        req.source_id
    );

    Ok(Json(DeleteFileResponse {
        success: true,
        source_id: req.source_id,
        filename: req.filename,
        chunks_deleted,
    }))
}
