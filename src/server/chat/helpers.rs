//! Chat-side helpers: message persistence + queue management + outcome
//! emission. Used by every `chat::*` flow plus a handful of API handlers
//! that need to persist or signal chat events.

use crate::engine::agent::AgentManager;
use crate::engine::AgentOutcome;
use crate::server::{ServerEvent, ServerState};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::broadcast;

// ---------------------------------------------------------------------------
// Turn-error formatting
// ---------------------------------------------------------------------------

/// Format a failed turn's error for the chat surface.
///
/// Always an `Error: …` string so the UI's `isError` path (which keys on the
/// `Error:` prefix) renders it reliably — that delivery path is proven, whereas
/// a structured-JSON message gets mangled by the chat ingest (strip/dedup/
/// block-parsing) and silently vanishes. `AUTH_REQUIRED:` errors keep their
/// marker in the text so the UI upgrades the banner into an inline "Sign in
/// with ChatGPT" CTA. Shared by every interactive turn path (main loop,
/// runtime wrapper, plan execution).
/// What the chat shows when a turn fails: one plain line for the person,
/// never the provider's text as if the agent had said it. The raw message
/// is in the log. AUTH_REQUIRED keeps its shape — the UI turns it into the
/// inline sign-in.
pub(crate) fn format_turn_error(msg: &str) -> String {
    if msg.contains("AUTH_REQUIRED") {
        return format!("Error: {}", msg);
    }
    // A spent Linggen Cloud allowance: the UI turns this into the subscribe
    // card, which only happens if the marker leads the line.
    if let Some(at) = msg.find("BILLING_REQUIRED:") {
        return format!("Error: {}", &msg[at..]);
    }
    // "Error:" is the prefix every surface reads as the error banner.
    let line = match model_error_code(msg) {
        "quota" => "The model has hit its limit for now — try again in a minute, or switch models.",
        "network" => "Couldn't reach the model — check the connection and try again.",
        "model_not_found" => "That model isn't available here — pick another in Settings → Models.",
        _ => "The model didn't answer this turn — try again.",
    };
    format!("Error: {line}")
}

/// Coarse telemetry bucket for a failed model turn. Buckets only — the
/// message text itself never leaves the machine (see telemetry/mod.rs).
pub(crate) fn model_error_code(msg: &str) -> &'static str {
    use crate::provider::error::{kind_of_message, ProviderErrorKind::*};
    let lower = msg.to_lowercase();
    if msg.contains("AUTH_REQUIRED") {
        return "auth_required";
    }
    if lower.contains("model") && lower.contains("not found") {
        return "model_not_found";
    }
    match kind_of_message(msg) {
        RateLimit | QuotaExhausted => "quota",
        Network | Server5xx => "network",
        Auth | BadRequest | ContextTooLong => "provider_http",
        EmptyResponse | Other if lower.contains("http") => "provider_http",
        EmptyResponse | Other => "other",
    }
}

// ---------------------------------------------------------------------------
// Message persistence
// ---------------------------------------------------------------------------

/// Emit a `ServerEvent::Message` **and** persist to session files. A run's
/// line (`run_id`: its error, its plan text) rides the agent-event queue,
/// behind the run's own events: sent straight, it overtook the run's queued
/// status, which then reopened the bubble the line had closed.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn persist_and_emit_message(
    manager: &Arc<AgentManager>,
    events_tx: &broadcast::Sender<ServerEvent>,
    root: &Path,
    agent_id: &str,
    from: &str,
    to: &str,
    content: &str,
    session_id: Option<&str>,
    is_observation: bool,
    run_id: Option<&str>,
) {
    if run_id.is_some() {
        let line = crate::engine::agent::AgentEvent::Message {
            from: from.to_string(),
            to: to.to_string(),
            content: content.to_string(),
            run_id: run_id.map(str::to_string),
            parent_id: None,
        };
        manager
            .send_event(line, session_id.map(str::to_string))
            .await;
    } else {
        let _ = events_tx.send(ServerEvent::Message {
            from: from.to_string(),
            to: to.to_string(),
            content: content.to_string(),
            session_id: session_id.map(|s| s.to_string()),
            run_id: None,
            parent_agent_id: None,
            client_id: None,
            via: None,
            via_note: None,
            images: Vec::new(),
        });
    }
    persist_message_only(
        manager,
        root,
        agent_id,
        from,
        to,
        content,
        session_id,
        is_observation,
        run_id,
    )
    .await;
}

/// Emit a `ServerEvent::Message` and persist directly to a `SessionStore`.
/// Used for mission sessions that live outside any project. `client_id`:
/// a person's message — the sending surface's id for it, kept and echoed.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn persist_and_emit_to_store(
    store: &crate::state_fs::SessionStore,
    events_tx: &broadcast::Sender<ServerEvent>,
    agent_id: &str,
    from: &str,
    to: &str,
    content: &str,
    session_id: Option<&str>,
    is_observation: bool,
    client_id: Option<&str>,
    via: Option<&str>,
    via_note: Option<&str>,
) {
    persist_and_emit_with_images(
        store,
        events_tx,
        agent_id,
        (from, to, content),
        session_id,
        is_observation,
        client_id,
        (via, via_note),
        &[],
    )
    .await
}

