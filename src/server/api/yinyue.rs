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
    // Muted, the line still goes out — as words only.
    let voice = !state.manager.pet_muted();
    // Every line she says, from any path, restarts the quiet an app moment
    // waits for — a line an app had her say directly counts too.
    crate::server::yinyue_moments::note_spoke();
    // Err only means no surface is currently connected — nothing to hear her.
    let _ = state.events_tx.send(ServerEvent::PetSpeak {
        text,
        emotion,
        voice,
    });
}

/// `/mute` → turn her voice off, `/unmute` → back on; any other message is
/// not a voice command. Caught before any model runs, in every chat on this
/// machine.
pub(crate) fn voice_command(message: &str) -> Option<bool> {
    match message.trim().to_ascii_lowercase().as_str() {
        "/mute" => Some(true),
        "/unmute" => Some(false),
        _ => None,
    }
}

/// Carry out a voice command; returns the system's line for the chat.
pub(crate) async fn run_voice_command(state: &Arc<ServerState>, muted: bool) -> &'static str {
    match state.manager.set_pet_muted(muted).await {
        Ok(()) if muted => {
            "Yinyue's voice is off on this Mac. She still writes — /unmute brings it back."
        }
        Ok(()) => "Yinyue's voice is back on on this Mac.",
        Err(e) => {
            tracing::warn!("[yinyue] voice command failed: {e}");
            "Couldn't change Yinyue's voice — try again."
        }
    }
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

#[derive(Deserialize)]
pub(crate) struct EventRequest {
    pub app: String,
    pub text: String,
    #[serde(default)]
    pub big: bool,
    /// The user asked her for this (a reading): answered at once.
    #[serde(default)]
    pub asked: bool,
    #[serde(default)]
    pub mood: Option<String>,
}

const EVENT_TEXT_MAX: usize = 300;

/// POST /api/yinyue/event — `{ app, text, big?, asked?, mood? }`. An app tells Yinyue
/// what just happened, as a plain fact. Nothing is said now: the moment waits
/// until the user has gone quiet (or, `big`, until the screen settles), and
/// then she judges whether a word fits (server/yinyue_moments.rs).
pub(crate) async fn event_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<EventRequest>,
) -> impl IntoResponse {
    let app = req.app.trim();
    let text = req.text.trim();
    if app.is_empty() || app.len() > 40 || text.is_empty() {
        return (StatusCode::BAD_REQUEST, "app and text are required").into_response();
    }
    let text: String = text.chars().take(EVENT_TEXT_MAX).collect();
    // A mood is a word for her face; anything else is dropped, not refused.
    let mood = req
        .mood
        .filter(|m| !m.is_empty() && m.len() <= 16 && m.chars().all(|c| c.is_ascii_lowercase()));
    tracing::info!(
        "[yinyue] app moment from {app} ({} chars, big={})",
        text.len(),
        req.big
    );
    // Asked while the pet is off: nobody will answer — say so now, rather
    // than let the page wait on a silence.
    if req.asked && !state.manager.get_config_snapshot().await.pet.enabled {
        return (StatusCode::SERVICE_UNAVAILABLE, "unanswered: pet off").into_response();
    }
    crate::server::yinyue_moments::push(crate::server::yinyue_moments::Moment {
        app: app.to_string(),
        text,
        big: req.big,
        asked: req.asked,
        mood,
        at: crate::util::now_ts_secs(),
    });
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
    if let Some(muted) = voice_command(&text) {
        // The avatar has no chat to write a line into: the surface shows
        // the change from the voice event.
        run_voice_command(&state, muted).await;
        return (StatusCode::OK, "ok").into_response();
    }
    tracing::info!("[yinyue] chat from user ({} chars)", text.len());
    // If a worker agent is currently blocked on a prompt, frame this turn so she
    // can relay the answer with `answer_prompt` rather than just chatting — the
    // user's reply and the open prompt land in the same turn (no cross-turn recall).
    let task = frame_with_pending_prompt(&state, &text).await;
    tokio::spawn(async move {
        if let Some(reply) =
            crate::server::yinyue_watch::run_yinyue_turn(&state, task, "user").await
        {
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
    let Some((qid, p)) = pending
        .iter()
        .find(|(_, p)| p.agent_id != crate::engine::agent::COMPANION_AGENT_ID)
    else {
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
    /// The app (skill) this surface shows, if it is one — lets an app's
    /// moments wait until that app is the one in front.
    #[serde(default)]
    pub app: Option<String>,
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
    state.manager.update_presence(
        beat.focused,
        beat.typing,
        beat.idle_ms,
        beat.app.filter(|a| !a.is_empty() && a.len() <= 40),
    );
    (StatusCode::OK, "ok").into_response()
}

#[cfg(test)]
mod voice_tests {
    use super::voice_command;

    #[test]
    fn mute_and_unmute_are_commands_and_nothing_else_is() {
        assert_eq!(voice_command("/mute"), Some(true));
        assert_eq!(voice_command("  /UNMUTE \n"), Some(false));
        for other in [
            "mute",
            "/mute please",
            "/muted",
            "/unmute me",
            "please /mute",
            "",
        ] {
            assert_eq!(voice_command(other), None, "{other:?}");
        }
    }
}
