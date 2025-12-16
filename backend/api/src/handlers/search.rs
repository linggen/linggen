use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::AppState;
use linggen_core::Chunk;

#[derive(Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub limit: Option<usize>,
    /// Optional: exclude results from a specific source/project ID
    pub exclude_source_id: Option<String>,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub results: Vec<Chunk>,
}

pub async fn search(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, (StatusCode, String)> {
    // 1. Embed the query
    let model_guard = state.embedding_model.read().await;
    let model = model_guard.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Embedding model is initializing. Please try again in a few seconds.".to_string(),
    ))?;

    let embedding = model
        .embed(&req.query)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 2. Search vector store
    let limit = req.limit.unwrap_or(10);
    let mut results = state
        .vector_store
        .search(embedding, Some(&req.query), limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Optional filtering: exclude chunks from a specific source
    if let Some(excluded) = req.exclude_source_id.as_deref() {
        results.retain(|c| c.source_id != excluded);
    }

    Ok(Json(SearchResponse { results }))
}
