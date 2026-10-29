//! Claude sessions are hosted here, outside the native AgentEngine pipeline.
//! Only transport, run bookkeeping and UI events are shared with native chat.
mod host;
mod install;
mod interaction;
mod media;
mod probe;
mod protocol;
mod runtime;
mod state;
pub(crate) mod subagents;
#[cfg(test)]
mod tests;
mod tasks;
pub(crate) mod trust;
pub(crate) mod usage;

pub(crate) use host::Host;

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
    /// A queued message the person chose to send now, into the running turn.
    Steer {
        text: String,
        images: Vec<String>,
        reply: tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
    },
    /// Stop on the turn: the turn ends; background tasks keep running.
    Interrupt {
        reply: tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
    },
    /// The runner's settings changed: it exits once it has nothing to do.
    End,
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
    if let Some(commands) = state::cached_commands(&dir) {
        return Json(commands).into_response();
    }
    match probe::probe(&state, &meta, meta.claude_permission_mode).await {
        Ok(()) => Json(state::cached_commands(&dir).unwrap_or_else(|| json!({"commands":[]})))
            .into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

/// A live change is acknowledged by the SDK before the API persists it.
pub(crate) async fn set_mode(
    state: &ServerState,
    session: &SessionMeta,
    mode: crate::state_fs::sessions::ClaudePermissionMode,
) -> Result<()> {
    let sender = state.claude.controls.lock().await.get(&session.id).cloned();
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
        if mode == crate::state_fs::sessions::ClaudePermissionMode::Auto {
            state::mark_auto_verified(&state.manager.global_sessions.session_dir(&session.id));
        }
    } else if mode == crate::state_fs::sessions::ClaudePermissionMode::Auto {
        probe::probe(state, session, mode).await?;
    }
    Ok(())
}

/// How long Clear/Delete waits for a cancelled turn before killing its runner.
const QUIESCE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// Clear/delete owns the turn lock only after the old SDK process unwound.
pub(crate) async fn quiesce(
    state: &ServerState,
    session: &str,
) -> Result<tokio::sync::OwnedMutexGuard<()>> {
    quiesce_within(state, session, QUIESCE_DEADLINE).await
}

