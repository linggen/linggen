use axum::{extract::State, http::StatusCode, Json};
use embeddings::EmbeddingModel;
use rememberme_enhancement::{EnhancedPrompt, PromptEnhancer};
use rememberme_llm::{LLMConfig, MiniLLM};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use storage::VectorStore;

use super::index::AppState;

#[derive(Deserialize)]
pub struct EnhanceRequest {
    pub query: String,
}

#[derive(Serialize)]
pub struct EnhanceResponse {
    pub result: EnhancedPrompt,
}

/// Enhance a user prompt through the full 5-stage pipeline
pub async fn enhance_prompt(
    State(state): State<Arc<AppState>>,
    Json(req): Json<EnhanceRequest>,
) -> Result<Json<EnhanceResponse>, (StatusCode, String)> {
    // Initialize LLM
    let llm = MiniLLM::new(LLMConfig::default()).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to init LLM: {}", e),
        )
    })?;

    // Get user preferences
    let preferences = state.metadata_store.get_preferences().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load preferences: {}", e),
        )
    })?;

    // Create enhancer
    let mut enhancer = PromptEnhancer::new(
        Arc::new(llm),
        state.embedding_model.clone(),
        state.vector_store.clone(),
    );

    // Run enhancement pipeline
    let result = enhancer
        .enhance(&req.query, &preferences)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Enhancement failed: {}", e),
            )
        })?;

    Ok(Json(EnhanceResponse { result }))
}
