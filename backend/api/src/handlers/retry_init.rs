use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;

use super::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryInitResponse {
    pub success: bool,
    pub message: String,
}

/// Retry model initialization (clears error state and triggers re-download)
/// Only runs if llm_enabled is true in settings
pub async fn retry_init(State(state): State<Arc<AppState>>) -> Json<RetryInitResponse> {
    // Check if LLM is enabled
    let app_settings = state
        .metadata_store
        .get_app_settings()
        .unwrap_or_default();

    if !app_settings.llm_enabled {
        return Json(RetryInitResponse {
            success: false,
            message: "LLM is disabled in settings. Enable it first to initialize the model."
                .to_string(),
        });
    }

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

    // Trigger re-initialization using the same pattern as main.rs
    let metadata_store_clone = state.metadata_store.clone();
    tokio::spawn(async move {
        // Clone for the closure
        let metadata_store_for_progress = metadata_store_clone.clone();

        // Progress callback that saves to redb
        let progress_callback = move |msg: &str| {
            info!("Model init progress: {}", msg);
            if let Err(e) = metadata_store_for_progress.set_setting("init_progress", msg) {
                info!("Failed to save progress: {}", e);
            }
        };

        // Use default config to trigger download
        let config = rememberme_llm::LLMConfig::default();

        // Initialize LLM singleton
        match rememberme_llm::LLMSingleton::initialize_with_progress(config, progress_callback)
            .await
        {
            Ok(_) => {
                info!("LLM singleton initialized successfully");

                // Register model in ModelManager
                if let Ok(model_manager) = rememberme_llm::ModelManager::new() {
                    let _ = model_manager.register_model(
                        "qwen3-4b",
                        "Qwen3-4B-Instruct-2507",
                        "main",
                        std::collections::HashMap::new(),
                    );
                }

                // Mark as initialized in redb
                if let Err(e) = metadata_store_clone.set_setting("model_initialized", "true") {
                    info!("Failed to save model initialization state: {}", e);
                }
                if let Err(e) = metadata_store_clone.set_setting("init_progress", "") {
                    info!("Failed to clear progress: {}", e);
                }
            }
            Err(e) => {
                let error_msg = format!("Failed to initialize LLM model: {}", e);
                info!("{}", error_msg);
                if let Err(e) = metadata_store_clone.set_setting("init_error", &error_msg) {
                    info!("Failed to save error: {}", e);
                }
                if let Err(e) = metadata_store_clone.set_setting("model_initialized", "error") {
                    info!("Failed to save model error state: {}", e);
                }
            }
        }
    });

    Json(RetryInitResponse {
        success: true,
        message: "Model initialization started".to_string(),
    })
}
