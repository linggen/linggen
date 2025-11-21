use axum::{extract::State, http::StatusCode, Json};
use chrono::Utc;
use ingestion::{Ingestor, LocalIngestor};
use rememberme_core::{Chunk, IndexingJob, JobStatus};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

use super::index::AppState;

#[derive(Deserialize)]
pub struct IndexSourceRequest {
    pub source_id: String,
}

#[derive(Serialize)]
pub struct IndexSourceResponse {
    pub job_id: String,
    pub files_indexed: usize,
    pub chunks_created: usize,
}

pub async fn index_source(
    State(state): State<Arc<AppState>>,
    Json(req): Json<IndexSourceRequest>,
) -> Result<Json<IndexSourceResponse>, (StatusCode, String)> {
    // 1. Get source from metadata store
    let source = state
        .metadata_store
        .get_source(&req.source_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("Source not found: {}", req.source_id),
        ))?;

    // 2. Create job
    let job_id = Uuid::new_v4().to_string();
    let mut job = IndexingJob {
        id: job_id.clone(),
        source_id: source.id.clone(),
        source_name: source.name.clone(),
        source_type: source.source_type.clone(),
        status: JobStatus::Running,
        started_at: Utc::now().to_rfc3339(),
        finished_at: None,
        files_indexed: None,
        chunks_created: None,
        error: None,
    };

    state
        .metadata_store
        .create_job(&job)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(
        "Started job {} for source '{}' ({})",
        job_id,
        source.name,
        source.path
    );

    // 3. Validate path
    let path = PathBuf::from(&source.path);
    if !path.exists() {
        let error_msg = format!("Path does not exist: {}", source.path);
        let failed_job = IndexingJob {
            status: JobStatus::Failed,
            finished_at: Some(Utc::now().to_rfc3339()),
            error: Some(error_msg.clone()),
            ..job
        };
        let _ = state.metadata_store.update_job(&failed_job);
        return Err((StatusCode::BAD_REQUEST, error_msg));
    }

    // 4. Ingest documents
    let ingestor = LocalIngestor::new(path);
    tracing::info!("Starting ingestion for folder: {}", source.path);

    let documents = match ingestor.ingest().await {
        Ok(docs) => docs,
        Err(e) => {
            let error_msg = format!("Ingestion failed: {}", e);
            tracing::error!("{}", error_msg);
            let failed_job = IndexingJob {
                status: JobStatus::Failed,
                finished_at: Some(Utc::now().to_rfc3339()),
                error: Some(error_msg.clone()),
                ..job
            };
            let _ = state.metadata_store.update_job(&failed_job);
            return Err((StatusCode::INTERNAL_SERVER_ERROR, error_msg));
        }
    };

    let files_count = documents.len();
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

    // Batch configuration for LanceDB writes
    const BATCH_SIZE: usize = 50; // Write every 50 files
    let mut chunk_buffer: Vec<Chunk> = Vec::new();
    let mut last_write_time = std::time::Instant::now();

    // 5. Process each document with batched writes
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

        // Chunk
        let chunk_start = std::time::Instant::now();
        let chunks_text = state.chunker.chunk(&doc.content);
        let chunk_time = chunk_start.elapsed();
        tracing::debug!(
            "  Created {} chunks in {:.2}ms",
            chunks_text.len(),
            chunk_time.as_secs_f64() * 1000.0
        );

        // Embed
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
                continue;
            }
        };
        let embed_time = embed_start.elapsed();
        tracing::debug!(
            "  Generated {} embeddings in {:.2}ms ({:.2}ms per chunk)",
            embeddings.len(),
            embed_time.as_secs_f64() * 1000.0,
            embed_time.as_secs_f64() * 1000.0 / embeddings.len() as f64
        );

        // Create chunks
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

        // Update job progress in redb periodically (every 10 files)
        if (idx + 1) % 10 == 0 || idx == documents.len() - 1 {
            job.files_indexed = Some(successful_files);
            job.chunks_created = Some(total_chunks);
            if let Err(e) = state.metadata_store.update_job(&job) {
                tracing::warn!("Failed to update job progress: {}", e);
            }
        }
    }

    // 6. Update job as completed
    let completed_job = IndexingJob {
        status: JobStatus::Completed,
        finished_at: Some(Utc::now().to_rfc3339()),
        files_indexed: Some(successful_files),
        chunks_created: Some(total_chunks),
        ..job
    };

    state
        .metadata_store
        .update_job(&completed_job)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let embedding_size_mb = (total_chunks * 1536) as f64 / 1_048_576.0;
    let processed_size_mb = (successful_files as f64 / files_count as f64) * total_size_mb;
    let estimated_lancedb_size_mb = embedding_size_mb + processed_size_mb;

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
        processed_size_mb
    );

    Ok(Json(IndexSourceResponse {
        job_id,
        files_indexed: successful_files,
        chunks_created: total_chunks,
    }))
}