/// Cancel until the turn lock frees. Past `deadline` the runner is killed,
/// and the turn gets one more `deadline` to unwind before this gives up.
pub(crate) async fn quiesce_within(
    state: &ServerState,
    session: &str,
    deadline: std::time::Duration,
) -> Result<tokio::sync::OwnedMutexGuard<()>> {
    state.queued_chats.lock().await.retain(|_, items| {
        items.retain(|item| item.session_id != session);
        !items.is_empty()
    });
    let turn = state.manager.session_turn(session);
    let started = std::time::Instant::now();
    let mut killed = false;
    let mut tick = runtime::cancel_tick();
    loop {
        if let Ok(guard) = turn.clone().try_lock_owned() {
            // An idle runner holds no turn, but it holds the conversation.
            end_runner(state, session, false).await;
            return Ok(guard);
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
        let waited = started.elapsed();
        if !killed && waited >= deadline {
            killed = true;
            tracing::warn!("Claude turn in {session} did not stop in time; killing its runner");
            state.claude.kill_runner(session);
        }
        if waited >= deadline * 2 {
            bail!("The Claude turn is still stopping; try again in a moment.");
        }
        tick.tick().await;
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
        .claude_default_model
        .iter()
        .chain(config.routing.default_models.iter())
        .chain(config.models.iter().map(|m| &m.id))
        .find_map(|id| available.iter().find(|m| m.id == *id && supports_model(m)))
        .cloned()
        .cloned()
        .context("Add a Claude API-key model in Settings → Models.")
}

pub(crate) fn new_meta(title: String, cwd: String, parent: Option<String>) -> SessionMeta {
    let mut meta = SessionMeta {
        id: format!(
            "sess-{}-{}",
            crate::util::now_ts_secs(),
            &uuid::Uuid::new_v4().to_string()[..8]
        ),
        title,
        created_at: crate::util::now_ts_secs(),
        runtime: SessionRuntime::ClaudeAgentSdk,
        agents: vec![SessionMember::new(AGENT_ID)],
        parent_session_id: parent,
        creator: "user".into(),
        title_locked: false,
        ..Default::default()
    };
    meta.set_cwd(&cwd);
    meta
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
    // Take the turn's place before spawning: two simultaneous first messages
    // cannot both appear idle, and queued ones run in the order they came.
    let (busy, mut lock) = chat::helpers::take_turn_place(state.manager.session_turn(&meta.id));
    let queued = chat::enqueue_if_busy(
        &state,
        busy,
        &meta.cwd,
        &meta.id,
        AGENT_ID,
        &req.message,
        (&req.message, &req.images),
        // A follow-up waits for the running turn, as Claude Code queues it;
        // only Stop cancels.
        crate::engine::skill::QueueMode::AfterTurn,
    )
    .await;
    if !busy {
        persist_user(&state, &meta, &req).await;
    }
    let sid = meta.id.clone();
    let queued_id = queued.as_ref().map(|item| item.id.clone());
    tokio::spawn(async move {
        let mut steer = queued.as_ref().map(|item| state.claude.await_steer(&item.id));
        let steered = match steer.as_mut() {
            Some(rx) => tokio::select! {
                guard = &mut lock => Err(guard),
                result = rx => Ok(result.is_ok()),
            },
            None => Err(lock.as_mut().await),
        };
        let _guard = match steered {
            Err(guard) => guard,
            // The steer channel closed unsent: the message keeps its place.
            Ok(false) => lock.as_mut().await,
            Ok(true) => {
                let item = queued.as_ref().expect("a steer is only armed for a queued message");
                if !dequeue_user(&state, &req, &meta, &item.id).await {
                    return;
                }
                if steer_into_turn(&state, &meta.id, &req).await {
                    return;
                }
                // The turn ended as the person pressed Steer: the message,
                // already shown, starts the next turn instead.
                let guard = lock.as_mut().await;
                execute_turn(state, req, meta).await;
                drop(guard);
                return;
            }
        };
        if let Some(item) = queued {
            state.claude.forget_steer(&item.id);
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
    Json(json!({"status": if busy {"queued"} else {"started"}, "session_id": sid, "agent_id": AGENT_ID, "queued_id": queued_id})).into_response()
}

/// Stop one SDK task of a live runner: a background shell, watch or subagent.
pub(crate) async fn stop_task(state: &ServerState, session: &str, task_id: &str) -> Result<()> {
    let sender = state
        .claude
        .controls
        .lock()
        .await
        .get(session)
        .cloned()
        .context("Claude task is no longer running")?;
    let (reply, response) = tokio::sync::oneshot::channel();
    sender
        .send(Control::StopTask { task_id: task_id.into(), reply })
        .map_err(|_| anyhow::anyhow!("Claude task ended"))?;
    tokio::time::timeout(std::time::Duration::from_secs(20), response)
        .await??
        .map_err(anyhow::Error::msg)
}

/// Take a queued message off the queue and show it in the chat; false when
/// the person removed it meanwhile.
async fn dequeue_user(
    state: &Arc<ServerState>,
    req: &chat::types::ChatRequest,
    meta: &SessionMeta,
    queued_id: &str,
) -> bool {
    chat::dequeue_and_emit(
        state,
        &state.events_tx,
        queued_id,
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
}

/// Hand a message to the running turn at priority `now`; false when no turn
/// is running or it has already finished taking messages.
async fn steer_into_turn(state: &ServerState, session: &str, req: &chat::types::ChatRequest) -> bool {
    let Some(sender) = state.claude.controls.lock().await.get(session).cloned() else {
        return false;
    };
    let (reply, response) = tokio::sync::oneshot::channel();
    let control = Control::Steer { text: req.message.clone(), images: media::data_urls(&req.images), reply };
    if sender.send(control).is_err() {
        return false;
    }
    matches!(
        tokio::time::timeout(std::time::Duration::from_secs(10), response).await,
        Ok(Ok(Ok(())))
    )
}

#[derive(serde::Deserialize)]
pub(crate) struct SteerRequest {
    id: String,
}

/// POST /api/queue/steer — send one queued message into the running turn.
pub(crate) async fn steer_queued(
    axum::extract::State(state): axum::extract::State<Arc<ServerState>>,
    Json(req): Json<SteerRequest>,
) -> Response {
    if state.claude.steer(&req.id) {
        Json(json!({"status": "ok"})).into_response()
    } else {
        (StatusCode::NOT_FOUND, "That message is no longer waiting.").into_response()
    }
}

/// Send one turn to the session's live runner, starting one when there is
/// none (doc/background-tasks-spec.md), and wait for the SDK's answer to it.
async fn deliver(
    state: &Arc<ServerState>,
    host: &runtime::ClaudeSession,
    req: &chat::types::ChatRequest,
) -> std::result::Result<(), String> {
    let turn_memory = host.turn_memory(&req.message).await;
    let chosen = req
        .model_id
        .clone()
        .or_else(|| host.meta.member(AGENT_ID).and_then(|m| m.model.clone()));
    let model = resolve_model(state, chosen.as_deref()).await.map_err(|e| e.to_string())?;
    let trusted = trust::known(state, &host.meta.id, &host.meta.cwd);
    let key = host.settings_key(&model.id, trusted).await;
    for _ in 0..2 {
        let (done, outcome) = tokio::sync::oneshot::channel();
        let turn = protocol::TurnRequest {
            run_id: host.run_id.clone(),
            prompt: req.message.clone(),
            images: media::data_urls(&req.images),
            uuid: uuid::Uuid::new_v4().to_string(),
            turn_memory: turn_memory.clone(),
            done,
        };
        hand(state, &host.meta, turn, &key, chosen.clone()).await;
        match wait_turn(state, host, outcome).await {
            Err(error) if error == protocol::RETRY => continue,
            outcome => return outcome,
        }
    }
    Err("Claude runner kept ending; try again.".into())
}

/// The live runner takes the turn when it runs on these settings (or has
/// background work it must not lose); otherwise it is ended and a fresh one
/// starts on the turn.
async fn hand(
    state: &Arc<ServerState>,
    meta: &SessionMeta,
    mut turn: protocol::TurnRequest,
    key: &str,
    chosen: Option<String>,
) {
    let session = meta.id.clone();
    let mut live = state.claude.live.lock().await;
    if let Some(runner) = live.get(&session) {
        let same = *runner.settings.lock().unwrap_or_else(|p| p.into_inner()) == key;
        let busy = state.tasks.list(&session).iter().any(|t| t.is_running());
        if same || busy {
            match runner.turns.send(turn) {
                Ok(()) => return,
                Err(returned) => turn = returned.0,
            }
            live.remove(&session);
        } else {
            drop(live);
            end_runner(state, &session, true).await;
            live = state.claude.live.lock().await;
        }
    }
    let (turns_tx, turns_rx) = tokio::sync::mpsc::unbounded_channel();
    let (control_tx, control_rx) = tokio::sync::mpsc::unbounded_channel();
    let settings = Arc::new(std::sync::Mutex::new(key.to_owned()));
    let ended = Arc::new(tokio::sync::Notify::new());
    live.insert(
        session.clone(),
        host::Live { turns: turns_tx, settings: settings.clone(), ended: ended.clone() },
    );
    state.claude.controls.lock().await.insert(session.clone(), control_tx.clone());
    drop(live);
    let mut runner = runtime::ClaudeSession {
        state: state.clone(),
        meta: meta.clone(),
        run_id: turn.run_id.clone(),
    };
    let state = state.clone();
    tokio::spawn(async move {
        let result = crate::util::panic_as_error(
            runner.serve(turn, chosen, turns_rx, control_rx, settings),
        )
        .await;
        if let Err(error) = result {
            tracing::info!("Claude runner for {session} ended: {error}");
        }
        let mut live = state.claude.live.lock().await;
        if live.get(&session).is_some_and(|l| Arc::ptr_eq(&l.ended, &ended)) {
            live.remove(&session);
        }
        drop(live);
        let mut controls = state.claude.controls.lock().await;
        if controls.get(&session).is_some_and(|c| c.same_channel(&control_tx)) {
            controls.remove(&session);
        }
        drop(controls);
        ended.notify_waiters();
    });
}

/// Wait for the turn's answer. Stop interrupts the SDK's turn, keeping the
/// runner and its background tasks; a runner that does not answer the
/// interrupt in time is killed.
async fn wait_turn(
    state: &ServerState,
    host: &runtime::ClaudeSession,
    outcome: tokio::sync::oneshot::Receiver<std::result::Result<(), String>>,
) -> std::result::Result<(), String> {
    tokio::pin!(outcome);
    let mut interrupted: Option<std::time::Instant> = None;
    let mut killed = false;
    let mut tick = runtime::cancel_tick();
    loop {
        tokio::select! {
            answer = &mut outcome => {
                return answer.unwrap_or_else(|_| Err("Claude runner ended".into()));
            }
            _ = tick.tick() => {
                if interrupted.is_none() && host.cancelled().await {
                    interrupted = Some(std::time::Instant::now());
                    if let Some(sender) = state.claude.controls.lock().await.get(&host.meta.id) {
                        let (reply, _ack) = tokio::sync::oneshot::channel();
                        let _ = sender.send(Control::Interrupt { reply });
                    }
                }
                if !killed && interrupted.is_some_and(|at| at.elapsed() >= QUIESCE_DEADLINE) {
                    killed = true;
                    state.claude.kill_runner(&host.meta.id);
                }
            }
        }
    }
}

/// End the session's live runner: `gently` asks it to exit once idle, else
/// it is killed. Returns once it is gone (or a deadline passed, then killed).
pub(crate) async fn end_runner(state: &ServerState, session: &str, gently: bool) {
    let gone = || async { !state.claude.live.lock().await.contains_key(session) };
    if gone().await {
        return;
    }
    if gently {
        if let Some(sender) = state.claude.controls.lock().await.get(session) {
            let _ = sender.send(Control::End);
        }
    } else {
        state.claude.kill_runner(session);
    }
    let started = std::time::Instant::now();
    let mut tick = runtime::cancel_tick();
    while !gone().await {
        if started.elapsed() >= QUIESCE_DEADLINE {
            state.claude.kill_runner(session);
            if started.elapsed() >= QUIESCE_DEADLINE * 2 {
                tracing::warn!("Claude runner for {session} did not exit");
                return;
            }
        }
        tick.tick().await;
    }
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
    let notify_parent = req
        .sender
        .as_deref()
        .is_some_and(|sender| sender != "user" && sender != "system");
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
    let host = runtime::ClaudeSession {
        state: state.clone(),
        meta,
        run_id: run_id.clone(),
    };
    host.emit(crate::engine::agent::AgentEvent::AgentStatus {
        agent_id: AGENT_ID.into(),
        status: "thinking".into(),
        detail: Some("Thinking".into()),
        parent_id: None,
        run_id: None,
        parent_run_id: None,
    })
    .await;
    let result = deliver(&state, &host, &req).await.map_err(anyhow::Error::msg);
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
            &session_status(&host.meta, result),
            Some(parent),
            false,
            None,
        )
        .await;
    }
}

/// A linked session's outcome as a fact the parent chat renders as a card
/// and its agent can read; the words are never the engine's.
pub(crate) fn session_status(meta: &SessionMeta, status: &str) -> String {
    json!({"type":"session_status", "session_id":meta.id, "title":meta.title,
        "runtime":"claude", "status":status})
    .to_string()
}
