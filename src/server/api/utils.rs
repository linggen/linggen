//! Small daemon utilities: health, the user's name, a folder picker, the
//! local Ollama's loaded models.

use crate::server::ServerState;
use axum::{extract::State, response::IntoResponse};
use serde_json::json;
use std::sync::Arc;

/// GET /api/user/name — the name to label the asker's own messages with.
/// For the owner it is the name core memory states: core is the one authority
/// for who the user is. A phone that is another person gets the name its
/// pairing record holds, never the owner's. `name` is null until one is
/// genuinely known — callers show nothing rather than a placeholder.
pub(crate) async fn get_user_name(
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    use crate::server::api::memory::{own_name_of, OwnName};
    let caller = crate::memory_host::caller::resolve(peer.ip(), &headers);
    match own_name_of(&caller) {
        OwnName::Core => {}
        OwnName::Held(name) => return axum::Json(json!({ "name": name })).into_response(),
        // Not "no name": a caller that keeps a name must not drop it on this.
        OwnName::NoOne => return axum::http::StatusCode::FORBIDDEN.into_response(),
    }
    // The read waits on memory's runtime from a blocking thread; keep the
    // worker free.
    let name = tokio::task::spawn_blocking(crate::engine::prompt::core_block::user_name_from_core)
        .await
        .ok()
        .flatten();
    axum::Json(json!({ "name": name })).into_response()
}

/// GET /api/user/language — the language Linggen writes in for the person.
/// `language` is the effective tag (the setting, else the Mac's, else "en");
/// `setting` is the explicit choice, null for Auto.
pub(crate) async fn get_user_language() -> impl IntoResponse {
    axum::Json(json!({
        "language": crate::language::effective(),
        "setting": crate::language::setting(),
    }))
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
    // With the contract numbers a phone checks this Mac against (`rtc::contract`).
    let mut body = crate::server::rtc::contract::numbers();
    body["ok"] = true.into();
    body["version"] = env!("CARGO_PKG_VERSION").into();
    body["exe"] = json!(crate::util::resolved_current_exe()
        .ok()
        .map(|p| p.to_string_lossy().into_owned()));
    body["commit"] = env!("LINGGEN_BUILD_COMMIT").into();
    axum::Json(body)
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
