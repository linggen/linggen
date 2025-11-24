use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryInitResponse {
    pub success: bool,
    pub message: String,
}

/// Retry model initialization (clears error state and triggers re-download)
pub async fn retry_init(State(state): State<Arc<AppState>>) -> Json<RetryInitResponse> {
    // Clear error state
    if let Err(e) = state.metadata_store.set_setting("model_initialized", "") {
        return Json(RetryInitResponse {
            success: false,
            message: format!("Failed to clear error state: {}", e),
        });
    }

    if let Err(e) = state.metadata_store.set_setting("init_error", "") {
        return Json(RetryInitResponse {
            success: false,
            message: format!("Failed to clear error message: {}", e),
        });
    }

    if let Err(e) = state.metadata_store.set_setting("init_progress", "") {
        return Json(RetryInitResponse {
            success: false,
            message: format!("Failed to clear progress: {}", e),
        });
    }

    // Trigger re-initialization
    let metadata_store_clone = state.metadata_store.clone();
    tokio::spawn(async move {
        let progress_callback = |msg: &str| {
            tracing::info!("Model init progress: {}", msg);
            if let Err(e) = metadata_store_clone.set_setting("init_progress", msg) {
                tracing::info!("Failed to save progress: {}", e);
            }
        };

        // Use local model path
        let config = if let Some(home_dir) = dirs::home_dir() {
            let model_dir = home_dir.join(".rememberme/models/qwen2.5-1.5b");
            rememberme_llm::LLMConfig {
                model_path: Some(model_dir.join("model.safetensors")),
                tokenizer_path: Some(model_dir.join("tokenizer.json")),
                ..Default::default()
            }
        } else {
            rememberme_llm::LLMConfig::default()
        };

        match rememberme_llm::MiniLLM::new_with_progress(config, progress_callback) {
            Ok(_) => {
                tracing::info!("LLM model initialized successfully");
                if let Err(e) = metadata_store_clone.set_setting("model_initialized", "true") {
                    tracing::info!("Failed to save model initialization state: {}", e);
                }
                if let Err(e) = metadata_store_clone.set_setting("init_progress", "") {
                    tracing::info!("Failed to clear progress: {}", e);
                }
            }
            Err(e) => {
                let error_msg = format!("Failed to initialize LLM model: {}", e);
                tracing::info!("{}", error_msg);
                if let Err(e) = metadata_store_clone.set_setting("init_error", &error_msg) {
                    tracing::info!("Failed to save error: {}", e);
                }
                if let Err(e) = metadata_store_clone.set_setting("model_initialized", "error") {
                    tracing::info!("Failed to save model error state: {}", e);
                }
            }
        }
    });

    Json(RetryInitResponse {
        success: true,
        message: "Model initialization retry started".to_string(),
    })
}