/// [`persist_and_emit_to_store`] for a person's message that carried
/// pictures (`sent`, base64 as the chat sent them): each is kept beside the
/// transcript and the row names them (`ChatMsg::images`), so every chat
/// surface draws them again after a reload.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn persist_and_emit_with_images(
    store: &crate::state_fs::SessionStore,
    events_tx: &broadcast::Sender<ServerEvent>,
    agent_id: &str,
    (from, to, content): (&str, &str, &str),
    session_id: Option<&str>,
    is_observation: bool,
    client_id: Option<&str>,
    (via, via_note): (Option<&str>, Option<&str>),
    sent: &[String],
) {
    let client_id = client_id.map(str::to_string);
    let sid = session_id.unwrap_or("default");
    // Kept first, so the live event names the same copies a reload shows.
    let kept = store.keep_images(sid, sent);
    let _ = events_tx.send(ServerEvent::Message {
        from: from.to_string(),
        to: to.to_string(),
        content: content.to_string(),
        session_id: session_id.map(|s| s.to_string()),
        run_id: None,
        parent_agent_id: None,
        client_id: client_id.clone(),
        via: via.map(str::to_string),
        via_note: via_note.map(str::to_string),
        images: kept
            .iter()
            .map(|n| crate::state_fs::session_images::url(sid, n))
            .collect(),
    });
    let msg = crate::state_fs::sessions::ChatMsg {
        run_id: None,
        agent_id: agent_id.to_string(),
        from_id: from.to_string(),
        to_id: to.to_string(),
        content: content.to_string(),
        timestamp: crate::util::now_ts_secs(),
        is_observation,
        client_id,
        failed: false,
        via: via.map(str::to_string),
        via_note: via_note.map(str::to_string),
        images: kept,
    };
    if let Err(e) = store.add_chat_message(sid, &msg) {
        tracing::warn!("Failed to persist chat message to mission store: {}", e);
    }
}

/// Persist to the global flat-file session store without emitting an SSE event.
pub(crate) async fn persist_message_only(
    manager: &Arc<AgentManager>,
    _root: &Path,
    agent_id: &str,
    from: &str,
    to: &str,
    content: &str,
    session_id: Option<&str>,
    is_observation: bool,
    run_id: Option<&str>,
) {
    let sid = session_id.unwrap_or("default");
    let msg = crate::state_fs::sessions::ChatMsg {
        run_id: run_id.map(str::to_string),
        via: None,
        via_note: None,
        agent_id: agent_id.to_string(),
        from_id: from.to_string(),
        to_id: to.to_string(),
        content: content.to_string(),
        timestamp: crate::util::now_ts_secs(),
        is_observation,
        client_id: None,
        failed: false,
        images: Vec::new(),
    };
    if let Err(e) = manager.global_sessions.add_chat_message(sid, &msg) {
        tracing::warn!("Failed to persist chat message: {}", e);
    }
}

// ---------------------------------------------------------------------------
// Queue management
// ---------------------------------------------------------------------------

pub(crate) fn queue_key(project_root: &str, session_id: &str, agent_id: &str) -> String {
    format!("{project_root}|{session_id}|{agent_id}")
}

pub(crate) fn queue_preview(message: &str) -> String {
    const LIMIT: usize = 100;
    let trimmed = message.trim();
    if trimmed.len() <= LIMIT {
        trimmed.to_string()
    } else {
        // Find a char boundary at or before LIMIT to avoid panic on multi-byte UTF-8.
        let end = trimmed
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= LIMIT)
            .last()
            .unwrap_or(0);
        format!("{}...", &trimmed[..end])
    }
}

pub(crate) async fn emit_queue_updated(
    state: &Arc<ServerState>,
    project_root: &str,
    session_id: &str,
    agent_id: &str,
) {
    let key = queue_key(project_root, session_id, agent_id);
    let items = {
        let guard = state.queued_chats.lock().await;
        guard.get(&key).cloned().unwrap_or_default()
    };
    let _ = state.events_tx.send(ServerEvent::QueueUpdated {
        project_root: project_root.to_string(),
        session_id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        items,
    });
}

// ---------------------------------------------------------------------------
// Outcome events
// ---------------------------------------------------------------------------

