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
pub(crate) fn format_turn_error(msg: &str) -> String {
    format!("Error: {}", msg)
}

// ---------------------------------------------------------------------------
// Message persistence
// ---------------------------------------------------------------------------

/// Emit a `ServerEvent::Message` **and** persist to session files.
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
) {
    let _ = events_tx.send(ServerEvent::Message {
        from: from.to_string(),
        to: to.to_string(),
        content: content.to_string(),
        session_id: session_id.map(|s| s.to_string()),
                run_id: None,
                parent_agent_id: None,
            });
    persist_message_only(manager, root, agent_id, from, to, content, session_id, is_observation)
        .await;
}

/// Emit a `ServerEvent::Message` and persist directly to a `SessionStore`.
/// Used for mission sessions that live outside any project.
pub(crate) async fn persist_and_emit_to_store(
    store: &crate::state_fs::SessionStore,
    events_tx: &broadcast::Sender<ServerEvent>,
    agent_id: &str,
    from: &str,
    to: &str,
    content: &str,
    session_id: Option<&str>,
    is_observation: bool,
) {
    let _ = events_tx.send(ServerEvent::Message {
        from: from.to_string(),
        to: to.to_string(),
        content: content.to_string(),
        session_id: session_id.map(|s| s.to_string()),
                run_id: None,
                parent_agent_id: None,
            });
    let sid = session_id.unwrap_or("default");
    let msg = crate::state_fs::sessions::ChatMsg {
        agent_id: agent_id.to_string(),
        from_id: from.to_string(),
        to_id: to.to_string(),
        content: content.to_string(),
        timestamp: crate::util::now_ts_secs(),
        is_observation,
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
) {
    let sid = session_id.unwrap_or("default");
    let msg = crate::state_fs::sessions::ChatMsg {
        agent_id: agent_id.to_string(),
        from_id: from.to_string(),
        to_id: to.to_string(),
        content: content.to_string(),
        timestamp: crate::util::now_ts_secs(),
        is_observation,
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

pub(crate) fn emit_outcome_event(
    outcome: &AgentOutcome,
    events_tx: &broadcast::Sender<ServerEvent>,
    from_id: &str,
    session_id: Option<&str>,
) {
    let sid = session_id.map(|s| s.to_string());
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
                run_id: None,
                parent_agent_id: None,
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
                run_id: None,
                parent_agent_id: None,
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
    let _ = events_tx.send(ServerEvent::Outcome {
        agent_id: from_id.to_string(),
        outcome: outcome.clone(),
        session_id: sid.clone(),
    });
}
