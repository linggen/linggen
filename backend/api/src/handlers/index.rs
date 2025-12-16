use crate::job_manager::JobManager;
use crate::memory::MemoryStore;
use dashmap::DashMap;
use embeddings::{EmbeddingModel, TextChunker};
use linggen_architect::GraphCache;
use std::sync::Arc;
use storage::{MetadataStore, VectorStore};

pub struct AppState {
    pub embedding_model: Arc<tokio::sync::RwLock<Option<EmbeddingModel>>>,
    pub chunker: Arc<TextChunker>,
    pub vector_store: Arc<VectorStore>,
    pub metadata_store: Arc<MetadataStore>,
    pub memory_store: Arc<MemoryStore>,
    pub cancellation_flags: DashMap<String, bool>, // job_id -> is_cancelled
    pub job_manager: Arc<JobManager>,
    pub graph_cache: Arc<GraphCache>,
}
