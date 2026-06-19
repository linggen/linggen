//! Yinyue's event-reactive watch loop.
//!
//! Taps the server event bus and, on a few coarse, report-worthy events, wakes
//! the Yinyue agent to decide whether to tell the user. First slice: react only
//! to a *non-Yinyue* mission finishing.
//!
//! Guards:
//! 1. No self-loop — Yinyue's own reaction run emits its own `MissionCompleted`;
//!    we skip mission ids that belong to her (the `yinyue` prefix), or her run
//!    would re-trigger her forever.
//! 2. Cost — match only the coarse event(s); the per-token firehose
//!    (`Token` / `TextSegment` / `ContentBlock*`) falls through the `else` arm at
//!    near-zero cost. The LLM is woken only on a narrow trigger.

use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

use super::events::{NotificationPayload, ServerEvent};
use super::state::ServerState;
use crate::engine::mission::record::Mission;

const YINYUE_AGENT: &str = "yinyue";
/// Mission id for Yinyue's ad-hoc reaction runs. The loop ignores
/// `MissionCompleted` events whose id starts with the agent name, so her own
/// reactions never re-trigger her (guard 1).
const REACT_MISSION_ID: &str = "yinyue-react";

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

    // Guard 1: never react to Yinyue's own reaction missions (self-loop).
    if mission_id.starts_with(YINYUE_AGENT) {
        return;
    }

    tracing::info!("[yinyue-watch] mission '{mission_id}' completed ({status}); waking Yinyue");
    let state = state.clone();
    tokio::spawn(async move {
        wake_for_mission(state, &mission_name, &status).await;
    });
}

/// Wake the Yinyue agent with the event as a user turn, reusing the mission
/// dispatch path. She decides whether it's worth surfacing to the user.
async fn wake_for_mission(state: Arc<ServerState>, mission_name: &str, status: &str) {
    let body = "You have been woken to react to something that just happened on the \
        user's machine. Read the message below, decide whether it is worth telling them, \
        and if so say it in one or two brief sentences, in your voice — what happened and \
        anything notable you can find (you may Memory_query for context). If it is routine \
        and not worth interrupting them, reply with exactly: SILENT. Be brief. Never nag."
        .to_string();

    let kickoff = vec![format!(
        "The background job \"{mission_name}\" just finished (status: {status})."
    )];

    let mission = Mission {
        id: REACT_MISSION_ID.to_string(),
        name: Some("Yinyue".to_string()),
        description: String::new(),
        schedule: String::new(),
        enabled: true,
        catchup_hours: None,
        cwd: None,
        model: None,
        kickoff,
        allowed_tools: vec![
            "Memory_query".to_string(),
            "Memory_write".to_string(),
            "Read".to_string(),
            "Grep".to_string(),
        ],
        permission: None,
        prompt: body,
        agent_id: YINYUE_AGENT.to_string(),
        project: None,
        created_at: crate::util::now_ts_secs(),
    };

    let root = crate::util::resolve_path(std::path::Path::new("~/.linggen"));
    let project_path = root.to_string_lossy().to_string();

    crate::extensions::missions::scheduler::dispatch_mission_prompt_public(
        state,
        root,
        &project_path,
        &mission,
        None,
    )
    .await;
}
