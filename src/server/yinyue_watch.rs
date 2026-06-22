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

/// Wake the Yinyue agent to react to a finished background mission. She decides
/// whether it's worth surfacing — replying `SILENT` means say nothing (the
/// never-nag discipline). Anything else is spoken to her surfaces.
async fn wake_for_mission(state: Arc<ServerState>, mission_name: &str, status: &str) {
    let task = format!(
        "You've been woken to react to a background event on the user's machine. \
         The background job \"{mission_name}\" just finished (status: {status}). \
         Decide whether it's worth telling the user. If so, reply with one or two brief \
         sentences in your voice — what happened and anything notable (you may Memory_query, \
         Read, or Grep for context). Your reply will be SPOKEN ALOUD, so write plain prose, \
         no markdown. If it's routine and not worth interrupting them, reply with exactly the \
         single word SILENT and nothing else. Be brief. Never nag."
    );

    let Some(line) = run_yinyue_turn(&state, task).await else {
        return; // run failed or she produced nothing
    };
    if line.eq_ignore_ascii_case("silent") {
        tracing::info!("[yinyue-watch] Yinyue chose silence");
        return;
    }
    let emotion = if status.eq_ignore_ascii_case("completed") || status.to_lowercase().contains("success") {
        "happy"
    } else {
        "neutral"
    };
    tracing::info!("[yinyue-watch] Yinyue speaks ({} chars, {emotion})", line.len());
    crate::server::api::yinyue::emit_speak(&state, line, Some(emotion.to_string()));
}

/// Run one Yinyue turn on her shared `sess-yinyue` session and return her final
/// line (trimmed; `None` if the run failed or she produced no text). The single
/// place that drives the Yinyue agent — used by the event-reactive watch above
/// and by the "talk to her" endpoint (`api::yinyue::chat_handler`). All turns
/// share the one session + agent lock, so they serialize into a single thread.
pub(crate) async fn run_yinyue_turn(state: &Arc<ServerState>, task: String) -> Option<String> {
    let root = crate::util::resolve_path(std::path::Path::new("~/.linggen"));

    let agent = match state
        .manager
        .get_or_create_session_agent(REACT_SESSION_ID, &root, YINYUE_AGENT)
        .await
    {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("[yinyue] could not create Yinyue agent: {e}");
            return None;
        }
    };

    let run_id = state
        .manager
        .begin_agent_run(&root, Some(REACT_SESSION_ID), YINYUE_AGENT, None, Some("yinyue".to_string()))
        .await
        .unwrap_or_else(|_| format!("run-{YINYUE_AGENT}-fallback"));

    let (run_status, err_msg, spoken) = {
        let mut engine = agent.lock().await;
        engine.set_parent_agent(None);
        engine.set_task(task);
        engine.set_run_id(Some(run_id.clone()));
        // Clear so we read THIS turn's final line — the engine is reused across
        // turns on sess-yinyue and would otherwise hold the prior one.
        engine.last_assistant_text = None;
        let result = engine.run_agent_loop(Some(REACT_SESSION_ID)).await;
        engine.set_run_id(None);
        let spoken = engine.last_assistant_text.clone();
        let (status, err) = match result {
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
        };
        (status, err, spoken)
    };

    if let Some(ref m) = err_msg {
        tracing::warn!("[yinyue] turn failed: {m}");
    }
    let ran_ok = err_msg.is_none();
    let _ = state.manager.finish_agent_run(&run_id, run_status, err_msg).await;

    if !ran_ok {
        return None;
    }
    spoken.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}
