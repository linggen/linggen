use axum::{extract::State, http::StatusCode, Json};
use rememberme_enhancement::{EnhancedPrompt, PromptEnhancer, PromptStrategy};
use serde::Deserialize;
use std::sync::Arc;

use super::index::AppState;

#[derive(Deserialize)]
pub struct EnhanceRequest {
    pub query: String,
    pub strategy: Option<PromptStrategy>,
    pub source_id: Option<String>,
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
    let llm = rememberme_llm::LLMSingleton::get().await;

    // Create enhancer
    let mut enhancer = PromptEnhancer::new(
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

    // Read app settings to see if intent detection is enabled
    let intent_detection_enabled = state
        .metadata_store
        .get_app_settings()
        .map(|s| s.intent_detection_enabled)
        .unwrap_or(true);

    // Run enhancement pipeline
    let result = enhancer
        .enhance(
            &req.query,
            &preferences,
            &profile,
            strategy,
            intent_detection_enabled,
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
