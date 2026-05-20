use crate::server::chat::helpers::persist_and_emit_message;
use crate::server::{AgentStatusKind, ServerEvent, ServerState};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;

use super::ChatRunCtx;

pub(super) async fn run_loop_with_tracking(
    manager: &Arc<crate::agent_manager::AgentManager>,
    root: &PathBuf,
    engine: &mut crate::engine::AgentEngine,
    agent_id: &str,
    session_id: Option<&str>,
    detail: &str,
    events_tx: &broadcast::Sender<ServerEvent>,
) -> Result<crate::engine::AgentOutcome, anyhow::Error> {
    let run_id = manager
        .begin_agent_run(root, session_id, agent_id, None, Some(detail.to_string()))
        .await
        .ok();

    engine.set_run_id(run_id.clone());
    let result = engine.run_agent_loop(session_id).await;
    engine.set_run_id(None);

    if let Some(run_id) = run_id {
        match &result {
            Ok(_) => {
                let _ = manager
                    .finish_agent_run(&run_id, crate::agent_manager::AgentRunStatus::Completed, None)
                    .await;
            }
            Err(err) => {
                let msg = err.to_string();
                let status = if msg.to_lowercase().contains("cancel") {
                    crate::agent_manager::AgentRunStatus::Cancelled
                } else {
                    tracing::error!("Agent loop failed: {}", msg);
                    crate::agent_manager::AgentRunStatus::Failed
                };
                let _ = manager.finish_agent_run(&run_id, status, Some(msg.clone())).await;
                // AUTH_REQUIRED errors include the engine's "open Settings" hint
                // so the UI can show the inline Settings → Models pointer.
                let display = if msg.starts_with("AUTH_REQUIRED:") {
                    msg.trim_start_matches("AUTH_REQUIRED:").trim().to_string()
                } else {
                    format!("Error: {}", msg)
                };
                let _ = events_tx.send(ServerEvent::Message {
                    from: agent_id.to_string(),
                    to: "user".to_string(),
                    content: display,
                    session_id: session_id.map(|s| s.to_string()),
                run_id: None,
                parent_agent_id: None,
            });
                // Reset agent status so the UI's "Model Loading…" spinner stops.
                let _ = events_tx.send(ServerEvent::AgentStatus {
                    agent_id: agent_id.to_string(),
                    status: "idle".to_string(),
                    detail: None,
                    status_id: None,
                    lifecycle: Some("done".to_string()),
                    parent_agent_id: None,
                    session_id: session_id.map(|s| s.to_string()),
                    run_id: None,
                    parent_run_id: None,
                });
            }
        }
    }

    result
}

