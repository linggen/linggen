use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStatus {
    pub status: String, // "initializing", "ready", "error"
    pub message: Option<String>,
    pub progress: Option<String>, // e.g., "Downloading model weights (2/3)"
}

pub async fn get_app_status(State(state): State<Arc<AppState>>) -> Json<AppStatus> {
    // Check if model is initialized
    let model_initialized = state
        .metadata_store
        .get_setting("model_initialized")
        .unwrap_or(None)
        .map(|v| v == "true")
        .unwrap_or(false);

    // Check for error state
    let is_error = state
        .metadata_store
        .get_setting("model_initialized")
        .unwrap_or(None)
        .map(|v| v == "error")
        .unwrap_or(false);

    if is_error {
        // Get error message from redb (persisted)
        let error_msg = state
            .metadata_store
            .get_setting("init_error")
            .unwrap_or(None)
            .unwrap_or_else(|| "Model initialization failed".to_string());

        return Json(AppStatus {
            status: "error".to_string(),
            message: Some(error_msg),
            progress: None,
        });
    }

    if !model_initialized {
        // Get current progress from redb (persisted across refreshes)
        let progress = state
            .metadata_store
            .get_setting("init_progress")
            .unwrap_or(None);

        let message = progress
            .clone()
            .unwrap_or_else(|| "Initializing...".to_string());

        return Json(AppStatus {
            status: "initializing".to_string(),
            message: Some(message.clone()),
            progress,
        });
    }

    Json(AppStatus {
        status: "ready".to_string(),
        message: None,
        progress: None,
    })
}
