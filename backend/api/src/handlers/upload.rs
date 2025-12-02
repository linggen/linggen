//! File upload handler for Uploads source type
//!
//! Accepts files via multipart form, extracts text, chunks, embeds, and stores in LanceDB.

use axum::{
    body::Body,
    extract::{Multipart, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures::stream::{self, StreamExt};
use ingestion::extract_text;
use linggen_core::Chunk;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::index::AppState;

#[derive(Serialize, Clone)]
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
    tracing::debug!("upload_file: Starting file upload request");

    let mut source_id: Option<String> = None;
    let mut file_data: Option<(String, Vec<u8>)> = None;

    // Parse multipart form
    tracing::debug!("upload_file: Parsing multipart form");
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

    // Create chunks for storage (include file_size in metadata for tracking)
    let file_size = data.len();
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
                "file_size": file_size,
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
    tracing::debug!("upload_file: Storing {} chunks in LanceDB", chunks_created);
    state.vector_store.add(chunks).await.map_err(|e| {
        tracing::error!("upload_file: Failed to store chunks: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(UploadError {
                error: format!("Failed to store chunks: {}", e),
            }),
        )
    })?;
    tracing::debug!("upload_file: Successfully stored chunks in LanceDB");

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

        // Get the previous file size (if re-uploading)
        let previous_file_size = source.file_sizes.get(&filename).copied().unwrap_or(0);

        // Only increment file count if this is a brand new file for this source
        let current_files = source.file_count.unwrap_or(0);
        source.file_count = Some(if previous_chunks > 0 {
            current_files
        } else {
            current_files + 1
        });

        // Update total size: subtract old file size, add new file size
        let current_size = source.total_size_bytes.unwrap_or(0);
        source.total_size_bytes = Some(
            current_size
                .saturating_sub(previous_file_size)
                .saturating_add(file_size),
        );

        // Track the file size in the file_sizes map
        source.file_sizes.insert(filename.clone(), file_size);

        let _ = state.metadata_store.update_source(&source);
    }

    Ok(Json(UploadResponse {
        success: true,
        source_id,
        filename,
        chunks_created,
    }))
}

// --- Streaming upload with progress updates ---

#[derive(Serialize, Clone)]
pub struct UploadProgress {
    pub phase: String,
    pub progress: u8,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<UploadResponse>,
}

/// Upload a file with streaming progress updates
/// Returns Server-Sent Events (SSE) style progress updates
pub async fn upload_file_stream(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Response {
    let (tx, mut rx) = mpsc::channel::<UploadProgress>(10);

    // Spawn the upload processing in background
    let state_clone = state.clone();
    tokio::spawn(async move {
        let result = process_upload_with_progress(state_clone, &mut multipart, tx.clone()).await;
        if let Err(e) = result {
            let _ = tx
                .send(UploadProgress {
                    phase: "error".to_string(),
                    progress: 0,
                    message: e.clone(),
                    error: Some(e),
                    result: None,
                })
                .await;
        }
    });

    // Create SSE stream from receiver
    let stream = stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some(progress) => {
                let json = serde_json::to_string(&progress).unwrap_or_default();
                let data = format!("data: {}\n\n", json);
                Some((Ok::<_, std::convert::Infallible>(data), rx))
            }
            None => None,
        }
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .header("Access-Control-Allow-Origin", "*")
        .body(Body::from_stream(stream))
        .unwrap()
}