/// A run's outcome: its plan, when it made one, then `Outcome` — which ends
/// the run's bubble (a turn may hold two runs: a plan and its execution).
/// `Outcome` rides the agent-event queue, behind the run's own blocks and
/// message, like the turn's end (`runtime::close_turn`).
pub(crate) async fn emit_outcome_event(
    ctx: &super::ChatRunCtx,
    outcome: &AgentOutcome,
    run_id: Option<&str>,
) {
    let events_tx = &ctx.events_tx;
    let from_id = ctx.agent_id.as_str();
    let sid = ctx.session_id.clone();
    let run = run_id.map(str::to_string);
    match outcome {
        AgentOutcome::Plan(plan) => {
            let _ = events_tx.send(ServerEvent::Message {
                from: from_id.to_string(),
                to: "user".to_string(),
                content: serde_json::json!({
                    "type": "plan",
                    "plan": plan
                })
                .to_string(),
                session_id: sid.clone(),
                run_id: run.clone(),
                parent_agent_id: None,
                client_id: None,
                via: None,
                via_note: None,
                images: Vec::new(),
            });
        }
        AgentOutcome::PlanApproved(plan) => {
            let _ = events_tx.send(ServerEvent::Message {
                from: from_id.to_string(),
                to: "user".to_string(),
                content: serde_json::json!({
                    "type": "plan",
                    "plan": plan
                })
                .to_string(),
                session_id: sid.clone(),
                run_id: run.clone(),
                parent_agent_id: None,
                client_id: None,
                via: None,
                via_note: None,
                images: Vec::new(),
            });
            let _ = events_tx.send(ServerEvent::PlanUpdate {
                agent_id: from_id.to_string(),
                plan: plan.clone(),
                session_id: sid.clone(),
            });
        }
        _ => {}
    }
    // Always emit an Outcome event so the UI transitions the run from
    // RUNNING to completed and resets status to idle.
    ctx.manager
        .send_event(
            crate::engine::agent::AgentEvent::Outcome {
                agent_id: from_id.to_string(),
                outcome: outcome.clone(),
                run_id: run,
            },
            sid,
        )
        .await;
}

#[cfg(test)]
mod turn_error_tests {
    use super::format_turn_error;

    #[test]
    fn a_failed_turn_is_one_plain_line_never_the_providers_text() {
        let raw = "Gemini API error (429): RESOURCE_EXHAUSTED quota exceeded for metric …";
        let shown = format_turn_error(raw);
        assert!(
            shown.starts_with("Error: "),
            "the prefix every surface reads as the banner"
        );
        assert!(
            !shown.contains("RESOURCE_EXHAUSTED"),
            "the raw text stays in the log"
        );
        assert!(shown.contains("limit"));
        assert!(format_turn_error("connection timed out").contains("reach the model"));
    }

    #[test]
    fn an_auth_failure_keeps_its_shape_for_the_inline_sign_in() {
        assert_eq!(
            format_turn_error("AUTH_REQUIRED: chatgpt"),
            "Error: AUTH_REQUIRED: chatgpt"
        );
    }

    #[test]
    fn a_spent_allowance_keeps_its_marker_for_the_subscribe_card() {
        assert_eq!(
            format_turn_error("BILLING_REQUIRED: Your free tokens are used up"),
            "Error: BILLING_REQUIRED: Your free tokens are used up"
        );
    }
}

#[cfg(test)]
mod via_tests {
    use super::{persist_and_emit_to_store, persist_and_emit_with_images};
    use crate::server::ServerEvent;

    /// A dequeued hand-off persists and emits through this call with the
    /// `via` it was queued with; the live event carries it too.
    #[tokio::test]
    async fn a_hand_off_keeps_via_on_the_row_and_the_live_event() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::state_fs::SessionStore::with_sessions_dir(dir.path().to_path_buf());
        let (tx, mut rx) = tokio::sync::broadcast::channel(8);
        persist_and_emit_to_store(
            &store,
            &tx,
            "ling",
            "user",
            "ling",
            "how many songs?",
            Some("s1"),
            false,
            None,
            Some("iphone"),
            Some("Let me check how many songs we have here."),
        )
        .await;
        match rx.recv().await.unwrap() {
            ServerEvent::Message { via, .. } => assert_eq!(via.as_deref(), Some("iphone")),
            _ => panic!("expected a message event"),
        }
        let rows = store.get_chat_history("s1").unwrap();
        assert_eq!(rows[0].via.as_deref(), Some("iphone"));
        assert_eq!(
            rows[0].via_note.as_deref(),
            Some("Let me check how many songs we have here.")
        );
    }

    /// A pictured turn's live event names the kept copies by the same URLs
    /// a reload hands out, so a second screen draws them as it lands.
    #[tokio::test]
    async fn a_pictured_turn_carries_its_kept_pictures_live() {
        use base64::Engine as _;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::state_fs::SessionStore::with_sessions_dir(dir.path().to_path_buf());
        let (tx, mut rx) = tokio::sync::broadcast::channel(8);
        // The smallest PNG header the keeper accepts as a picture.
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(&[0u8; 32]);
        let sent = vec![base64::engine::general_purpose::STANDARD.encode(&png)];
        persist_and_emit_with_images(
            &store,
            &tx,
            "ling",
            ("user", "ling", "what is this"),
            Some("s1"),
            false,
            Some("c-1"),
            (None, None),
            &sent,
        )
        .await;
        let rows = store.get_chat_history("s1").unwrap();
        assert_eq!(rows[0].images.len(), 1, "the row keeps the picture");
        let want = crate::state_fs::session_images::url("s1", &rows[0].images[0]);
        match rx.recv().await.unwrap() {
            ServerEvent::Message {
                images, client_id, ..
            } => {
                assert_eq!(images, vec![want]);
                assert_eq!(client_id.as_deref(), Some("c-1"));
            }
            _ => panic!("expected a message event"),
        }
    }
}
