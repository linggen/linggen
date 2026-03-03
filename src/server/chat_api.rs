use crate::agent_manager::AgentManager;
use crate::server::chat_helpers::{
    emit_outcome_event, emit_queue_updated, persist_and_emit_message,
    persist_message_only, queue_key, queue_preview,
};
use crate::server::{AgentStatusKind, QueuedChatItem, ServerEvent, ServerState};
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Deserialize)]
pub(crate) struct ChatRequest {
    project_root: String,
    agent_id: String,
    message: String,
    session_id: Option<String>,
    mode: Option<String>,
    #[serde(default)]
    images: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct PlanActionRequest {
    project_root: String,
    agent_id: String,
    session_id: Option<String>,
    clear_context: Option<bool>,
}

#[derive(Deserialize)]
pub(crate) struct EditPlanRequest {
    project_root: String,
    agent_id: String,
    text: String,
}

#[derive(Deserialize)]
pub(crate) struct ClearChatRequest {
    project_root: String,
    session_id: Option<String>,
}

fn parse_explicit_target_prefix(message: &str) -> Option<(&str, &str)> {
    let rest = message.strip_prefix('@')?;
    let space_idx = rest.find(' ')?;
    let candidate = rest[..space_idx].trim();
    let body = rest[space_idx + 1..].trim_start();
    if candidate.is_empty() {
        return None;
    }
    if !candidate
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    Some((candidate, body))
}


async fn run_loop_with_tracking(
    manager: &Arc<crate::agent_manager::AgentManager>,
    root: &PathBuf,
    engine: &mut crate::engine::AgentEngine,
    agent_id: &str,
    session_id: Option<&str>,
    detail: &str,
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
                    .finish_agent_run(&run_id, crate::project_store::AgentRunStatus::Completed, None)
                    .await;
            }
            Err(err) => {
                let msg = err.to_string();
                let status = if msg.to_lowercase().contains("cancel") {
                    crate::project_store::AgentRunStatus::Cancelled
                } else {
                    tracing::error!("Agent loop failed: {}", msg);
                    crate::project_store::AgentRunStatus::Failed
                };
                let _ = manager.finish_agent_run(&run_id, status, Some(msg)).await;
            }
        }
    }

    result
}

