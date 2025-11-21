use axum::{extract::State, http::StatusCode, Json};
use embeddings::{EmbeddingModel, TextChunker};
use rememberme_core::Chunk;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use storage::VectorStore;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct IndexRequest {
    pub document_id: String,
    pub content: String,
}

#[derive(Serialize)]
pub struct IndexResponse {
    pub chunks_indexed: usize,
    pub document_id: String,
}

pub struct AppState {
    pub embedding_model: Arc<EmbeddingModel>,
    pub chunker: Arc<TextChunker>,
    pub vector_store: Arc<VectorStore>,
}

pub async fn index_document(
    State(state): State<Arc<AppState>>,
    Json(req): Json<IndexRequest>,
) -> Result<Json<IndexResponse>, (StatusCode, String)> {
    // 1. Chunk the document
    let chunks_text = state.chunker.chunk(&req.content);

    // 2. Generate embeddings for all chunks
    let chunk_refs: Vec<&str> = chunks_text.iter().map(|s| s.as_str()).collect();
    let embeddings = state
        .embedding_model
        .embed_batch(&chunk_refs)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 3. Create Chunk objects
    let mut chunks = Vec::new();
    for (text, embedding) in chunks_text.iter().zip(embeddings.iter()) {
        chunks.push(Chunk {
            id: Uuid::new_v4(),
            document_id: req.document_id.clone(),
            content: text.clone(),
            embedding: Some(embedding.clone()),
            metadata: serde_json::json!({}),
        });
    }

    // 4. Store in LanceDB
    let chunks_count = chunks.len();
    state
        .vector_store
        .add(chunks)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(IndexResponse {
        chunks_indexed: chunks_count,
        document_id: req.document_id,
    }))
}
