//! Yinyue-specific HTTP endpoints.
//!
//! `POST /api/yinyue/say` pushes a "speak" cue onto the event bus; it fans out
//! to every connected surface (pet / menubar / web overlay) over the WebRTC
//! data channel. Surfaces render the bubble + expression and fetch the audio
//! from `/api/tts` — the cue is small, the blob is pulled. See the "Adaptive
//! presentation / One event spine" section of `doc/yinyue-spec.md`.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::server::{ServerEvent, ServerState};

#[derive(Deserialize)]
pub(crate) struct SayRequest {
    pub text: String,
    #[serde(default)]
    pub emotion: Option<String>,
}

/// Push a speak cue to all of the user's surfaces. The single producer — the
/// event-reactive watch loop calls this when Yinyue reacts, and the test
/// endpoint below calls it directly.
pub fn emit_speak(state: &Arc<ServerState>, text: String, emotion: Option<String>) {
    // Err only means no surface is currently connected — nothing to hear her.
    let _ = state.events_tx.send(ServerEvent::PetSpeak { text, emotion });
}

/// POST /api/yinyue/say — `{ text, emotion? }`. Trigger entry point for the
/// spine (manual + tests); the reaction path will call `emit_speak` directly.
pub(crate) async fn say_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<SayRequest>,
) -> impl IntoResponse {
    if req.text.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "empty text").into_response();
    }
    tracing::info!(
        "[yinyue] speak cue emotion={:?} ({} chars)",
        req.emotion,
        req.text.len()
    );
    emit_speak(&state, req.text, req.emotion);
    (StatusCode::OK, "ok").into_response()
}

/// POST /api/yinyue/chat — `{ text }`. The user talks to Yinyue directly (e.g.
/// clicking the desktop avatar). Her turn runs in the background and her reply
/// is spoken over the event spine; returns immediately so the UI isn't blocked.
pub(crate) async fn chat_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<SayRequest>,
) -> impl IntoResponse {
    let text = req.text.trim().to_string();
    if text.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty text").into_response();
    }
    tracing::info!("[yinyue] chat from user ({} chars)", text.len());
    tokio::spawn(async move {
        if let Some(reply) = crate::server::yinyue_watch::run_yinyue_turn(&state, text).await {
            if !reply.eq_ignore_ascii_case("silent") {
                emit_speak(&state, reply, None);
            }
        }
    });
    (StatusCode::OK, "ok").into_response()
}