pub(crate) async fn clear_chat_history_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ClearChatRequest>,
) -> impl IntoResponse {
    let session_id = req
        .session_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let root = match PathBuf::from(&req.project_root).canonicalize() {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    match state.manager.get_or_create_project(root).await {
        Ok(ctx) => match ctx.sessions.clear_chat_history(&session_id) {
            Ok(removed) => {
                // Also clear in-memory chat history for all agents in this project/session
                let agents = ctx.agents.lock().await;
                for agent_mutex in agents.values() {
                    let mut agent = agent_mutex.lock().await;
                    agent.chat_history.clear();
                    agent.observations.clear();
                }
                drop(agents);

                let _ = state.events_tx.send(ServerEvent::StateUpdated);
                Json(serde_json::json!({ "removed": removed })).into_response()
            }
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Compute the per-session plan directory from the project store.
fn compute_session_plan_dir(
    store: &crate::project_store::ProjectStore,
    root: &Path,
    session_id: Option<&str>,
) -> std::path::PathBuf {
    let root_str = root.to_string_lossy().to_string();
    store
        .project_dir(&root_str)
        .join("sessions")
        .join(session_id.unwrap_or("default"))
}

/// Shared context for the async chat dispatch functions.
struct ChatRunCtx {
    state: Arc<ServerState>,
    manager: Arc<AgentManager>,
    events_tx: broadcast::Sender<ServerEvent>,
    root: PathBuf,
    agent_id: String,
    session_id: Option<String>,
    clean_msg: String,
    images: Vec<String>,
}

/// Dispatch a skill (slash command) invocation.
async fn run_skill_dispatch(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
) {
    let parts: Vec<&str> = ctx.clean_msg.trim().splitn(2, ' ').collect();
    let cmd = parts[0].trim_start_matches('/');
    let user_args = parts
        .get(1)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // Resolve the skill.
    if let Some(manager) = engine.tools.get_manager() {
        if let Some(skill) = manager.skill_manager.get_skill(cmd).await {
            if !skill.user_invocable {
                let err_msg = format!(
                    "Skill '{}' is not user-invocable and cannot be activated with /{cmd}.",
                    skill.name
                );
                persist_and_emit_message(
                    &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                    &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
                )
                .await;
                return;
            }
            // If no arguments given and the skill has a usage hint, respond immediately
            // with a subcommand list instead of spinning up the agent loop.
            if user_args.is_none() {
                if let Some(hint) = &skill.argument_hint {
                    let subcommands: Vec<String> = hint
                        .split('|')
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .map(|s| format!("  `/{} {}`", skill.name, s))
                        .collect();
                    let usage_msg = format!(
                        "{}\n\n**Commands:**\n{}",
                        skill.description,
                        subcommands.join("\n"),
                    );
                    persist_and_emit_message(
                        &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                        &ctx.agent_id, "user", &usage_msg, ctx.session_id.as_deref(), false,
                    )
                    .await;
                    return;
                }
            }
            engine.active_skill = Some(skill);
        }
    }

    let task_for_loop = user_args
        .unwrap_or_else(|| "Initialize this workspace and summarize status.".to_string());

    engine.observations.clear();
    engine.task = Some(task_for_loop);

    // Add user message to chat history so subsequent turns have conversational context.
    engine.chat_history.push(crate::ollama::ChatMessage::new("user", ctx.clean_msg.clone()));
    engine.truncate_chat_history();

    let skill_msg = format!("Running skill: {}", cmd);
    persist_and_emit_message(
        &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
        &ctx.agent_id, "user", &skill_msg, ctx.session_id.as_deref(), false,
    )
    .await;

    let interrupt_key = wire_interrupt_channel(ctx, engine).await;
    wire_ask_user_bridge(&ctx.state, engine);

    let outcome = run_loop_with_tracking(
        &ctx.manager, &ctx.root, engine, &ctx.agent_id,
        ctx.session_id.as_deref(), "chat:skill",
    )
    .await;

    unwire_interrupt_channel(ctx, engine, &interrupt_key).await;

    if let Err(e) = outcome {
        tracing::warn!("Skill loop failed: {}", e);
        let err_msg = format!("Error: {}", e);
        persist_and_emit_message(
            &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
            &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
        )
        .await;
    } else {
        if let Ok(outcome) = &outcome {
            emit_outcome_event(outcome, &ctx.events_tx, &ctx.agent_id);
        }
        let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
    }
}

/// Dispatch a user-defined trigger activation.
/// Similar to `run_skill_dispatch` but takes a pre-resolved skill name and remaining input.
async fn run_trigger_dispatch(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
    skill_name: &str,
    remaining: &str,
) {
    let user_args = if remaining.is_empty() {
        None
    } else {
        Some(remaining.to_string())
    };

    let mut skill_default_task: Option<String> = None;
    if let Some(manager) = engine.tools.get_manager() {
        if let Some(skill) = manager.skill_manager.get_skill(skill_name).await {
            if !skill.user_invocable {
                let err_msg = format!(
                    "Skill '{}' is not user-invocable and cannot be activated via trigger.",
                    skill.name
                );
                persist_and_emit_message(
                    &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                    &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
                )
                .await;
                return;
            }
            if user_args.is_none() {
                skill_default_task =
                    Some(format!("Run the '{}' skill: {}", skill.name, skill.description));
            }
            engine.active_skill = Some(skill);
        }
    }

    let task_for_loop = user_args
        .or(skill_default_task)
        .unwrap_or_else(|| "Initialize this workspace and summarize status.".to_string());

    engine.observations.clear();
    engine.task = Some(task_for_loop);

    // Add user message to chat history so subsequent turns have conversational context.
    engine.chat_history.push(crate::ollama::ChatMessage::new("user", ctx.clean_msg.clone()));
    engine.truncate_chat_history();

    let skill_msg = format!("Running skill via trigger: {}", skill_name);
    persist_and_emit_message(
        &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
        &ctx.agent_id, "user", &skill_msg, ctx.session_id.as_deref(), false,
    )
    .await;

    let interrupt_key = wire_interrupt_channel(ctx, engine).await;
    wire_ask_user_bridge(&ctx.state, engine);

    let outcome = run_loop_with_tracking(
        &ctx.manager, &ctx.root, engine, &ctx.agent_id,
        ctx.session_id.as_deref(), "chat:trigger",
    )
    .await;

    unwire_interrupt_channel(ctx, engine, &interrupt_key).await;

    if let Err(e) = outcome {
        tracing::warn!("Trigger skill loop failed: {}", e);
        let err_msg = format!("Error: {}", e);
        persist_and_emit_message(
            &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
            &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
        )
        .await;
    } else {
        if let Ok(outcome) = &outcome {
            emit_outcome_event(outcome, &ctx.events_tx, &ctx.agent_id);
        }
        let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
    }
}

/// Dispatch plan mode: agent researches codebase and produces a structured plan (read-only).
async fn run_plan_dispatch(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
) {
    ctx.state
        .send_agent_status(
            ctx.agent_id.clone(),
            AgentStatusKind::Thinking,
            Some("Planning".to_string()),
            None,
        )
        .await;

    // Extract task from "/plan <task>" prefix or use full message.
    let task_text = ctx
        .clean_msg
        .strip_prefix("/plan ")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ctx.clean_msg.trim());

    engine.plan_mode = true;
    engine.plan = None;
    engine.observations.clear();
    engine.task = Some(task_text.to_string());
    engine.session_plan_dir = Some(compute_session_plan_dir(
        &ctx.state.manager.store,
        &ctx.root,
        ctx.session_id.as_deref(),
    ));

    // Add user message to chat history so subsequent turns have conversational context.
    engine.chat_history.push(crate::ollama::ChatMessage::new("user", ctx.clean_msg.clone()));
    engine.truncate_chat_history();

    // Wire up the thinking channel.
    let (thinking_tx, mut thinking_rx) = tokio::sync::mpsc::unbounded_channel();
    engine.thinking_tx = Some(thinking_tx);

    // Wire up the interrupt channel so user messages reach the running loop.
    let interrupt_key = wire_interrupt_channel(ctx, engine).await;
    wire_ask_user_bridge(&ctx.state, engine);

    let events_tx_clone = ctx.events_tx.clone();
    let agent_id_clone = ctx.agent_id.clone();
    tokio::spawn(async move {
        while let Some(event) = thinking_rx.recv().await {
            match event {
                crate::engine::ThinkingEvent::Token(token) => {
                    let _ = events_tx_clone.send(ServerEvent::Token {
                        agent_id: agent_id_clone.clone(),
                        token,
                        done: false,
                        thinking: true,
                    });
                }
                crate::engine::ThinkingEvent::ContentToken(token) => {
                    let _ = events_tx_clone.send(ServerEvent::Token {
                        agent_id: agent_id_clone.clone(),
                        token,
                        done: false,
                        thinking: false,
                    });
                }
                crate::engine::ThinkingEvent::Done => {
                    let _ = events_tx_clone.send(ServerEvent::Token {
                        agent_id: agent_id_clone.clone(),
                        token: String::new(),
                        done: true,
                        thinking: true,
                    });
                }
                crate::engine::ThinkingEvent::ContentDone => {
                    let _ = events_tx_clone.send(ServerEvent::Token {
                        agent_id: agent_id_clone.clone(),
                        token: String::new(),
                        done: true,
                        thinking: false,
                    });
                }
            }
        }
    });

    let outcome = run_loop_with_tracking(
        &ctx.manager,
        &ctx.root,
        engine,
        &ctx.agent_id,
        ctx.session_id.as_deref(),
        "chat:plan",
    )
    .await;

    engine.thinking_tx = None;
    unwire_interrupt_channel(ctx, engine, &interrupt_key).await;
    engine.plan_mode = false;

    match outcome {
        Ok(ref outcome) => {
            emit_outcome_event(outcome, &ctx.events_tx, &ctx.agent_id);
            if let crate::engine::AgentOutcome::Plan(ref plan) = outcome {
                // Always wait for explicit user approval before execution.
                ctx.manager
                    .set_pending_plan(
                        &ctx.root.to_string_lossy(),
                        &ctx.agent_id,
                        plan.clone(),
                    )
                    .await;
            }
        }
        Err(err) => {
            let error_msg = format!("Error: {}", err);
            persist_and_emit_message(
                &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                &ctx.agent_id, "user", &error_msg, ctx.session_id.as_deref(), false,
            )
            .await;
        }
    }
    let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
}

/// Wire the interrupt channel into the engine and store the sender in ServerState.
/// Returns the interrupt_key used to look up the sender later for cleanup.
async fn wire_interrupt_channel(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
) -> String {
    let (interrupt_tx, interrupt_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    engine.interrupt_rx = Some(interrupt_rx);

    let interrupt_key = queue_key(
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
async fn unwire_interrupt_channel(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
    interrupt_key: &str,
) {
    engine.interrupt_rx = None;
    let mut guard = ctx.state.interrupt_tx.lock().await;
    guard.remove(interrupt_key);
}

/// Wire the AskUser bridge so the tool can emit SSE events and block on user response.
fn wire_ask_user_bridge(
    state: &Arc<ServerState>,
    engine: &mut crate::engine::AgentEngine,
) {
    let bridge = Arc::new(crate::engine::tools::AskUserBridge {
        events_tx: state.events_tx.clone(),
        pending: state.pending_ask_user.clone(),
    });
    engine.tools.set_ask_user_bridge(bridge);
}

/// Dispatch the structured (auto) mode agent loop.
async fn run_structured_loop(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
) {
    // Vision gate: reject images if the model doesn't support vision.
    if !ctx.images.is_empty() {
        let has_vision = engine
            .model_manager
            .has_vision(&engine.model_id)
            .await
            .unwrap_or(false);
        if !has_vision {
            let err_msg = format!(
                "Model `{}` does not support vision/image input. Please use a vision-capable model (e.g. qwen3-vl, llava, llama3.2-vision).",
                engine.model_id
            );
            persist_and_emit_message(
                &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
            )
            .await;
            let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
            return;
        }
        engine.pending_images = ctx.images.clone();
    }

    ctx.state
        .send_agent_status(ctx.agent_id.clone(), AgentStatusKind::Thinking, Some("Thinking".to_string()), None)
        .await;
    engine.observations.clear();
    // Clear stale "planned" plan from a previous plan-mode run so it doesn't
    // block execution of the new structured loop.
    if let Some(p) = &engine.plan {
        if p.status == crate::engine::PlanStatus::Planned {
            engine.plan = None;
        }
    }
    let task_for_loop = ctx.clean_msg.trim().to_string();
    engine.task = Some(task_for_loop);
    engine.session_plan_dir = Some(compute_session_plan_dir(
        &ctx.state.manager.store,
        &ctx.root,
        ctx.session_id.as_deref(),
    ));

    // Add user message to chat history so subsequent turns have conversational context.
    engine.chat_history.push(crate::ollama::ChatMessage::new("user", ctx.clean_msg.clone()));
    engine.truncate_chat_history();

    // Wire up the thinking channel so streaming thinking tokens reach the UI.
    let (thinking_tx, mut thinking_rx) = tokio::sync::mpsc::unbounded_channel();
    engine.thinking_tx = Some(thinking_tx);

    // Wire up the interrupt channel so user messages reach the running loop.
    let interrupt_key = wire_interrupt_channel(ctx, engine).await;
    wire_ask_user_bridge(&ctx.state, engine);

    let events_tx_clone = ctx.events_tx.clone();
    let agent_id_clone = ctx.agent_id.clone();
    tokio::spawn(async move {
        while let Some(event) = thinking_rx.recv().await {
            match event {
                crate::engine::ThinkingEvent::Token(token) => {
                    let _ = events_tx_clone.send(ServerEvent::Token {
                        agent_id: agent_id_clone.clone(),
                        token,
                        done: false,
                        thinking: true,
                    });
                }
                crate::engine::ThinkingEvent::ContentToken(token) => {
                    let _ = events_tx_clone.send(ServerEvent::Token {
                        agent_id: agent_id_clone.clone(),
                        token,
                        done: false,
                        thinking: false,
                    });
                }
                crate::engine::ThinkingEvent::Done => {
                    let _ = events_tx_clone.send(ServerEvent::Token {
                        agent_id: agent_id_clone.clone(),
                        token: String::new(),
                        done: true,
                        thinking: true,
                    });
                }
                crate::engine::ThinkingEvent::ContentDone => {
                    let _ = events_tx_clone.send(ServerEvent::Token {
                        agent_id: agent_id_clone.clone(),
                        token: String::new(),
                        done: true,
                        thinking: false,
                    });
                }
            }
        }
    });

    let outcome = run_loop_with_tracking(
        &ctx.manager, &ctx.root, engine, &ctx.agent_id,
        ctx.session_id.as_deref(), "chat:structured-loop",
    )
    .await;

    // Drop the thinking sender so the forwarder task exits.
    engine.thinking_tx = None;
    unwire_interrupt_channel(ctx, engine, &interrupt_key).await;

    // Agent requested plan mode — re-dispatch using existing plan machinery.
    if let Ok(crate::engine::AgentOutcome::PlanModeRequested { ref reason }) = outcome {
        let plan_task = reason.clone().unwrap_or_else(|| ctx.clean_msg.clone());
        engine.task = Some(plan_task);
        run_plan_dispatch(ctx, engine).await;
        return;
    }

    // Agent created a plan that needs approval — store as pending.
    if let Ok(ref ok_outcome @ crate::engine::AgentOutcome::Plan(ref plan)) = outcome {
        emit_outcome_event(ok_outcome, &ctx.events_tx, &ctx.agent_id);
        ctx.manager
            .set_pending_plan(
                &ctx.root.to_string_lossy(),
                &ctx.agent_id,
                plan.clone(),
            )
            .await;
        let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
        return;
    }

    if let Ok(outcome) = &outcome {
        emit_outcome_event(outcome, &ctx.events_tx, &ctx.agent_id);
        // For text-only responses (AgentOutcome::None with streamed content),
        // the engine persists the message to session files but doesn't emit a Message event.
        // Emit one now so the UI can finalize liveText → text.
        if matches!(outcome, crate::engine::AgentOutcome::None) {
            if let Some(text) = &engine.last_assistant_text {
                if !text.is_empty() {
                    let _ = ctx.events_tx.send(ServerEvent::Message {
                        from: ctx.agent_id.clone(),
                        to: "user".to_string(),
                        content: text.clone(),
                    });
                }
            }
        }
    } else if let Err(err) = outcome {
        let error_msg = format!("Error: {}", err);
        persist_and_emit_message(
            &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
            &ctx.agent_id, "user", &error_msg, ctx.session_id.as_deref(), false,
        )
        .await;
    }
    let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
}

/// Generate a session title from the first few words of the user's message.
fn auto_session_title(message: &str) -> String {
    let words: Vec<&str> = message.split_whitespace().collect();
    if words.is_empty() {
        return "New Chat".to_string();
    }
    let first: String = words.iter().take(6).copied().collect::<Vec<_>>().join(" ");
    if first.chars().count() > 50 {
        let s: String = first.chars().take(47).collect();
        format!("{}...", s.trim_end())
    } else if words.len() > 6 {
        format!("{first}...")
    } else {
        first
    }
}

pub(crate) async fn chat_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);
    let project_root_str = root.to_string_lossy().to_string();

    // Auto-create a new session when none is provided, instead of falling
    // back to a hidden "default" session.
    let session_id: Option<String> = if req.session_id.is_some() {
        req.session_id.clone()
    } else {
        let now = crate::util::now_ts_secs();
        let new_id = format!("sess-{}-{}", now, &uuid::Uuid::new_v4().to_string()[..8]);
        if let Ok(ctx) = state.manager.get_or_create_project(root.clone()).await {
            let meta = crate::state_fs::sessions::SessionMeta {
                id: new_id.clone(),
                title: auto_session_title(&req.message),
                created_at: now,
            };
            let _ = ctx.sessions.add_session(&meta);
        }
        Some(new_id)
    };
    let effective_session_id = session_id.clone().unwrap_or_else(|| "default".to_string());
    let events_tx = state.events_tx.clone();

    // Optional explicit target prefix: "@agent_id <message>".
    // Only reroute when the candidate agent exists in this project.
    let (target_id, clean_msg) =
        if let Some((candidate, body)) = parse_explicit_target_prefix(&req.message) {
            let candidate_id = candidate.to_string();
            if state
                .manager
                .agent_exists(&root, &candidate_id)
                .await
            {
                (candidate_id, body.to_string())
            } else {
                (req.agent_id.clone(), req.message.clone())
            }
        } else {
            (req.agent_id.clone(), req.message.clone())
        };

    match state.manager.get_or_create_agent(&root, &target_id).await {
        Ok(agent) => {
            let was_busy = agent.try_lock().is_err();
            let queued_item = if was_busy {
                Some(QueuedChatItem {
                    id: format!(
                        "{}-{}",
                        crate::util::now_ts_ms(),
                        state.queue_seq.fetch_add(1, Ordering::Relaxed)
                    ),
                    agent_id: target_id.clone(),
                    session_id: effective_session_id.clone(),
                    preview: queue_preview(&clean_msg),
                    timestamp: crate::util::now_ts_secs(),
                })
            } else {
                None
            };
            if let Some(item) = &queued_item {
                let key = queue_key(&project_root_str, &effective_session_id, &target_id);
                {
                    let mut guard = state.queued_chats.lock().await;
                    guard.entry(key).or_default().push(item.clone());
                }
                emit_queue_updated(&state, &project_root_str, &effective_session_id, &target_id)
                    .await;

                // Cancel any pending AskUser for this agent so the tool
                // unblocks immediately and the loop can pick up the new message.
                {
                    let mut pending = state.pending_ask_user.lock().await;
                    pending.retain(|_, entry| entry.agent_id != target_id);
                }

                // Send through interrupt channel so the running loop sees the message.
                {
                    let interrupt_guard = state.interrupt_tx.lock().await;
                    let ikey = queue_key(&project_root_str, &effective_session_id, &target_id);
                    if let Some(tx) = interrupt_guard.get(&ikey) {
                        let _ = tx.send(clean_msg.clone());
                    }
                }
            }

            let session_id_response = session_id.clone(); // for the HTTP response
            let events_tx_clone = events_tx.clone();
            let target_id_clone = target_id.clone();
            let clean_msg_clone = clean_msg.clone();
            let root_clone = root.clone();
            let manager = state.manager.clone();
            let state_clone = state.clone();
            let queued_item_id = queued_item.as_ref().map(|q| q.id.clone());
            let session_id_for_queue = effective_session_id.clone();
            let project_root_for_queue = project_root_str.clone();
            let req_mode = req.mode.clone();
            let req_images = req.images.clone();

            if was_busy {
                // When queued, only persist to disk — don't emit via SSE.
                // The frontend shows the message in the queue banner and removes
                // the optimistic chat bubble. Emitting here would re-add it,
                // causing duplication.
                persist_message_only(
                    &state.manager,
                    &root,
                    &target_id,
                    "user",
                    &target_id,
                    &clean_msg,
                    session_id.as_deref(),
                    false,
                )
                .await;
            } else {
                // Not queued — persist and emit so the UI shows the message.
                persist_and_emit_message(
                    &state.manager,
                    &events_tx,
                    &root,
                    &target_id,
                    "user",
                    &target_id,
                    &clean_msg,
                    session_id.as_deref(),
                    false,
                )
                .await;
            }

            tokio::spawn(async move {
                let mut engine = agent.lock().await;
                if let Some(queued_id) = queued_item_id.as_deref() {
                    let key = queue_key(
                        &project_root_for_queue,
                        &session_id_for_queue,
                        &target_id_clone,
                    );
                    {
                        let mut guard = state_clone.queued_chats.lock().await;
                        if let Some(items) = guard.get_mut(&key) {
                            items.retain(|item| item.id != queued_id);
                            if items.is_empty() {
                                guard.remove(&key);
                            }
                        }
                    }
                    emit_queue_updated(
                        &state_clone,
                        &project_root_for_queue,
                        &session_id_for_queue,
                        &target_id_clone,
                    )
                    .await;
                }

                let model_label = &engine.model_id;
                state_clone
                    .send_agent_status(
                        target_id_clone.clone(), AgentStatusKind::ModelLoading,
                        Some(format!("Loading model: {model_label}")),
                        None,
                    )
                    .await;
                let ctx = ChatRunCtx {
                    state: state_clone.clone(),
                    manager: manager.clone(),
                    events_tx: events_tx_clone.clone(),
                    root: root_clone,
                    agent_id: target_id_clone.clone(),
                    session_id: session_id.clone(),
                    clean_msg: clean_msg_clone.clone(),
                    images: req_images,
                };

                // Restore chat history from session store when the engine was
                // freshly created (e.g. after model change invalidated cache).
                if engine.chat_history.is_empty() {
                    if let Ok(proj_ctx) = ctx.manager.get_or_create_project(ctx.root.clone()).await {
                        let sid = ctx.session_id.as_deref().unwrap_or("default");
                        if let Ok(msgs) = proj_ctx.sessions.get_chat_history(sid, Some(&ctx.agent_id)) {
                            for m in &msgs {
                                if m.is_observation || m.from_id == "system" {
                                    continue;
                                }
                                let role = if m.from_id == "user" {
                                    "user"
                                } else if m.from_id == ctx.agent_id {
                                    "assistant"
                                } else {
                                    continue;
                                };
                                engine.chat_history.push(
                                    crate::ollama::ChatMessage::new(role, &m.content),
                                );
                            }
                            engine.truncate_chat_history();
                            if !engine.chat_history.is_empty() {
                                tracing::info!(
                                    "Restored {} chat_history messages from session store",
                                    engine.chat_history.len()
                                );
                            }
                        }
                    }
                }

                let is_plan_mode = req_mode.as_deref() == Some("plan")
                    || clean_msg_clone.trim_start().starts_with("/plan ");

                if is_plan_mode {
                    // 0. Plan mode dispatch
                    run_plan_dispatch(&ctx, &mut engine).await;
                } else if clean_msg_clone.trim_start().starts_with('/') {
                    // 1. Slash-command skill dispatch
                    run_skill_dispatch(&ctx, &mut engine).await;
                } else if let Some((skill_name, remaining)) =
                    manager.skill_manager.match_trigger(&clean_msg_clone).await
                {
                    // 2. User-defined trigger prefix match
                    run_trigger_dispatch(&ctx, &mut engine, &skill_name, &remaining).await;
                } else {
                    // 3. Structured agent loop (always)
                    run_structured_loop(&ctx, &mut engine).await;
                }

                // Emit TurnComplete so the Web UI has a single finalizer.
                let _ = state_clone.events_tx.send(ServerEvent::TurnComplete {
                    agent_id: target_id_clone.clone(),
                    duration_ms: None,
                    context_tokens: None,
                    parent_id: None,
                });
                state_clone
                    .send_agent_status(
                        target_id_clone.clone(), AgentStatusKind::Idle,
                        Some("Idle".to_string()),
                        None,
                    )
                    .await;
            });

            let status = if was_busy { "queued" } else { "started" };
            Json(serde_json::json!({ "status": status, "session_id": session_id_response })).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

pub(crate) async fn approve_plan_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<PlanActionRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);
    let root_str = root.to_string_lossy().to_string();
    let session_id = req.session_id.clone();
    let clear_context = req.clear_context.unwrap_or(false);

    let plan = state
        .manager
        .take_pending_plan(&root_str, &req.agent_id)
        .await;
    let Some(mut plan) = plan else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "No pending plan" })),
        )
            .into_response();
    };

    plan.status = crate::engine::PlanStatus::Approved;

    let agent = match state.manager.get_or_create_agent(&root, &req.agent_id).await {
        Ok(a) => a,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Agent not found" })),
            )
                .into_response();
        }
    };

    let events_tx = state.events_tx.clone();
    let manager = state.manager.clone();
    let agent_id = req.agent_id.clone();
    let state_clone = state.clone();

    // Emit approval message.
    persist_and_emit_message(
        &state.manager,
        &events_tx,
        &root,
        &agent_id,
        "user",
        &agent_id,
        "Plan approved. Starting execution.",
        session_id.as_deref(),
        false,
    )
    .await;

    let root_clone = root.clone();
    let plan_dir = compute_session_plan_dir(
        &state.manager.store,
        &root,
        session_id.as_deref(),
    );

    tokio::spawn(async move {
        let mut engine = agent.lock().await;

        state_clone
            .send_agent_status(
                agent_id.clone(),
                AgentStatusKind::Thinking,
                Some("Executing plan".to_string()),
                None,
            )
            .await;

        // Set the approved plan on the engine.
        engine.plan = Some(plan);
        engine.plan_mode = false;
        engine.session_plan_dir = Some(plan_dir);
        if clear_context {
            // Full context clear — plan file is the sole source of truth
            engine.observations.clear();
            engine.context_records.clear();
            engine.next_context_id = 1;
            engine.chat_history.clear();
            engine.task = Some(format!(
                "Execute the approved plan: {}",
                engine.plan.as_ref().map(|p| p.summary.as_str()).unwrap_or("Plan")
            ));
        } else {
            // Keep context, just clear observations
            engine.observations.clear();
            if engine.task.is_none() {
                if let Some(ref p) = engine.plan {
                    engine.task = Some(format!("Execute the approved plan: {}", p.summary));
                }
            }
        }

        // Wire up thinking channel.
        let (thinking_tx, mut thinking_rx) = tokio::sync::mpsc::unbounded_channel();
        engine.thinking_tx = Some(thinking_tx);

        // Wire up interrupt channel.
        let (interrupt_tx_ch, interrupt_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        engine.interrupt_rx = Some(interrupt_rx);
        let interrupt_key = queue_key(
            &root_clone.to_string_lossy(),
            session_id.as_deref().unwrap_or(""),
            &agent_id,
        );
        {
            let mut guard = state_clone.interrupt_tx.lock().await;
            guard.insert(interrupt_key.clone(), interrupt_tx_ch);
        }

        let events_tx_inner = events_tx.clone();
        let agent_id_inner = agent_id.clone();
        tokio::spawn(async move {
            while let Some(event) = thinking_rx.recv().await {
                match event {
                    crate::engine::ThinkingEvent::Token(token) => {
                        let _ = events_tx_inner.send(ServerEvent::Token {
                            agent_id: agent_id_inner.clone(),
                            token,
                            done: false,
                            thinking: true,
                        });
                    }
                    crate::engine::ThinkingEvent::ContentToken(token) => {
                        let _ = events_tx_inner.send(ServerEvent::Token {
                            agent_id: agent_id_inner.clone(),
                            token,
                            done: false,
                            thinking: false,
                        });
                    }
                    crate::engine::ThinkingEvent::Done => {
                        let _ = events_tx_inner.send(ServerEvent::Token {
                            agent_id: agent_id_inner.clone(),
                            token: String::new(),
                            done: true,
                            thinking: true,
                        });
                    }
                    crate::engine::ThinkingEvent::ContentDone => {
                        let _ = events_tx_inner.send(ServerEvent::Token {
                            agent_id: agent_id_inner.clone(),
                            token: String::new(),
                            done: true,
                            thinking: false,
                        });
                    }
                }
            }
        });

        let outcome = run_loop_with_tracking(
            &manager,
            &root_clone,
            &mut engine,
            &agent_id,
            session_id.as_deref(),
            "chat:plan-execution",
        )
        .await;

        engine.thinking_tx = None;
        engine.interrupt_rx = None;
        {
            let mut guard = state_clone.interrupt_tx.lock().await;
            guard.remove(&interrupt_key);
        }

        if let Ok(ref outcome) = outcome {
            emit_outcome_event(outcome, &events_tx, &agent_id);
        } else if let Err(err) = outcome {
            let error_msg = format!("Error: {}", err);
            persist_and_emit_message(
                &manager, &events_tx, &root_clone, &agent_id,
                &agent_id, "user", &error_msg, session_id.as_deref(), false,
            )
            .await;
        }
        let _ = events_tx.send(ServerEvent::StateUpdated);

        // Emit TurnComplete so the Web UI has a single finalizer.
        let _ = events_tx.send(ServerEvent::TurnComplete {
            agent_id: agent_id.clone(),
            duration_ms: None,
            context_tokens: None,
            parent_id: None,
        });
        state_clone
            .send_agent_status(agent_id.clone(), AgentStatusKind::Idle, Some("Idle".to_string()), None)
            .await;
    });

    Json(serde_json::json!({ "status": "approved" })).into_response()
}