/// Wire the interrupt channel into the engine and store the sender in ServerState.
/// Returns the interrupt_key used to look up the sender later for cleanup.
pub(super) async fn wire_interrupt_channel(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
) -> String {
    let (interrupt_tx, interrupt_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    engine.interrupt_rx = Some(interrupt_rx);

    let interrupt_key = crate::server::chat::helpers::queue_key(
        &ctx.root.to_string_lossy(),
        ctx.session_id.as_deref().unwrap_or(""),
        &ctx.agent_id,
    );
    {
        let mut guard = ctx.state.interrupt_tx.lock().await;
        guard.insert(interrupt_key.clone(), interrupt_tx);
    }
    interrupt_key
}

/// Remove the interrupt channel from both the engine and ServerState.
pub(super) async fn unwire_interrupt_channel(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
    interrupt_key: &str,
) {
    engine.interrupt_rx = None;
    let mut guard = ctx.state.interrupt_tx.lock().await;
    guard.remove(interrupt_key);
}

/// Wire the AskUser bridge so the tool can emit SSE events and block on user response.
pub(super) fn wire_ask_user_bridge(
    state: &Arc<ServerState>,
    engine: &mut crate::engine::AgentEngine,
    session_id: Option<String>,
) {
    let bridge = Arc::new(crate::engine::tools::AskUserBridge {
        events_tx: state.events_tx.clone(),
        pending: state.pending_ask_user.clone(),
        session_id,
    });
    engine.tools.set_ask_user_bridge(bridge);
}

/// Persist and emit the assistant's streamed text content so the UI can
/// finalize liveText → a permanent message bubble. Used for plan outcomes
/// where the engine doesn't persist the text itself.
pub(super) async fn persist_and_emit_last_assistant_text(
    ctx: &ChatRunCtx,
    engine: &crate::engine::AgentEngine,
) {
    if let Some(text) = &engine.last_assistant_text {
        if !text.is_empty() {
            persist_and_emit_message(
                &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                &ctx.agent_id, "user", text, ctx.session_id.as_deref(), false,
            )
            .await;
        }
    }
}

/// Per-turn semantic recall against the active memory provider.
///
/// Bails silently on any path that could block the user: short prompts,
/// no memory provider installed, daemon unreachable within the budget,
/// dispatch errors, malformed responses. Project-scoped rows from a
/// different project are filtered out.
async fn auto_recall_memory(
    state: &Arc<ServerState>,
    prompt: &str,
    session_id: Option<&str>,
) -> Option<String> {
    use std::time::Duration;
    const RECALL_BUDGET: Duration = Duration::from_secs(3);
    const FETCH_LIMIT: usize = 8;
    const TOP_K: usize = 3;
    const MIN_PROMPT_CHARS: usize = 8;
    /// Cosine similarity floor — calibrated for MiniLM-L6-v2 (strong matches
    /// land in [0.30, 0.45]; noise sits below 0.25). Override per-process with
    /// `LINGGEN_RECALL_MIN_SCORE`.
    const DEFAULT_MIN_SCORE: f32 = 0.30;

    let trimmed = prompt.trim();
    if trimmed.chars().count() < MIN_PROMPT_CHARS {
        return None;
    }

    if state.skill_manager.active_provider("memory").await.is_none() {
        return None;
    }

    let project_name: Option<String> = session_id
        .and_then(|sid| state.manager.global_sessions.get_session_meta(sid).ok().flatten())
        .and_then(|m| m.project_name);

    let min_score: f32 = std::env::var("LINGGEN_RECALL_MIN_SCORE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MIN_SCORE);

    let args = serde_json::json!({
        "verb": "search",
        "query": trimmed,
        "limit": FETCH_LIMIT,
        "min_score": min_score,
    });

    let dispatch = crate::engine::capability_tools::dispatch(
        &state.skill_manager,
        "Memory_query",
        args,
    );
    let result = match tokio::time::timeout(RECALL_BUDGET, dispatch).await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            tracing::debug!("auto-recall: dispatch failed: {e}");
            return None;
        }
        Err(_) => {
            tracing::debug!("auto-recall: dispatch exceeded {}s budget", RECALL_BUDGET.as_secs());
            return None;
        }
    };

    let rows = result.as_array()?;
    if rows.is_empty() {
        return None;
    }

    let want_proj_ctx = project_name.as_deref().map(|p| format!("project/{p}"));

    let mut hits: Vec<String> = Vec::new();
    for row in rows {
        if hits.len() >= TOP_K {
            break;
        }

        // Drop rows scoped to a different project; rows with no project/*
        // context (cross-project, no-context, domain-scoped) always pass.
        let project_scoped: Vec<&str> = row
            .get("contexts")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .filter(|s| s.starts_with("project/"))
                    .collect()
            })
            .unwrap_or_default();
        let project_ok = project_scoped.is_empty()
            || want_proj_ctx
                .as_deref()
                .map(|w| project_scoped.iter().any(|s| *s == w))
                .unwrap_or(false);
        if !project_ok {
            continue;
        }

        let typ = row.get("type").and_then(|v| v.as_str()).unwrap_or("fact");
        let date = row
            .get("created_at")
            .and_then(|v| v.as_str())
            .map(|s| if s.len() >= 10 { &s[..10] } else { s })
            .unwrap_or("");
        let content = row
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if content.is_empty() {
            continue;
        }
        hits.push(format!("From memory ({typ}, {date}): {content}"));
    }

    if hits.is_empty() {
        None
    } else {
        Some(hits.join("\n"))
    }
}

/// Push the user message onto the engine's chat history with auto-recall
/// applied. Used by every turn-start site (skill dispatch, trigger
/// dispatch, plan mode, structured loop).
pub(super) async fn push_user_turn_with_recall(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
) {
    if let Some(prefix) = auto_recall_memory(
        &ctx.state,
        &ctx.clean_msg,
        ctx.session_id.as_deref(),
    )
    .await
    {
        engine
            .chat_history
            .push(crate::message::ChatMessage::new("system", prefix));
    }
    engine
        .chat_history
        .push(crate::message::ChatMessage::new("user", ctx.clean_msg.clone()));
}

/// Forward an engine's thinking-channel events to the SSE event bus.
/// Spawned per-turn for plan dispatch, plan execution, and the structured loop.
pub(super) fn spawn_thinking_forwarder(
    mut thinking_rx: tokio::sync::mpsc::UnboundedReceiver<crate::engine::ThinkingEvent>,
    events_tx: broadcast::Sender<ServerEvent>,
    agent_id: String,
    session_id: Option<String>,
) {
    tokio::spawn(async move {
        while let Some(event) = thinking_rx.recv().await {
            let (token, done, thinking) = match event {
                crate::engine::ThinkingEvent::Token(t) => (t, false, true),
                crate::engine::ThinkingEvent::ContentToken(t) => (t, false, false),
                crate::engine::ThinkingEvent::Done => (String::new(), true, true),
                crate::engine::ThinkingEvent::ContentDone => (String::new(), true, false),
            };
            let _ = events_tx.send(ServerEvent::Token {
                session_id: session_id.clone(),
                agent_id: agent_id.clone(),
                token,
                done,
                thinking,
            });
        }
    });
}

/// Send "Thinking" agent-status with the supplied detail.
pub(super) async fn send_thinking_status(
    ctx: &ChatRunCtx,
    detail: impl Into<String>,
) {
    ctx.state
        .send_agent_status(
            ctx.agent_id.clone(),
            AgentStatusKind::Thinking,
            Some(detail.into()),
            None,
            ctx.session_id.clone(),
        )
        .await;
}
