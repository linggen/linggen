//! Small daemon utilities: health, the user's name, a folder picker, the
//! local Ollama's loaded models.

use crate::server::ServerState;
use axum::{extract::State, response::IntoResponse};
use serde_json::json;
use std::sync::Arc;

/// GET /api/user/name — the user's name as core memory states it. Core is
/// the one authority for who the user is, so every chat surface labels the
/// user's bubbles from this answer. `name` is null until core genuinely
/// holds one — callers show nothing rather than a placeholder.
pub(crate) async fn get_user_name(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let ling_mem_url = state.manager.get_config_snapshot().await.agent.ling_mem_url;
    // The extractor blocks on its own bounded thread; keep the worker free.
    let name = tokio::task::spawn_blocking(move || {
        crate::engine::prompt::core_block::user_name_from_core(&ling_mem_url)
    })
    .await
    .ok()
    .flatten();
    axum::Json(serde_json::json!({ "name": name }))
}

pub(crate) async fn health_handler(
    axum::extract::State(state): axum::extract::State<Arc<ServerState>>,
) -> impl IntoResponse {
    // A health ping is a sign of life: bundled-app shells ping this every ~60s
    // while their window is open, which keeps the idle-shutdown clock fresh.
    state.last_activity.store(
        crate::server::unix_secs_now(),
        std::sync::atomic::Ordering::Relaxed,
    );
    axum::Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

pub(crate) async fn get_ollama_status(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let models_guard = state.manager.models.read().await;
    if let Some(client) = models_guard.first_ollama_client() {
        match client.get_ps().await {
            Ok(status) => axum::Json(status).into_response(),
            Err(e) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
            }
        }
    } else {
        (
            axum::http::StatusCode::NOT_FOUND,
            "No Ollama models configured",
        )
            .into_response()
    }
}
