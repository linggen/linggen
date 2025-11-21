use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::index::AppState;

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    10
}

#[derive(Serialize)]
pub struct SearchResult {
    pub document_id: String,
    pub content: String,
    pub score: f32,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub query: String,
}

pub async fn search(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, (StatusCode, String)> {
    // 1. Embed the query
    let query_embedding = state
        .embedding_model
        .embed(&query.q)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 2. Search in vector store
    let chunks = state
        .vector_store
        .search(query_embedding, Some(&query.q), query.limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 3. Convert to response (no scores available from current VectorStore implementation)
    let results = chunks
        .into_iter()
        .enumerate()
        .map(|(idx, chunk)| SearchResult {
            document_id: chunk.document_id,
            content: chunk.content,
            score: 1.0 / (idx as f32 + 1.0), // Placeholder score based on rank
        })
        .collect();

    Ok(Json(SearchResponse {
        results,
        query: query.q,
    }))
}
