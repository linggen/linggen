use crate::job_manager::JobManager;
use axum::{extract::State, http::StatusCode, Json};
use dashmap::DashMap;
use embeddings::{EmbeddingModel, TextChunker};
use rememberme_core::Chunk;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use storage::{MetadataStore, VectorStore};
use uuid::Uuid;


pub struct AppState {
    pub embedding_model: Arc<EmbeddingModel>,
    pub chunker: Arc<TextChunker>,
    pub vector_store: Arc<VectorStore>,
    pub metadata_store: Arc<MetadataStore>,
    pub cancellation_flags: DashMap<String, bool>, // job_id -> is_cancelled
    pub job_manager: Arc<JobManager>,
}

