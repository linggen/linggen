//! Claude sessions are hosted here, outside the native AgentEngine pipeline.
//! Only transport, run bookkeeping and UI events are shared with native chat.
mod install;
mod interaction;
mod media;
mod protocol;
mod runtime;
pub(crate) mod subagents;
#[cfg(test)]
mod tests;
pub(crate) mod usage;

use super::{chat, ServerState};
use crate::config::ModelConfig;
use crate::engine::agent::AgentRunStatus;
use crate::state_fs::sessions::{SessionMember, SessionMeta, SessionRuntime};
use anyhow::{bail, Context, Result};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::{path::Path, sync::Arc};

pub(crate) const AGENT_ID: &str = "claude";

pub(crate) struct ModeChange {
    pub mode: crate::state_fs::sessions::ClaudePermissionMode,
    pub reply: tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
}

pub(crate) enum Control {
    Mode(ModeChange),
    StopTask {
        task_id: String,
        reply: tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
    },
}

#[derive(serde::Deserialize)]
pub(crate) struct CapabilitiesQuery {
    session_id: String,
}

pub(crate) async fn capabilities(
    axum::extract::State(state): axum::extract::State<Arc<ServerState>>,
    axum::extract::Query(query): axum::extract::Query<CapabilitiesQuery>,
) -> Response {
    let Ok(Some(meta)) = state
        .manager
        .global_sessions
        .get_session_meta(&query.session_id)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if meta.runtime.is_linggen() || meta.skill.is_some() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let dir = state.manager.global_sessions.session_dir(&meta.id);
    for name in ["claude.json", "claude-capabilities.json"] {
        if let Some(value) = std::fs::read(dir.join(name))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        {
            if value["commands"].is_array() {
                return Json(json!({"commands":value["commands"]})).into_response();
            }
        }
    }
    match runtime::validate_mode(&state, &meta, meta.claude_permission_mode).await {
        Ok(()) => match std::fs::read(dir.join("claude-capabilities.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        {
            Some(value) => Json(value).into_response(),
            None => Json(json!({"commands":[]})).into_response(),
        },
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

/// A live change is acknowledged by the SDK before the API persists it.
pub(crate) async fn set_mode(
    state: &ServerState,
    session: &SessionMeta,
    mode: crate::state_fs::sessions::ClaudePermissionMode,
) -> Result<()> {
    let sender = state.claude_controls.lock().await.get(&session.id).cloned();
    if let Some(sender) = sender {
        let (reply, response) = tokio::sync::oneshot::channel();
        sender
            .send(Control::Mode(ModeChange { mode, reply }))
            .map_err(|_| anyhow::anyhow!("Claude turn ended; try changing the mode again."))?;
        tokio::time::timeout(std::time::Duration::from_secs(20), response)
            .await
            .context("Claude permission change timed out")?
            .context("Claude turn ended before applying the permission change")?
            .map_err(anyhow::Error::msg)?;
    } else if mode == crate::state_fs::sessions::ClaudePermissionMode::Auto {
        runtime::validate_mode(state, session, mode).await?;
    }
    Ok(())
}

/// Clear/delete owns the turn lock only after the old SDK process unwound.
pub(crate) async fn quiesce(
    state: &ServerState,
    session: &str,
) -> tokio::sync::OwnedMutexGuard<()> {
    state.queued_chats.lock().await.retain(|_, items| {
        items.retain(|item| item.session_id != session);
        !items.is_empty()
    });
    let turn = state.manager.session_turn(session);
    loop {
        if let Ok(guard) = turn.clone().try_lock_owned() {
            return guard;
        }
        // A submitted turn can own the lock before its run record exists.
        // Keep cancelling until it unwinds, including that startup window.
        for run in state
            .manager
            .list_agent_runs(&std::path::PathBuf::new(), Some(session))
            .await
            .unwrap_or_default()
        {
            let _ = state.manager.cancel_run_tree(&run.run_id).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

pub(crate) fn supports_model(model: &ModelConfig) -> bool {
    model.provider == "anthropic"
        && model.auth_mode.as_deref() != Some("claude_oauth")
        && model.api_key.as_deref().is_some_and(|key| !key.is_empty())
}

pub(crate) async fn resolve_model(
    state: &ServerState,
    chosen: Option<&str>,
) -> Result<ModelConfig> {
    let config = state.manager.get_config_snapshot().await;
    let models = state.manager.models.read().await;
    let available = models.list_models();
    if let Some(id) = chosen.filter(|id| !id.is_empty()) {
        let model = available
            .iter()
            .find(|m| m.id == id)
            .context("Claude model is not configured")?;
        if !supports_model(model) {
            bail!("Choose a Claude API-key model in Settings → Models.");
        }
        return Ok((*model).clone());
    }
    config
        .routing
        .default_models
        .iter()
        .chain(config.models.iter().map(|m| &m.id))
        .find_map(|id| available.iter().find(|m| m.id == *id && supports_model(m)))
        .cloned()
        .cloned()
        .context("Add a Claude API-key model in Settings → Models.")
}

pub(crate) fn new_meta(title: String, cwd: String, parent: Option<String>) -> SessionMeta {
    SessionMeta {
        id: format!(
            "sess-{}-{}",
            crate::util::now_ts_secs(),
            &uuid::Uuid::new_v4().to_string()[..8]
        ),
        title,
        cwd,
        created_at: crate::util::now_ts_secs(),
        runtime: SessionRuntime::ClaudeAgentSdk,
        agents: vec![SessionMember::new(AGENT_ID)],
        parent_session_id: parent,
        creator: "user".into(),
        title_locked: false,
        ..Default::default()
    }
}

pub(crate) async fn start_task(state: Arc<ServerState>, id: String, message: String, from: String) {
    let Ok(Some(meta)) = state.manager.global_sessions.get_session_meta(&id) else {
        return;
    };
    let req: chat::types::ChatRequest =
        serde_json::from_value(json!({"project_root":"", "session_id":id,
        "agent_id":AGENT_ID, "message":message, "sender":from}))
        .expect("host task request");
    let _ = start_turn(state, req, meta).await;
}

pub(crate) async fn start_turn(
    state: Arc<ServerState>,
    req: chat::types::ChatRequest,
    meta: SessionMeta,
) -> Response {
    if req.skill_name.is_some() || meta.skill.is_some() {
        return (
            StatusCode::BAD_REQUEST,
            "Linggen apps require a Linggen session.",
        )
            .into_response();
    }
    // Consumer rooms share model inference, not access to an owner's SDK shell.
    if req.user_type == "consumer" {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !req.agent_id.is_empty() && req.agent_id != AGENT_ID {
        return (
            StatusCode::BAD_REQUEST,
            "Claude sessions have one Claude Agent.",
        )
            .into_response();
    }
    let chosen = req
        .model_id
        .as_deref()
        .or_else(|| meta.member(AGENT_ID).and_then(|m| m.model.as_deref()));
    if let Err(error) = resolve_model(&state, chosen).await {
        return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
    }
    let turn = state.manager.session_turn(&meta.id);
    // Reserve before spawning: two simultaneous first messages cannot both
    // appear idle and write messages out of order.
    let reservation = turn.clone().try_lock_owned().ok();
    let busy = reservation.is_none();
    let queued = chat::enqueue_if_busy(
        &state,
        busy,
        &meta.cwd,
        &meta.id,
        AGENT_ID,
        &req.message,
        crate::engine::skill::QueueMode::Steer,
    )
    .await;
    if !busy {
        persist_user(&state, &meta, &req).await;
    }
    let sid = meta.id.clone();
    tokio::spawn(async move {
        let _guard = match reservation {
            Some(guard) => guard,
            None => turn.lock_owned().await,
        };
        if let Some(item) = queued {
            if !chat::dequeue_and_emit(
                &state,
                &state.events_tx,
                &item.id,
                &meta.cwd,
                &meta.id,
                AGENT_ID,
                req.sender.as_deref().unwrap_or("user"),
                &req.message,
                Some(&meta.id),
                req.via.as_deref(),
                req.via_note.as_deref(),
                &req.images,
            )
            .await
            {
                return;
            }
        }
        execute_turn(state, req, meta).await;
    });
    Json(json!({"status": if busy {"queued"} else {"started"}, "session_id": sid, "agent_id": AGENT_ID})).into_response()
}

async fn persist_user(state: &ServerState, meta: &SessionMeta, req: &chat::types::ChatRequest) {
    chat::helpers::persist_and_emit_with_images(
        &state.manager.global_sessions,
        &state.events_tx,
        AGENT_ID,
        (
            req.sender.as_deref().unwrap_or("user"),
            AGENT_ID,
            &req.message,
        ),
        Some(&meta.id),
        false,
        req.client_id.as_deref(),
        (req.via.as_deref(), req.via_note.as_deref()),
        &req.images,
    )
    .await;
    if !meta.title_locked {
        let title: String = req
            .message
            .split_whitespace()
            .take(6)
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(50)
            .collect();
        let _ = state
            .manager
            .global_sessions
            .rename_session(&meta.id, &title);
    }
}

async fn execute_turn(state: Arc<ServerState>, req: chat::types::ChatRequest, meta: SessionMeta) {
    // Only the task Ling handed over reports back. Subsequent user chat
    // belongs entirely to the Claude session.
    let notify_parent = req.sender.as_deref().is_some_and(|sender| sender != "user");
    let root = Path::new(&meta.cwd);
    let run_id = match state
        .manager
        .begin_agent_run(
            root,
            Some(&meta.id),
            AGENT_ID,
            None,
            Some(req.message.clone()),
        )
        .await
    {
        Ok(id) => id,
        Err(error) => {
            tracing::error!("Claude run could not start: {error}");
            return;
        }
    };
    let key = chat::helpers::queue_key(&meta.cwd, &meta.id, AGENT_ID);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    state.interrupt_tx.lock().await.insert(key.clone(), tx);
    let (control_tx, control_rx) = tokio::sync::mpsc::unbounded_channel();
    state
        .claude_controls
        .lock()
        .await
        .insert(meta.id.clone(), control_tx);
    let mut host = runtime::ClaudeSession {
        state: state.clone(),
        meta,
        run_id: run_id.clone(),
        interrupt: rx,
    };
    let result =
        crate::util::panic_as_error(host.run(req.message, req.images, req.model_id, control_rx))
            .await;
    state.claude_controls.lock().await.remove(&host.meta.id);
    state.interrupt_tx.lock().await.remove(&key);
    let cancelled = state.manager.is_run_cancelled(&run_id).await
        || result
            .as_ref()
            .err()
            .is_some_and(|e| e.to_string().contains("cancelled"));
    let status = if cancelled {
        AgentRunStatus::Cancelled
    } else if result.is_err() {
        AgentRunStatus::Failed
    } else {
        AgentRunStatus::Completed
    };
    if let Err(ref error) = result {
        if !cancelled {
            let text = format!("Error: {error}");
            host.text(&text).await;
            host.finalize_text(&text).await;
        }
    }
    let _ = state
        .manager
        .finish_agent_run(&run_id, status, result.err().map(|e| e.to_string()))
        .await;
    chat::close_turn(
        &state,
        AGENT_ID.into(),
        Some(host.meta.id.clone()),
        Some(run_id),
    )
    .await;
    // A linked parent gets a status/link, never the SDK transcript or a new
    // native turn. Follow-up interaction stays in the Claude session.
    if let Some(parent) = host
        .meta
        .parent_session_id
        .as_ref()
        .filter(|_| notify_parent)
    {
        let result = if cancelled {
            "stopped"
        } else if status == AgentRunStatus::Failed {
            "failed"
        } else {
            "completed"
        };
        chat::helpers::persist_and_emit_message(
            &state.manager,
            &state.events_tx,
            Path::new(&host.meta.cwd),
            "ling",
            AGENT_ID,
            "user",
            &format!(
                "Claude task {result}. [Open session](/?session={})",
                host.meta.id
            ),
            Some(parent),
            false,
            None,
        )
        .await;
    }
}