async fn process_upload_with_progress(
    state: Arc<AppState>,
    multipart: &mut Multipart,
    tx: mpsc::Sender<UploadProgress>,
) -> Result<(), String> {
    // Send initial progress
    let _ = tx
        .send(UploadProgress {
            phase: "receiving".to_string(),
            progress: 5,
            message: "Receiving file...".to_string(),
            error: None,
            result: None,
        })
        .await;

    // Parse multipart form
    let mut source_id: Option<String> = None;
    let mut file_data: Option<(String, Vec<u8>)> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| e.to_string())? {
        let field_name = field.name().unwrap_or("").to_string();

        if field_name == "source_id" {
            source_id = Some(field.text().await.map_err(|e| e.to_string())?);
        } else if field_name == "file" {
            let filename = field.file_name().unwrap_or("unknown").to_string();
            let data = field.bytes().await.map_err(|e| e.to_string())?;
            file_data = Some((filename, data.to_vec()));
        }
    }

    let source_id = source_id.ok_or("Missing source_id field")?;
    let (filename, data) = file_data.ok_or("Missing file field")?;

    // Verify source exists
    let source = state
        .metadata_store
        .get_source(&source_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Source not found: {}", source_id))?;

    // Check source type
    if !matches!(source.source_type, linggen_core::SourceType::Uploads) {
        return Err(format!("Source '{}' is not an uploads type", source_id));
    }

    // Progress: Extracting text
    let _ = tx
        .send(UploadProgress {
            phase: "extracting".to_string(),
            progress: 15,
            message: "Extracting text from file...".to_string(),
            error: None,
            result: None,
        })
        .await;

    // Write to temp file and extract
    let extension = filename.split('.').last().unwrap_or("txt");
    let mut temp_file = NamedTempFile::with_suffix(&format!(".{}", extension))
        .map_err(|e| format!("Failed to create temp file: {}", e))?;
    temp_file
        .write_all(&data)
        .map_err(|e| format!("Failed to write temp file: {}", e))?;

    let content = extract_text(temp_file.path())
        .ok_or_else(|| format!("Failed to extract text from '{}'", filename))?;

    if content.trim().is_empty() {
        return Err("File contains no extractable text".to_string());
    }

    tracing::info!("Extracted {} characters from '{}'", content.len(), filename);

    // Progress: Chunking
    let _ = tx
        .send(UploadProgress {
            phase: "chunking".to_string(),
            progress: 30,
            message: "Splitting into chunks...".to_string(),
            error: None,
            result: None,
        })
        .await;

    let chunks_text = state.chunker.chunk(&content);
    tracing::info!("Created {} chunks", chunks_text.len());

    if chunks_text.is_empty() {
        return Err("File produced no chunks".to_string());
    }

    // Progress: Embedding (this is the slow part)
    let _ = tx
        .send(UploadProgress {
            phase: "embedding".to_string(),
            progress: 40,
            message: format!("Generating embeddings for {} chunks...", chunks_text.len()),
            error: None,
            result: None,
        })
        .await;

    let chunk_refs: Vec<&str> = chunks_text.iter().map(|s| s.as_str()).collect();

    let model_guard = state.embedding_model.read().await;
    let model = model_guard
        .as_ref()
        .ok_or("Embedding model is initializing. Please try again.")?;

    // For large documents, embed in batches and report progress
    let total_chunks = chunk_refs.len();
    let batch_size = 10.max(total_chunks / 5); // At least 5 progress updates
    let mut all_embeddings: Vec<Vec<f32>> = Vec::new();

    for (batch_idx, chunk_batch) in chunk_refs.chunks(batch_size).enumerate() {
        let batch_embeddings = model
            .embed_batch(chunk_batch)
            .map_err(|e| format!("Failed to generate embeddings: {}", e))?;
        all_embeddings.extend(batch_embeddings);

        // Calculate progress (40-85% is embedding phase)
        let processed = (batch_idx + 1) * batch_size;
        let progress =
            40 + ((processed.min(total_chunks) as f32 / total_chunks as f32) * 45.0) as u8;

        let _ = tx
            .send(UploadProgress {
                phase: "embedding".to_string(),
                progress,
                message: format!(
                    "Embedding chunks... {}/{}",
                    processed.min(total_chunks),
                    total_chunks
                ),
                error: None,
                result: None,
            })
            .await;
    }
    drop(model_guard);

    // Progress: Storing
    let _ = tx
        .send(UploadProgress {
            phase: "storing".to_string(),
            progress: 90,
            message: "Storing in database...".to_string(),
            error: None,
            result: None,
        })
        .await;

    // Create chunks
    let file_size = data.len();
    let chunks: Vec<Chunk> = chunks_text
        .iter()
        .zip(all_embeddings.iter())
        .map(|(text, embedding)| Chunk {
            id: Uuid::new_v4(),
            source_id: source_id.clone(),
            document_id: filename.clone(),
            content: text.clone(),
            embedding: Some(embedding.clone()),
            metadata: serde_json::json!({
                "file_path": filename,
                "uploaded": true,
                "file_size": file_size,
            }),
        })
        .collect();

    let chunks_created = chunks.len();

    // Delete existing chunks for this file (if re-uploading)
    let previous_chunks = state
        .vector_store
        .delete_document_from_source(&source_id, &filename)
        .await
        .map_err(|e| format!("Failed to delete existing chunks: {}", e))?;

    // Store new chunks
    state
        .vector_store
        .add(chunks)
        .await
        .map_err(|e| format!("Failed to store chunks: {}", e))?;

    // Update source stats
    if let Ok(Some(mut source)) = state.metadata_store.get_source(&source_id) {
        let current_chunks = source.chunk_count.unwrap_or(0);
        source.chunk_count = Some(
            current_chunks
                .saturating_sub(previous_chunks)
                .saturating_add(chunks_created),
        );

        let previous_file_size = source.file_sizes.get(&filename).copied().unwrap_or(0);
        let current_files = source.file_count.unwrap_or(0);
        source.file_count = Some(if previous_chunks > 0 {
            current_files
        } else {
            current_files + 1
        });

        let current_size = source.total_size_bytes.unwrap_or(0);
        source.total_size_bytes = Some(
            current_size
                .saturating_sub(previous_file_size)
                .saturating_add(file_size),
        );
        source.file_sizes.insert(filename.clone(), file_size);

        let _ = state.metadata_store.update_source(&source);
    }

    // Send completion
    let _ = tx
        .send(UploadProgress {
            phase: "complete".to_string(),
            progress: 100,
            message: "Upload complete!".to_string(),
            error: None,
            result: Some(UploadResponse {
                success: true,
                source_id,
                filename,
                chunks_created,
            }),
        })
        .await;

    Ok(())
}

// --- List and Delete files for uploads sources ---

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
    tracing::debug!(
        "list_uploaded_files: Listing files for source '{}'",
        req.source_id
    );

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

    tracing::debug!(
        "list_uploaded_files: Found {} files for source '{}'",
        files.len(),
        req.source_id
    );

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
        // Subtract chunk count
        source.chunk_count = Some(
            source
                .chunk_count
                .unwrap_or(0)
                .saturating_sub(chunks_deleted),
        );

        // Decrement file count
        source.file_count = Some(source.file_count.unwrap_or(0).saturating_sub(1));

        // Subtract file size from total and remove from tracking map
        if let Some(file_size) = source.file_sizes.remove(&req.filename) {
            source.total_size_bytes = Some(
                source
                    .total_size_bytes
                    .unwrap_or(0)
                    .saturating_sub(file_size),
            );
            tracing::debug!(
                "delete_uploaded_file: Subtracted {} bytes for '{}', new total: {:?}",
                file_size,
                req.filename,
                source.total_size_bytes
            );
        }

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