pub(crate) async fn reject_plan_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<PlanActionRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);
    let root_str = root.to_string_lossy().to_string();

    let removed = state
        .manager
        .take_pending_plan(&root_str, &req.agent_id)
        .await;

    if removed.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "No pending plan" })),
        )
            .into_response();
    }

    persist_and_emit_message(
        &state.manager,
        &state.events_tx,
        &root,
        &req.agent_id,
        &req.agent_id,
        "user",
        "Plan rejected.",
        req.session_id.as_deref(),
        false,
    )
    .await;

    Json(serde_json::json!({ "status": "rejected" })).into_response()
}

// ── Edit plan endpoint ──────────────────────────────────────────────────

pub(crate) async fn edit_plan_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<EditPlanRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);
    let root_str = root.to_string_lossy().to_string();

    let updated = state
        .manager
        .edit_pending_plan(&root_str, &req.agent_id, &req.text)
        .await;

    if updated {
        (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "No pending plan"})),
        )
            .into_response()
    }
}

// ── AskUser response endpoint ────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct AskUserResponseRequest {
    question_id: String,
    answers: Vec<crate::engine::tools::AskUserAnswer>,
}

pub(crate) async fn ask_user_response_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<AskUserResponseRequest>,
) -> impl IntoResponse {
    let sender = {
        let mut pending = state.pending_ask_user.lock().await;
        pending.remove(&req.question_id)
    };

    match sender {
        Some(entry) => {
            if entry.sender.send(req.answers).is_ok() {
                Json(serde_json::json!({ "status": "ok" })).into_response()
            } else {
                (StatusCode::GONE, Json(serde_json::json!({ "error": "Question already expired" }))).into_response()
            }
        }
        None => {
            (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Unknown question_id" }))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_explicit_target_prefix;

    #[test]
    fn parse_explicit_target_prefix_accepts_valid_mention() {
        let parsed = parse_explicit_target_prefix("@coder please review src/main.rs");
        assert_eq!(parsed, Some(("coder", "please review src/main.rs")));
    }

    #[test]
    fn parse_explicit_target_prefix_rejects_missing_body() {
        let parsed = parse_explicit_target_prefix("@coder");
        assert_eq!(parsed, None);
    }

    #[test]
    fn parse_explicit_target_prefix_rejects_invalid_agent_token() {
        let parsed = parse_explicit_target_prefix("@coder! please review");
        assert_eq!(parsed, None);
    }
}
