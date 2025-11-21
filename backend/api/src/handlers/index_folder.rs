use axum::{extract::State, http::StatusCode, Json};
use ingestion::{Ingestor, LocalIngestor};
use rememberme_core::Chunk;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

use super::index::AppState;

#[derive(Deserialize)]
pub struct IndexFolderRequest {
    pub folder_path: String,
}

#[derive(Serialize)]
pub struct IndexFolderResponse {
    pub files_indexed: usize,
    pub chunks_created: usize,
    pub folder_path: String,
}

pub async fn index_folder(
    State(state): State<Arc<AppState>>,
    Json(req): Json<IndexFolderRequest>,
) -> Result<Json<IndexFolderResponse>, (StatusCode, String)> {
    // 1. Create LocalIngestor for the folder
    let path = PathBuf::from(&req.folder_path);
    if !path.exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Path does not exist: {}", req.folder_path),
        ));
    }
    if !path.is_dir() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Path is not a directory: {}", req.folder_path),
        ));
    }

    let ingestor = LocalIngestor::new(path);

    // 2. Ingest all documents from the folder
    tracing::info!("Starting ingestion for folder: {}", req.folder_path);
    let documents = ingestor.ingest().await.map_err(|e| {
        tracing::error!("Ingestion failed: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    let files_count = documents.len();

    // Calculate total size
    let total_size_bytes: usize = documents.iter().map(|d| d.content.len()).sum();
    let total_size_mb = total_size_bytes as f64 / 1_048_576.0;

    tracing::info!(
        "Ingested {} files ({:.2} MB total)",
        files_count,
        total_size_mb
    );

    let mut total_chunks = 0;
    let mut successful_files = 0;
    let mut failed_files = 0;
    let mut processed_size_bytes: usize = 0;

    // Batch configuration for LanceDB writes
    const BATCH_SIZE: usize = 50; // Write every 50 files
    let mut chunk_buffer: Vec<Chunk> = Vec::new();
    let mut last_write_time = std::time::Instant::now();

    // 3. Process each document: chunk, embed, and store (with batched writes)
    for (idx, doc) in documents.iter().enumerate() {
        let file_size_kb = doc.content.len() as f64 / 1024.0;
        let progress_pct = (idx + 1) as f64 / files_count as f64 * 100.0;

        tracing::info!(
            "[{:.1}%] Processing {}/{}: {} ({:.1} KB)",
            progress_pct,
            idx + 1,
            files_count,
            doc.source_url,
            file_size_kb
        );

        // Chunk the document content
        let chunk_start = std::time::Instant::now();
        let chunks_text = state.chunker.chunk(&doc.content);
        let chunk_time = chunk_start.elapsed();
        tracing::debug!(
            "  Created {} chunks in {:.2}ms",
            chunks_text.len(),
            chunk_time.as_secs_f64() * 1000.0
        );

        // Generate embeddings for all chunks
        let embed_start = std::time::Instant::now();
        let chunk_refs: Vec<&str> = chunks_text.iter().map(|s| s.as_str()).collect();
        let embeddings = match state.embedding_model.embed_batch(&chunk_refs) {
            Ok(emb) => emb,
            Err(e) => {
                tracing::error!(
                    "Embedding error for file '{}' (doc {}): {} - SKIPPING",
                    doc.source_url,
                    doc.id,
                    e
                );
                failed_files += 1;
                continue; // Skip this file and continue with next
            }
        };
        let embed_time = embed_start.elapsed();
        tracing::debug!(
            "  Generated {} embeddings in {:.2}ms ({:.2}ms per chunk)",
            embeddings.len(),
            embed_time.as_secs_f64() * 1000.0,
            embed_time.as_secs_f64() * 1000.0 / embeddings.len() as f64
        );

        // Create Chunk objects
        let mut chunks = Vec::new();
        for (text, embedding) in chunks_text.iter().zip(embeddings.iter()) {
            chunks.push(Chunk {
                id: Uuid::new_v4(),
                document_id: doc.id.clone(),
                content: text.clone(),
                embedding: Some(embedding.clone()),
                metadata: doc.metadata.clone(),
            });
        }

        total_chunks += chunks.len();
        successful_files += 1;
        processed_size_bytes += doc.content.len();

        // Add to buffer
        chunk_buffer.extend(chunks);

        // Batch write: write every BATCH_SIZE files or at the end
        let should_write = chunk_buffer.len() >= BATCH_SIZE * 10  // ~10 chunks per file avg
            || idx == documents.len() - 1  // Last file
            || last_write_time.elapsed().as_secs() >= 5; // Every 5 seconds

        if should_write && !chunk_buffer.is_empty() {
            let write_start = std::time::Instant::now();
            let chunks_to_write = chunk_buffer.len();

            tracing::info!(
                "  💾 Writing batch of {} chunks to LanceDB...",
                chunks_to_write
            );

            match state
                .vector_store
                .add(chunk_buffer.drain(..).collect())
                .await
            {
                Ok(_) => {
                    let write_time = write_start.elapsed();
                    tracing::info!(
                        "  ✓ Batch written in {:.2}ms ({:.2}ms per chunk)",
                        write_time.as_secs_f64() * 1000.0,
                        write_time.as_secs_f64() * 1000.0 / chunks_to_write as f64
                    );
                    last_write_time = std::time::Instant::now();
                }
                Err(e) => {
                    tracing::error!("LanceDB batch write error: {} - Continuing...", e);
                    // Don't fail the entire job, just log the error
                }
            }
        }
    }

    let processed_size_mb = processed_size_bytes as f64 / 1_048_576.0;

    // Calculate LanceDB storage size estimate
    // Each chunk has: id (16 bytes UUID) + content (variable) + embedding (384 * 4 = 1536 bytes)
    let embedding_size_mb = (total_chunks * 1536) as f64 / 1_048_576.0;
    let metadata_size_mb = processed_size_mb; // Approximate content size
    let estimated_lancedb_size_mb = embedding_size_mb + metadata_size_mb;

    tracing::info!("✅ Indexing complete!");
    tracing::info!(
        "  Files: {} successful, {} failed ({} total)",
        successful_files,
        failed_files,
        files_count
    );
    tracing::info!(
        "  Content: {:.2} MB processed, {} chunks created",
        processed_size_mb,
        total_chunks
    );
    tracing::info!(
        "  LanceDB size: ~{:.2} MB ({:.2} MB embeddings + {:.2} MB metadata)",
        estimated_lancedb_size_mb,
        embedding_size_mb,
        metadata_size_mb
    );

    Ok(Json(IndexFolderResponse {
        files_indexed: successful_files,
        chunks_created: total_chunks,
        folder_path: req.folder_path,
    }))
}
