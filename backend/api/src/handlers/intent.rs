use axum::{http::StatusCode, Json};
use rememberme_intent::{IntentClassifier, IntentResult};
use rememberme_llm::{LLMConfig, MiniLLM};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Deserialize)]
pub struct ClassifyRequest {
    pub query: String,
}

#[derive(Serialize)]
pub struct ClassifyResponse {
    pub intent: IntentResult,
}

/// Classify developer query intent
pub async fn classify_intent(
    Json(req): Json<ClassifyRequest>,
) -> Result<Json<ClassifyResponse>, (StatusCode, String)> {
    // Initialize LLM (will download model on first use)
    let llm = MiniLLM::new(LLMConfig::default()).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to init LLM: {}", e),
        )
    })?;

    // Create classifier
    let mut classifier = IntentClassifier::new(Arc::new(llm));

    // Classify
    let result = classifier.classify(&req.query).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Classification failed: {}", e),
        )
    })?;

    Ok(Json(ClassifyResponse { intent: result }))
}
