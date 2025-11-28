use axum::{extract::State, http::StatusCode, Json};
use chrono::Utc;
use ingestion::{GitIngestor, Ingestor, LocalIngestor, WebIngestor};
use rememberme_core::{Chunk, IndexingJob, JobStatus, SourceConfig, SourceType};
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
    let job = IndexingJob {
        id: job_id.clone(),
        source_id: source.id.clone(),
        source_name: source.name.clone(),
        source_type: source.source_type.clone(),
        status: JobStatus::Pending,
        started_at: Utc::now().to_rfc3339(),
        finished_at: None,
        files_indexed: None,
        chunks_created: None,
        total_files: None,
        total_size_bytes: None,
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

    // 4. Spawn background task for ingestion and indexing
    let state_clone = state.clone();
    let job_id_clone = job_id.clone();
    let source_clone = source.clone();
    let initial_job = job.clone();

    tokio::spawn(async move {
        run_indexing_job(state_clone, job_id_clone, source_clone, initial_job).await;
    });

    Ok(Json(IndexSourceResponse {
        job_id,
        files_indexed: 0,
        chunks_created: 0,
    }))
}

async fn run_indexing_job(
    state: Arc<AppState>,
    job_id: String,
    source: SourceConfig,
    initial_job: IndexingJob,
) {
    let mut running_job = initial_job.clone();
    let source_path = source.path.clone();

    // 0. Acquire permit from JobManager (this blocks if queue is full)
    tracing::info!("Job {} waiting for execution permit...", job_id);
    let _permit = state.job_manager.acquire().await;
    tracing::info!("Job {} acquired permit, starting execution", job_id);

    // Update status to Running
    running_job.status = JobStatus::Running;
    running_job.started_at = Utc::now().to_rfc3339();
    let _ = state.metadata_store.update_job(&running_job);

    // 1. Ingest documents
    tracing::info!(
        "Starting ingestion for {} source: {}",
        match initial_job.source_type {
            SourceType::Local => "Local",
            SourceType::Git => "Git",
            SourceType::Web => "Web",
            SourceType::Uploads => "Uploads",
        },
        source_path
    );

    // Log patterns if any
    if !source.include_patterns.is_empty() {
        tracing::info!("Include patterns: {:?}", source.include_patterns);
    }
    if !source.exclude_patterns.is_empty() {
        tracing::info!("Exclude patterns: {:?}", source.exclude_patterns);
    }

    let ingestor: Box<dyn Ingestor> = match initial_job.source_type {
        SourceType::Local | SourceType::Uploads => Box::new(LocalIngestor::with_patterns(
            PathBuf::from(&source_path),
            &source.include_patterns,
            &source.exclude_patterns,
        )),
        SourceType::Git => {
            // Store repos in ./backend/data/repos/<source_id>
            let mut repo_path = std::env::current_dir().unwrap_or_default();
            repo_path.push("backend");
            repo_path.push("data");
            repo_path.push("repos");
            repo_path.push(&initial_job.source_id);

            Box::new(GitIngestor::new(source_path.clone(), repo_path))
        }
        SourceType::Web => {
            // Default max depth of 2 for now
            Box::new(WebIngestor::new(source_path.clone(), 2))
        }
    };

    let documents = match ingestor.ingest().await {
        Ok(docs) => docs,
        Err(e) => {
            let error_msg = format!("Ingestion failed: {}", e);
            tracing::error!("{}", error_msg);
            let failed_job = IndexingJob {
                status: JobStatus::Failed,
                finished_at: Some(Utc::now().to_rfc3339()),
                error: Some(error_msg.clone()),
                ..running_job
            };
            let _ = state.metadata_store.update_job(&failed_job);
            return;
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

    // Update job with totals
    running_job.total_files = Some(files_count);
    running_job.total_size_bytes = Some(total_size_bytes);
    let _ = state.metadata_store.update_job(&running_job);

    let mut total_chunks = 0;
    let mut successful_files = 0;
    let mut failed_files = 0;

    // Batch configuration
    const BATCH_SIZE: usize = 50;
    let mut chunk_buffer: Vec<Chunk> = Vec::new();
    let mut last_write_time = std::time::Instant::now();

    tracing::info!(
        "🔄 Starting to process {} files for job {}",
        files_count,
        job_id
    );

    // 2. Process documents
    for (idx, doc) in documents.iter().enumerate() {
        // Check cancellation
        if check_cancellation(
            &state,
            &job_id,
            &running_job,
            successful_files,
            total_chunks,
        )
        .is_some()
        {
            return;
        }

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

        // Check cancellation before embedding
        if check_cancellation(
            &state,
            &job_id,
            &running_job,
            successful_files,
            total_chunks,
        )
        .is_some()
        {
            return;
        }

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

        // Derive a stable document identifier for pattern matching.
        // Prefer the logical file path from metadata (used for glob patterns),
        // fall back to the source_url or the document's UUID if missing.
        let document_id: String = doc
            .metadata
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| doc.source_url.clone());

        // Create chunks – all chunks from the same file share the same document_id
        for (text, embedding) in chunks_text.iter().zip(embeddings.iter()) {
            chunk_buffer.push(Chunk {
                id: Uuid::new_v4(),
                source_id: running_job.source_id.clone(),
                document_id: document_id.clone(),
                content: text.clone(),
                embedding: Some(embedding.clone()),
                metadata: doc.metadata.clone(),
            });
        }

        total_chunks += chunks_text.len();
        successful_files += 1;

        // Batch write
        let should_write = chunk_buffer.len() >= BATCH_SIZE * 10
            || idx == documents.len() - 1
            || last_write_time.elapsed().as_secs() >= 5;

        if should_write && !chunk_buffer.is_empty() {
            // Check cancellation before DB write
            if check_cancellation(
                &state,
                &job_id,
                &running_job,
                successful_files,
                total_chunks,
            )
            .is_some()
            {
                return;
            }

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
                        "  ✓ Batch written in {:.2}ms",
                        write_time.as_secs_f64() * 1000.0
                    );
                    last_write_time = std::time::Instant::now();
                }
                Err(e) => {
                    tracing::error!("LanceDB batch write error: {} - Continuing...", e);
                }
            }
        }

        // Update progress
        if (idx + 1) % 10 == 0 || idx == documents.len() - 1 {
            running_job.files_indexed = Some(successful_files);
            running_job.chunks_created = Some(total_chunks);
            if let Err(e) = state.metadata_store.update_job(&running_job) {
                tracing::warn!("Failed to update job progress: {}", e);
            }
        }
    }

    // 3. Complete job
    let completed_job = IndexingJob {
        status: JobStatus::Completed,
        finished_at: Some(Utc::now().to_rfc3339()),
        files_indexed: Some(successful_files),
        chunks_created: Some(total_chunks),
        ..running_job
    };

    if let Err(e) = state.metadata_store.update_job(&completed_job) {
        tracing::error!("Failed to update job completion status: {}", e);
    }

    let embedding_size_mb = (total_chunks * 1536) as f64 / 1_048_576.0;
    let processed_size_mb = (successful_files as f64 / files_count as f64) * total_size_mb;
    let estimated_lancedb_size_mb = embedding_size_mb + processed_size_mb;

    tracing::info!(
        "  LanceDB size: ~{:.2} MB ({:.2} MB embeddings + {:.2} MB metadata)",
        estimated_lancedb_size_mb,
        embedding_size_mb,
        processed_size_mb
    );

    // 4. Auto-generate profile (skip for Uploads type - these are just document drops)
    if !matches!(initial_job.source_type, SourceType::Uploads) {
        tracing::info!(
            "🤖 Auto-generating profile for source {}...",
            initial_job.source_id
        );

        // Get LLM instance
        let llm = rememberme_llm::LLMSingleton::get().await;
        if let Some(llm) = llm {
            let profile_manager = rememberme_enhancement::ProfileManager::new(Some(llm));

            // Fetch chunks for profile generation (prioritize READMEs)
            // We can use the vector store to get chunks, or just use the ones we just created if we kept them.
            // But chunk_buffer was drained. Let's query the store to be safe and consistent.
            // We'll ask for READMEs first, then everything else if needed.

            let mut profile_chunks = Vec::new();

            // Try to get README chunks first
            match state
                .vector_store
                .get_chunks_by_file_pattern(&initial_job.source_id, "README*")
                .await
            {
                Ok(chunks) => profile_chunks.extend(chunks),
                Err(e) => tracing::warn!("Failed to fetch README chunks: {}", e),
            }

            // If we have very few chunks, maybe fetch more?
            // For now, let's just trust the ProfileManager's fallback logic if we pass what we have.
            // Actually, let's just fetch a reasonable sample if READMEs are missing.
            if profile_chunks.is_empty() {
                match state.vector_store.search(vec![0.0; 384], None, 50).await {
                    Ok(chunks) => {
                        // Filter for this source only (search returns global results potentially?)
                        // VectorStore::search doesn't filter by source_id in the current impl shown earlier.
                        // We should probably use get_chunks_by_file_pattern with "*"
                        tracing::info!("No READMEs found, trying to fetch all chunks...");
                    }
                    Err(_) => {}
                }

                match state
                    .vector_store
                    .get_chunks_by_file_pattern(&initial_job.source_id, "*")
                    .await
                {
                    Ok(chunks) => {
                        // Take first 50 to avoid overwhelming
                        profile_chunks.extend(chunks.into_iter().take(50));
                    }
                    Err(e) => tracing::warn!("Failed to fetch fallback chunks: {}", e),
                }
            }

            if !profile_chunks.is_empty() {
                match profile_manager
                    .generate_initial_profile(profile_chunks)
                    .await
                {
                    Ok(profile) => {
                        if let Err(e) = state
                            .metadata_store
                            .update_source_profile(&initial_job.source_id, &profile)
                        {
                            tracing::error!("Failed to save auto-generated profile: {}", e);
                        } else {
                            tracing::info!("✅ Auto-generated profile saved successfully!");
                        }
                    }
                    Err(e) => tracing::error!("Failed to generate profile: {}", e),
                }
            } else {
                tracing::warn!("No chunks available for profile generation");
            }
        } else {
            tracing::warn!("LLM not available, skipping auto-profile generation");
        }
    } else {
        tracing::info!("⏭️  Skipping profile generation for Uploads source");
    }

    // 5. Save stats to source config in Redb
    tracing::info!("💾 Saving stats to source config...");
    if let Ok(Some(mut source)) = state.metadata_store.get_source(&initial_job.source_id) {
        source.chunk_count = Some(total_chunks);
        source.file_count = Some(successful_files);
        source.total_size_bytes = completed_job.total_size_bytes;

        if let Err(e) = state.metadata_store.update_source(&source) {
            tracing::error!("Failed to save stats to source: {}", e);
        } else {
            tracing::info!("✅ Stats saved to source config!");
        }
    }
}

fn check_cancellation(
    state: &Arc<AppState>,
    job_id: &str,
    job: &IndexingJob,
    files_indexed: usize,
    chunks_created: usize,
) -> Option<()> {
    if state.cancellation_flags.get(job_id).map_or(false, |v| *v) {
        tracing::warn!("🛑 Job {} cancelled by user", job_id);

        let cancelled_job = IndexingJob {
            status: JobStatus::Failed,
            finished_at: Some(Utc::now().to_rfc3339()),
            files_indexed: Some(files_indexed),
            chunks_created: Some(chunks_created),
            error: Some("Job cancelled by user".to_string()),
            ..job.clone()
        };

        let _ = state.metadata_store.update_job(&cancelled_job);
        state.cancellation_flags.remove(job_id);
        Some(())
    } else {
        None
    }
}
