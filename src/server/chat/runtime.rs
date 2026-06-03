use crate::server::chat::helpers::persist_and_emit_message;
use crate::server::{AgentStatusKind, ServerEvent, ServerState};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;

use super::ChatRunCtx;

pub(super) async fn run_loop_with_tracking(
    manager: &Arc<crate::engine::agent::AgentManager>,
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
                    .finish_agent_run(&run_id, crate::engine::agent::AgentRunStatus::Completed, None)
                    .await;
            }
            Err(err) => {
                let msg = err.to_string();
                let status = if msg.to_lowercase().contains("cancel") {
                    crate::engine::agent::AgentRunStatus::Cancelled
                } else {
                    tracing::error!("Agent loop failed: {}", msg);
                    crate::engine::agent::AgentRunStatus::Failed
                };
                let _ = manager.finish_agent_run(&run_id, status, Some(msg.clone())).await;
                // AUTH_REQUIRED errors render as a structured block in chat so
                // the UI can show an inline "Sign in with ChatGPT" button —
                // no need to navigate to Settings → Models to re-authenticate.
                let display = crate::server::chat::helpers::format_turn_error(&msg);
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

/// One row surfaced by per-turn auto-recall. Carries the id so the
/// agent can act on duplicates / conflicts directly via `Memory_write`,
/// and the UI can deep-link a row to the memory dashboard. Score is the
/// raw cosine similarity from the embedding store so the UI can render
/// match strength and the engine can gate on quality.
#[derive(Debug, Clone)]
pub(super) struct RecallRow {
    pub id: String,
    pub r#type: String,
    pub host: String,
    pub date: String,
    pub content: String,
    pub score: f32,
}

impl RecallRow {
    /// One-line rendering for injection into the model's context. Matches
    /// CC's `recall.sh` shape so a row reads the same in linggen and CC,
    /// with an added `score=0.NN` field — the UI parses it for a badge
    /// and the model can use it to gauge confidence.
    fn to_line(&self) -> String {
        format!(
            "From memory ({}, {}, {}, score={:.2}, id={}): {}",
            self.r#type, self.host, self.date, self.score, self.id, self.content
        )
    }
}

/// Format a recall hit list into the block the model sees. Includes the
/// same reconcile footer the always-on block uses (`prompt/core_block.rs`),
/// gated on `rows.len() > 1` — single-hit blocks have nothing to dedup
/// or compare against.
fn format_recall_for_model(rows: &[RecallRow]) -> String {
    let mut out = rows
        .iter()
        .map(|r| r.to_line())
        .collect::<Vec<_>>()
        .join("\n");
    if rows.len() > 1 {
        out.push_str(crate::engine::prompt::core_block::RECONCILE_FOOTER);
    }
    out
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
    min_score: Option<f32>,
    ling_mem_url: &str,
) -> Option<Vec<RecallRow>> {
    use std::time::Duration;
    const RECALL_BUDGET: Duration = Duration::from_secs(3);
    // Fetch wide so the project-scope filter has headroom before the
    // TOP_K cap kicks in.
    const FETCH_LIMIT: usize = 30;
    const TOP_K: usize = 10;
    const MIN_PROMPT_CHARS: usize = 8;

    let trimmed = prompt.trim();
    if trimmed.chars().count() < MIN_PROMPT_CHARS {
        return None;
    }

    // Memory is engine-built-in now; no `active_provider("memory")`
    // gate. If the daemon isn't up, the dispatch call's autostart
    // (`ling-mem start`) will spin it up; if even that fails, the
    // dispatch errors out and recall silently bails below.

    let project_name: Option<String> = session_id
        .and_then(|sid| state.manager.global_sessions.get_session_meta(sid).ok().flatten())
        .and_then(|m| m.project_name);

    // `min_score` is the per-row cosine floor (Settings → General → Memory
    // Inject Score). `None` means defer to the daemon's store-wide
    // `recall_min_score` — we omit the field and ling-mem applies its own
    // floor, so all hosts share one selectivity. `Some(s)` overrides. Either
    // way the daemon drops weak rows before they cross the wire; there's no
    // separate aggregate gate.
    let mut args = serde_json::json!({
        "verb": "search",
        "query": trimmed,
        "limit": FETCH_LIMIT,
    });
    if let Some(s) = min_score {
        args["min_score"] = serde_json::json!(s);
    }

    let dispatch = crate::engine::tools::memory_tool::call_memory_http(
        ling_mem_url,
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

    let mut hits: Vec<RecallRow> = Vec::new();
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

        let id = row.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if id.is_empty() {
            // Without an id the agent can't act on the row (delete/replace_ids
            // need one) and the UI can't deep-link. Skip rather than surface
            // a half-row.
            continue;
        }
        let typ = row.get("type").and_then(|v| v.as_str()).unwrap_or("fact").to_string();
        let host = row.get("host").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
        let date = row
            .get("created_at")
            .and_then(|v| v.as_str())
            .map(|s| if s.len() >= 10 { &s[..10] } else { s })
            .unwrap_or("")
            .to_string();
        let content = row
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if content.is_empty() {
            continue;
        }
        let score = row
            .get("score")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
            .unwrap_or(0.0);
        hits.push(RecallRow { id, r#type: typ, host, date, content, score });
    }

    if hits.is_empty() {
        None
    } else {
        Some(hits)
    }
}

/// Push the user message onto the engine's chat history with auto-recall
/// applied. Used by every turn-start site (skill dispatch, trigger
/// dispatch, plan mode, structured loop).
pub(super) async fn push_user_turn_with_recall(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
) {
    // Same gate as the core block + memory protocol injection: skill /
    // mission sessions don't query the user's biographical memory.
    // Without this, a Pulse turn fires Memory_query, surfaces hits, and
    // the "🧠 N memories recalled" widget appears in a skill session that
    // shouldn't touch memory at all.
    let recalled = if engine.prompt_profile.include_memory {
        auto_recall_memory(
            &ctx.state,
            &ctx.clean_msg,
            ctx.session_id.as_deref(),
            engine.cfg.memory_inject_min_score,
            &engine.cfg.ling_mem_url,
        )
        .await
    } else {
        None
    };
    if let Some(rows) = recalled {
        // What the model sees: structured "From memory (...): ..." lines
        // followed by the reconcile footer (when ≥2 rows). Same shape as
        // CC's recall.sh and the engine's always-on block — one protocol
        // across all surfaces.
        let model_text = format_recall_for_model(&rows);
        engine
            .chat_history
            .push(crate::message::ChatMessage::new("system", model_text.clone()));

        // What the user sees: persisted as a chat message with
        // from_id="memory" so the UI can render a collapsible widget and
        // the chat export naturally includes the recall. Content is the
        // same text the model received — no separate channel — so what
        // the user sees is exactly what the model saw.
        crate::server::chat::helpers::persist_and_emit_to_store(
            &ctx.manager.global_sessions,
            &ctx.events_tx,
            &ctx.agent_id,
            "memory",
            &ctx.agent_id,
            &model_text,
            ctx.session_id.as_deref(),
            false,
        )
        .await;
    }

    // Always-on per-turn capture nudge — model-only, fires every owner turn
    // (including zero-recall turns, which often produce the new memory).
    // Gated on the same include_memory check as recall so skill/mission
    // sessions stay out. Not persisted to the recall widget: it's an
    // instruction to the agent, not a recalled row. Mirrors CC/Codex
    // recall.sh so the per-turn reminder is identical across hosts.
    if engine.prompt_profile.include_memory {
        engine.chat_history.push(crate::message::ChatMessage::new(
            "system",
            crate::engine::prompt::core_block::CAPTURE_REMINDER.to_string(),
        ));
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

#[cfg(test)]
mod tests {
    use super::RecallRow;

    /// The injected line shape is a contract with `MemoryRecallMessage.tsx`
    /// (regex on the UI side parses these tokens). Keep them in lockstep.
    #[test]
    fn recall_row_to_line_includes_score_in_expected_shape() {
        let row = RecallRow {
            id: "abc12345-aaaa-bbbb-cccc-deadbeef0000".into(),
            r#type: "fact".into(),
            host: "linggen".into(),
            date: "2026-05-20".into(),
            content: "User prefers ~150-word replies.".into(),
            score: 0.7150189,
        };
        assert_eq!(
            row.to_line(),
            "From memory (fact, linggen, 2026-05-20, score=0.72, id=abc12345-aaaa-bbbb-cccc-deadbeef0000): User prefers ~150-word replies."
        );
    }
}
