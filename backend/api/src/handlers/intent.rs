use axum::{http::StatusCode, Json};
use rememberme_intent::{IntentClassifier, IntentResult};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

#[derive(Deserialize)]
pub struct ClassifyRequest {
    pub query: String,
}

/// Classify developer query intent
pub async fn classify_intent(
    Json(req): Json<ClassifyRequest>,
) -> Result<Json<IntentResult>, (StatusCode, String)> {
    info!("Classifying intent for query: {}", req.query);

    // Get LLM instance if available
    let llm = rememberme_llm::LLMSingleton::get().await;

    // Use the intent classifier with optional LLM support
    let mut classifier = IntentClassifier::new(llm);
    let result = classifier.classify(&req.query).await.map_err(|e| {
        error!("Heuristic intent classification failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Classification failed: {}", e),
        )
    })?;

    info!("Intent classified successfully: {:?}", result.intent);
    Ok(Json(result))
}
