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
    // If a worker agent is currently blocked on a prompt, frame this turn so she
    // can relay the answer with `answer_prompt` rather than just chatting — the
    // user's reply and the open prompt land in the same turn (no cross-turn recall).
    let task = frame_with_pending_prompt(&state, &text).await;
    tokio::spawn(async move {
        if let Some(reply) = crate::server::yinyue_watch::run_yinyue_turn(&state, task).await {
            if !reply.eq_ignore_ascii_case("silent") {
                emit_speak(&state, reply, None);
            }
        }
    });
    (StatusCode::OK, "ok").into_response()
}

/// If another agent is parked on a prompt, wrap the user's message with that
/// context (question, options, `question_id`) so Yinyue can relay it with
/// `answer_prompt`. Plain message through when nothing is waiting.
async fn frame_with_pending_prompt(state: &Arc<ServerState>, text: &str) -> String {
    let pending = state.pending_ask_user.lock().await;
    let Some((qid, p)) = pending.iter().find(|(_, p)| p.agent_id != "yinyue") else {
        return text.to_string();
    };
    let q0 = p.questions.first();
    let question = q0.map(|q| q.question.as_str()).unwrap_or("their input");
    let options = q0
        .map(|q| {
            q.options
                .iter()
                .map(|o| o.label.clone())
                .collect::<Vec<_>>()
                .join(" / ")
        })
        .unwrap_or_default();
    format!(
        "[The agent \"{}\" is blocked, waiting on an answer to: \"{question}\" \
         (options: {options}; question_id \"{qid}\"). If the user's message below is their \
         answer, relay it with `answer_prompt` — only their actual words, never your own \
         decision. Otherwise just talk with them normally.]\n\nUser says: {text}",
        p.agent_id
    )
}

#[derive(Deserialize)]
pub(crate) struct PresenceBeat {
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub typing: bool,
    /// Milliseconds since the user's last input (key/pointer), measured client-side.
    #[serde(default)]
    pub idle_ms: u64,
}

/// POST /api/presence — a throttled liveness beat from a client surface. Carries
/// only recency + focus + a typing flag (never keystroke content); feeds the
/// `sense` tool so Yinyue can tell whether the user is here, reading, or away.
/// Generic (not Yinyue-specific) but lives here as the companion is its only
/// consumer today.
pub(crate) async fn presence_handler(
    State(state): State<Arc<ServerState>>,
    Json(beat): Json<PresenceBeat>,
) -> impl IntoResponse {
    state
        .manager
        .update_presence(beat.focused, beat.typing, beat.idle_ms);
    (StatusCode::OK, "ok").into_response()
}
