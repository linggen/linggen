use axum::{extract::State, http::StatusCode, Json};
use linggen_enhancement::{EnhancedPrompt, PromptEnhancer, PromptStrategy};
use serde::Deserialize;
use std::sync::Arc;

use super::index::AppState;

#[derive(Deserialize)]
pub struct EnhanceRequest {
    pub query: String,
    pub strategy: Option<PromptStrategy>,
    pub source_id: Option<String>,
    /// Optional: exclude results from a specific source (project)
    pub exclude_source_id: Option<String>,
}

/// Enhance a user prompt through the full 5-stage pipeline
pub async fn enhance_prompt(
    State(state): State<Arc<AppState>>,
    Json(req): Json<EnhanceRequest>,
) -> Result<Json<EnhancedPrompt>, (StatusCode, String)> {
    // Get user preferences
    let preferences = state.metadata_store.get_preferences().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load preferences: {}", e),
        )
    })?;

    // Get LLM instance if available
    let llm = linggen_llm::LLMSingleton::get().await;

    // Create enhancer
    let enhancer = PromptEnhancer::new(
        state.embedding_model.clone(),
        state.vector_store.clone(),
        llm,
    );

    // Get source profile if source_id is provided
    let profile = if let Some(source_id) = &req.source_id {
        state
            .metadata_store
            .get_source_profile(source_id)
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to load source profile: {}", e),
                )
            })?
    } else {
        // Use a default empty profile if no source is specified
        storage::SourceProfile::default()
    };

    // Determine strategy
    let strategy = req.strategy.unwrap_or(PromptStrategy::FullCode);

    // Run enhancement pipeline (intent detection is now handled by MCP)
    let result = enhancer
        .enhance(
            &req.query,
            &preferences,
            &profile,
            strategy,
            req.exclude_source_id.as_deref(),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Enhancement failed: {}", e),
            )
        })?;

    Ok(Json(result))
}
