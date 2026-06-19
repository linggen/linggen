//! Yinyue's event-reactive watch loop.
//!
//! Taps the server event bus and, on a few coarse, report-worthy events, wakes
//! the Yinyue agent to decide whether to tell the user. First slice: react only
//! to a *non-Yinyue* mission finishing.
//!
//! The reaction is launched as a plain **agent run** (not a mission), so it (a)
//! never persists a `missions/yinyue-react/` dir that would pollute the mission
//! list, and (b) runs with Yinyue's full `yinyue.md` system prompt rather than a
//! mission body that replaces it.
//!
//! Guards:
//! 1. No self-loop — an agent run does not emit `MissionCompleted`, so a reaction
//!    can't re-trigger this loop. The `yinyue` mission-id check below is kept as
//!    belt-and-suspenders for any future mission-shaped reaction.
//! 2. Cost — match only the coarse event(s); the per-token firehose
//!    (`Token` / `TextSegment` / `ContentBlock*`) falls through the `else` arm at
//!    near-zero cost. The LLM is woken only on a narrow trigger.

use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

use super::events::{NotificationPayload, ServerEvent};
use super::state::ServerState;

const YINYUE_AGENT: &str = "yinyue";
/// All of Yinyue's reactions share one ongoing session, so they serialize
/// (one `agent.lock()` at a time) and read as a single continuing thread
/// rather than littering the session list.
const REACT_SESSION_ID: &str = "sess-yinyue";

pub async fn yinyue_watch_loop(state: Arc<ServerState>) {
    let mut rx = state.events_tx.subscribe();
    tracing::info!("[yinyue-watch] started");
    loop {
        match rx.recv().await {
            Ok(event) => handle_event(&state, event),
            Err(RecvError::Lagged(n)) => {
                tracing::warn!("[yinyue-watch] lagged; skipped {n} events");
            }
            Err(RecvError::Closed) => break,
        }
    }
}

/// Cheap, synchronous classifier. Matches only the coarse trigger and spawns
/// the (async) wake; every other event returns immediately.
fn handle_event(state: &Arc<ServerState>, event: ServerEvent) {
    let ServerEvent::Notification(NotificationPayload::MissionCompleted {
        mission_id,
        mission_name,
        status,
        ..
    }) = event
    else {
        return; // firehose + all other events dropped here, near-free
    };

    // Guard: never react to a Yinyue-shaped mission (belt-and-suspenders).
    if mission_id.starts_with(YINYUE_AGENT) {
        return;
    }

    tracing::info!("[yinyue-watch] mission '{mission_id}' completed ({status}); waking Yinyue");
    let state = state.clone();
    tokio::spawn(async move {
        wake_for_mission(state, &mission_name, &status).await;
    });
}

/// Wake the Yinyue agent with the event as her task, via a plain agent run
/// (mirrors `api::agents::run_agent`, minus the mission machinery). She runs
/// with her full system prompt and decides whether to surface anything.
async fn wake_for_mission(state: Arc<ServerState>, mission_name: &str, status: &str) {
    let task = format!(
        "You've been woken to react to a background event on the user's machine. \
         The background job \"{mission_name}\" just finished (status: {status}). \
         Decide whether it's worth telling the user; if so, say it in one or two brief \
         sentences, in your voice — what happened and anything notable (you may Memory_query, \
         Read, or Grep for context). If it's routine and not worth interrupting them, do \
         nothing. Be brief. Never nag."
    );

    let root = crate::util::resolve_path(std::path::Path::new("~/.linggen"));

    let agent = match state
        .manager
        .get_or_create_session_agent(REACT_SESSION_ID, &root, YINYUE_AGENT)
        .await
    {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("[yinyue-watch] could not create Yinyue agent: {e}");
            return;
        }
    };

    let run_id = state
        .manager
        .begin_agent_run(
            &root,
            Some(REACT_SESSION_ID),
            YINYUE_AGENT,
            None,
            Some("yinyue-watch".to_string()),
        )
        .await
        .unwrap_or_else(|_| format!("run-{YINYUE_AGENT}-fallback"));

    let (run_status, err_msg) = {
        let mut engine = agent.lock().await;
        engine.set_parent_agent(None);
        engine.set_task(task);
        engine.set_run_id(Some(run_id.clone()));
        let result = engine.run_agent_loop(Some(REACT_SESSION_ID)).await;
        engine.set_run_id(None);
        match result {
            Ok(_) => (crate::engine::agent::AgentRunStatus::Completed, None),
            Err(e) => {
                let msg = e.to_string();
                let status = if msg.to_lowercase().contains("cancel") {
                    crate::engine::agent::AgentRunStatus::Cancelled
                } else {
                    crate::engine::agent::AgentRunStatus::Failed
                };
                (status, Some(msg))
            }
        }
    };

    if let Some(ref m) = err_msg {
        tracing::warn!("[yinyue-watch] reaction run failed: {m}");
    }
    let _ = state
        .manager
        .finish_agent_run(&run_id, run_status, err_msg)
        .await;
}
