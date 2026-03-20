use crate::agent_manager::AgentManager;
use crate::server::chat_helpers::{
    emit_outcome_event, emit_queue_updated, persist_and_emit_message,
    persist_and_emit_to_store, persist_message_only, queue_key, queue_preview,
};
use crate::server::{AgentStatusKind, QueuedChatItem, ServerEvent, ServerState};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::path::PathBuf;
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
    /// When set, this chat belongs to a mission session — persist messages
    /// under `~/.linggen/missions/{mission_id}/sessions/` instead of
    /// the project's session store.
    mission_id: Option<String>,
    /// When set, this chat belongs to a skill session — persist messages
    /// under `~/.linggen/skills/{skill_name}/sessions/` instead of
    /// the project's session store.
    skill_name: Option<String>,
    /// Session-level model override. Takes priority over routing.default_models.
    model_id: Option<String>,
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

#[derive(Deserialize)]
pub(crate) struct CompactChatRequest {
    project_root: String,
    session_id: Option<String>,
    agent_id: Option<String>,
    focus: Option<String>,
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
    events_tx: &tokio::sync::broadcast::Sender<crate::server::ServerEvent>,
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
                let _ = manager.finish_agent_run(&run_id, status, Some(msg.clone())).await;
                // Emit error to chat so the user sees it in the UI.
                let _ = events_tx.send(crate::server::ServerEvent::Message {
                    from: agent_id.to_string(),
                    to: "user".to_string(),
                    content: format!("Error: {}", msg),
                    session_id: session_id.map(|s| s.to_string()),
                });
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

#[derive(Deserialize)]
pub(crate) struct SystemPromptQuery {
    project_root: String,
    agent_id: String,
}

pub(crate) async fn get_system_prompt_api(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<SystemPromptQuery>,
) -> impl IntoResponse {
    let root = match PathBuf::from(&query.project_root).canonicalize() {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    let ctx = match state.manager.get_or_create_project(root).await {
        Ok(ctx) => ctx,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let agents = ctx.agents.lock().await;
    if let Some(agent_mutex) = agents.get(&query.agent_id) {
        let mut engine = agent_mutex.lock().await;
        // Build the full system prompt including tool schemas, plan mode, delegation, etc.
        let (messages, _, _) = engine.prepare_loop_messages("(export)", true);
        let system_prompt = messages.first()
            .map(|m| m.content.clone())
            .unwrap_or_default();
        Json(serde_json::json!({ "system_prompt": system_prompt })).into_response()
    } else {
        (StatusCode::NOT_FOUND, format!("Agent '{}' not found", query.agent_id)).into_response()
    }
}

pub(crate) async fn compact_chat_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CompactChatRequest>,
) -> impl IntoResponse {
    let session_id = req
        .session_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let agent_id = req.agent_id.clone().unwrap_or_else(|| "ling".to_string());
    let root = match PathBuf::from(&req.project_root).canonicalize() {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    let focus = req.focus.as_deref();

    match state.manager.get_or_create_project(root).await {
        Ok(ctx) => {
            let agents = ctx.agents.lock().await;
            let agent_mutex = match agents.get(&agent_id) {
                Some(a) => a.clone(),
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        format!("Agent '{}' not found", agent_id),
                    )
                        .into_response()
                }
            };
            drop(agents);

            let mut engine = agent_mutex.lock().await;
            let mut messages = std::mem::take(&mut engine.chat_history);
            let result = engine.force_compact(&mut messages, focus).await;

            // Extract referenced file paths from the compacted messages.
            let referenced_files: Vec<String> = messages
                .iter()
                .flat_map(|m| extract_file_references(&m.content))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();

            // Rewrite the persisted session file with the compacted messages.
            if result.is_some() {
                let chat_msgs: Vec<crate::state_fs::sessions::ChatMsg> = messages
                    .iter()
                    .map(|m| {
                        let is_user = m.role == "user" || m.role == "system";
                        crate::state_fs::sessions::ChatMsg {
                            agent_id: agent_id.clone(),
                            from_id: if is_user { "user".to_string() } else { agent_id.clone() },
                            to_id: if is_user { agent_id.clone() } else { "user".to_string() },
                            content: m.content.clone(),
                            timestamp: crate::util::now_ts_secs(),
                            is_observation: m.role == "tool",
                        }
                    })
                    .collect();
                if let Err(e) = ctx.sessions.rewrite_chat_history(&session_id, &chat_msgs) {
                    tracing::warn!("Failed to rewrite session after compact: {e}");
                }
            }

            engine.chat_history = messages;
            drop(engine);

            let _ = state.events_tx.send(ServerEvent::StateUpdated);

            match result {
                Some(summary) => Json(serde_json::json!({
                    "compacted": true,
                    "summary": summary,
                    "referenced_files": referenced_files,
                }))
                .into_response(),
                None => Json(serde_json::json!({
                    "compacted": false,
                    "summary": "Nothing to compact — context is too small.",
                }))
                .into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Extract file paths referenced in message content (e.g. from Read/Edit/Write tool calls).
fn extract_file_references(content: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in content.lines() {
        // Match patterns like "Reading file src/foo.rs", "Editing file src/bar.rs", etc.
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Reading file ").or_else(|| trimmed.strip_prefix("Editing file "))
            .or_else(|| trimmed.strip_prefix("Writing file "))
            .or_else(|| trimmed.strip_prefix("Read "))
            .or_else(|| trimmed.strip_prefix("Edit "))
            .or_else(|| trimmed.strip_prefix("Write "))
        {
            let path = rest.split_whitespace().next().unwrap_or("").trim_matches('`');
            if !path.is_empty() {
                files.push(path.to_string());
            }
        }
        // Match file_path patterns from tool JSON
        if let Some(start) = trimmed.find("file_path") {
            if let Some(colon) = trimmed[start..].find(':') {
                let after = trimmed[start + colon + 1..].trim().trim_matches('"').trim_matches(',');
                if !after.is_empty() && (after.contains('/') || after.contains('.')) {
                    files.push(after.to_string());
                }
            }
        }
    }
    files
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
            // App skill: launch app UI only when --web flag is present.
            // Without --web, fall through to run as a regular skill (model uses tools).
            let wants_web = user_args.as_ref().map_or(false, |a| a.contains("--web"));
            if wants_web {
                if let Some(ref app) = skill.app {
                    let launch_msg = format!("Launching app: {}", skill.name);
                    persist_and_emit_message(
                        &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                        "user", &ctx.agent_id, &launch_msg, ctx.session_id.as_deref(), false,
                    )
                    .await;

                    match app.launcher.as_str() {
                        "web" => {
                            let url = format!("/apps/{}/{}", skill.name, app.entry);
                            let full_url = format!("http://localhost:{}{}", ctx.state.port, url);
                            let _ = ctx.events_tx.send(ServerEvent::AppLaunched {
                                skill: skill.name.clone(),
                                launcher: "web".to_string(),
                                url,
                                title: skill.description.clone(),
                                width: app.width,
                                height: app.height,
                            });
                            if ctx.events_tx.receiver_count() <= 1 {
                                let _ = open_in_browser(&full_url);
                            }
                        }
                        "url" => {
                            let _ = ctx.events_tx.send(ServerEvent::AppLaunched {
                                skill: skill.name.clone(),
                                launcher: "url".to_string(),
                                url: app.entry.clone(),
                                title: skill.description.clone(),
                                width: app.width,
                                height: app.height,
                            });
                            if ctx.events_tx.receiver_count() <= 1 {
                                let _ = open_in_browser(&app.entry);
                            }
                        }
                        "bash" => {
                            if let Some(ref skill_dir) = skill.skill_dir {
                                let script = skill_dir.join(&app.entry);
                                let mut cmd = std::process::Command::new("sh");
                                cmd.arg(script.as_os_str());
                                if let Some(ref args) = user_args {
                                    for arg in args.split_whitespace() {
                                        cmd.arg(arg);
                                    }
                                }
                                cmd.current_dir(&ctx.root);
                                match cmd.output() {
                                    Ok(output) => {
                                        let result_msg = String::from_utf8_lossy(&output.stdout).to_string();
                                        if !result_msg.trim().is_empty() {
                                            persist_and_emit_message(
                                                &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                                                &ctx.agent_id, "user", &result_msg, ctx.session_id.as_deref(), false,
                                            )
                                            .await;
                                        }
                                    }
                                    Err(e) => {
                                        let err_msg = format!("Failed to run app: {}", e);
                                        persist_and_emit_message(
                                            &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                                            &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
                                        )
                                        .await;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                    let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
                    return;
                }
            }
            engine.active_skill = Some(skill);
        }
    }

    let skill_default_task = engine.active_skill.as_ref().map(|s| {
        format!("Run the '{}' skill: {}", s.name, s.description)
    });
    let task_for_loop = user_args
        .or(skill_default_task)
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

    tracing::info!("Skill started: {}", cmd);

    ctx.state
        .send_agent_status(
            ctx.agent_id.clone(),
            AgentStatusKind::Thinking,
            Some(format!("Running skill: {}", cmd)),
            None,
        )
        .await;

    let interrupt_key = wire_interrupt_channel(ctx, engine).await;
    wire_ask_user_bridge(&ctx.state, engine, ctx.session_id.clone());

    let outcome = run_loop_with_tracking(
        &ctx.manager, &ctx.root, engine, &ctx.agent_id,
        ctx.session_id.as_deref(), "chat:skill", &ctx.events_tx,
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
        tracing::info!("Skill completed: {}", cmd);
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

    tracing::info!("Trigger skill started: {}", skill_name);

    ctx.state
        .send_agent_status(
            ctx.agent_id.clone(),
            AgentStatusKind::Thinking,
            Some(format!("Running skill: {}", skill_name)),
            None,
        )
        .await;

    let interrupt_key = wire_interrupt_channel(ctx, engine).await;
    wire_ask_user_bridge(&ctx.state, engine, ctx.session_id.clone());

    let outcome = run_loop_with_tracking(
        &ctx.manager, &ctx.root, engine, &ctx.agent_id,
        ctx.session_id.as_deref(), "chat:trigger", &ctx.events_tx,
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
        tracing::info!("Trigger skill completed: {}", skill_name);
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
    // Forward images so the plan-mode sub-loop can see them (pending_images
    // was consumed by std::mem::take in the previous loop's prepare_loop_messages).
    if engine.pending_images.is_empty() && !ctx.images.is_empty() {
        engine.pending_images = ctx.images.clone();
    }
    // Add user message to chat history so subsequent turns have conversational context.
    engine.chat_history.push(crate::ollama::ChatMessage::new("user", ctx.clean_msg.clone()));
    engine.truncate_chat_history();

    // Wire up the thinking channel.
    let (thinking_tx, mut thinking_rx) = tokio::sync::mpsc::unbounded_channel();
    engine.thinking_tx = Some(thinking_tx);

    // Wire up the interrupt channel so user messages reach the running loop.
    let interrupt_key = wire_interrupt_channel(ctx, engine).await;
    wire_ask_user_bridge(&ctx.state, engine, ctx.session_id.clone());

    let events_tx_clone = ctx.events_tx.clone();
    let agent_id_clone = ctx.agent_id.clone();
    let session_id_for_thinking = ctx.session_id.clone();
    tokio::spawn(async move {
        while let Some(event) = thinking_rx.recv().await {
            match event {
                crate::engine::ThinkingEvent::Token(token) => {
                    let _ = events_tx_clone.send(ServerEvent::Token { session_id: session_id_for_thinking.clone(),
                        agent_id: agent_id_clone.clone(),
                        token,
                        done: false,
                        thinking: true,
                    });
                }
                crate::engine::ThinkingEvent::ContentToken(token) => {
                    let _ = events_tx_clone.send(ServerEvent::Token { session_id: session_id_for_thinking.clone(),
                        agent_id: agent_id_clone.clone(),
                        token,
                        done: false,
                        thinking: false,
                    });
                }
                crate::engine::ThinkingEvent::Done => {
                    let _ = events_tx_clone.send(ServerEvent::Token { session_id: session_id_for_thinking.clone(),
                        agent_id: agent_id_clone.clone(),
                        token: String::new(),
                        done: true,
                        thinking: true,
                    });
                }
                crate::engine::ThinkingEvent::ContentDone => {
                    let _ = events_tx_clone.send(ServerEvent::Token { session_id: session_id_for_thinking.clone(),
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
        &ctx.events_tx,
    )
    .await;

    engine.thinking_tx = None;
    unwire_interrupt_channel(ctx, engine, &interrupt_key).await;
    engine.plan_mode = false;

    match outcome {
        Ok(ref out) => {
            // Skip emitting the raw plan text as a regular message — the plan
            // content reaches the UI via the PlanUpdate SSE event instead.
            // Emitting it here would create a duplicate text message that hides
            // the PlanBlock widget.
            if !matches!(out, crate::engine::AgentOutcome::Plan(_)) {
                persist_and_emit_last_assistant_text(ctx, engine).await;
            }
            if let crate::engine::AgentOutcome::Plan(ref plan) = out {
                // Persist the plan as a JSON message so it survives session reload.
                let plan_json = serde_json::json!({ "type": "plan", "plan": plan }).to_string();
                persist_message_only(
                    &ctx.manager, &ctx.root, &ctx.agent_id,
                    &ctx.agent_id, "user", &plan_json,
                    ctx.session_id.as_deref(), false,
                ).await;
            }
            emit_outcome_event(out, &ctx.events_tx, &ctx.agent_id);
            if let crate::engine::AgentOutcome::Plan(ref plan) = out {
                // Store pending plan — user approves via PlanBlock buttons in UI.
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

/// Run the execution loop for an approved plan. Wires thinking/interrupt channels,
/// runs the loop, and emits outcome events. Engine must already have plan + task set.
async fn run_plan_execution(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
) {
    let (thinking_tx, mut thinking_rx) = tokio::sync::mpsc::unbounded_channel();
    engine.thinking_tx = Some(thinking_tx);
    let interrupt_key = wire_interrupt_channel(ctx, engine).await;
    wire_ask_user_bridge(&ctx.state, engine, ctx.session_id.clone());

    let events_tx_clone = ctx.events_tx.clone();
    let agent_id_clone = ctx.agent_id.clone();
    let session_id_for_thinking = ctx.session_id.clone();
    tokio::spawn(async move {
        while let Some(event) = thinking_rx.recv().await {
            match event {
                crate::engine::ThinkingEvent::Token(token) => {
                    let _ = events_tx_clone.send(ServerEvent::Token { session_id: session_id_for_thinking.clone(),
                        agent_id: agent_id_clone.clone(), token, done: false, thinking: true,
                    });
                }
                crate::engine::ThinkingEvent::ContentToken(token) => {
                    let _ = events_tx_clone.send(ServerEvent::Token { session_id: session_id_for_thinking.clone(),
                        agent_id: agent_id_clone.clone(), token, done: false, thinking: false,
                    });
                }
                crate::engine::ThinkingEvent::Done => {
                    let _ = events_tx_clone.send(ServerEvent::Token { session_id: session_id_for_thinking.clone(),
                        agent_id: agent_id_clone.clone(), token: String::new(), done: true, thinking: true,
                    });
                }
                crate::engine::ThinkingEvent::ContentDone => {
                    let _ = events_tx_clone.send(ServerEvent::Token { session_id: session_id_for_thinking.clone(),
                        agent_id: agent_id_clone.clone(), token: String::new(), done: true, thinking: false,
                    });
                }
            }
        }
    });

    ctx.state
        .send_agent_status(
            ctx.agent_id.clone(),
            AgentStatusKind::Thinking,
            Some("Executing plan".to_string()),
            None,
        )
        .await;

    let exec_outcome = run_loop_with_tracking(
        &ctx.manager, &ctx.root, engine, &ctx.agent_id,
        ctx.session_id.as_deref(), "chat:plan-execution", &ctx.events_tx,
    )
    .await;

    engine.thinking_tx = None;
    unwire_interrupt_channel(ctx, engine, &interrupt_key).await;

    match exec_outcome {
        Ok(ref out) => {
            emit_outcome_event(out, &ctx.events_tx, &ctx.agent_id);
            // For Done (AgentOutcome::None): emit the last_assistant_text as a
            // Message so the UI shows the completion summary.
            if matches!(out, crate::engine::AgentOutcome::None) {
                if let Some(text) = &engine.last_assistant_text {
                    if !text.is_empty() {
                        let _ = ctx.events_tx.send(ServerEvent::Message {
                            from: ctx.agent_id.clone(),
                            to: "user".to_string(),
                            content: text.clone(),
                            session_id: ctx.session_id.clone(),
                        });
                    }
                }
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
/// finalize liveText → a permanent message bubble.  Used for plan outcomes
/// where the engine doesn't persist the text itself.
async fn persist_and_emit_last_assistant_text(
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
    // Add user message to chat history so subsequent turns have conversational context.
    engine.chat_history.push(crate::ollama::ChatMessage::new("user", ctx.clean_msg.clone()));
    engine.truncate_chat_history();

    // Wire up the thinking channel so streaming thinking tokens reach the UI.
    let (thinking_tx, mut thinking_rx) = tokio::sync::mpsc::unbounded_channel();
    engine.thinking_tx = Some(thinking_tx);

    // Wire up the interrupt channel so user messages reach the running loop.
    let interrupt_key = wire_interrupt_channel(ctx, engine).await;
    wire_ask_user_bridge(&ctx.state, engine, ctx.session_id.clone());

    let events_tx_clone = ctx.events_tx.clone();
    let agent_id_clone = ctx.agent_id.clone();
    let session_id_for_thinking = ctx.session_id.clone();
    tokio::spawn(async move {
        while let Some(event) = thinking_rx.recv().await {
            match event {
                crate::engine::ThinkingEvent::Token(token) => {
                    let _ = events_tx_clone.send(ServerEvent::Token { session_id: session_id_for_thinking.clone(),
                        agent_id: agent_id_clone.clone(),
                        token,
                        done: false,
                        thinking: true,
                    });
                }
                crate::engine::ThinkingEvent::ContentToken(token) => {
                    let _ = events_tx_clone.send(ServerEvent::Token { session_id: session_id_for_thinking.clone(),
                        agent_id: agent_id_clone.clone(),
                        token,
                        done: false,
                        thinking: false,
                    });
                }
                crate::engine::ThinkingEvent::Done => {
                    let _ = events_tx_clone.send(ServerEvent::Token { session_id: session_id_for_thinking.clone(),
                        agent_id: agent_id_clone.clone(),
                        token: String::new(),
                        done: true,
                        thinking: true,
                    });
                }
                crate::engine::ThinkingEvent::ContentDone => {
                    let _ = events_tx_clone.send(ServerEvent::Token { session_id: session_id_for_thinking.clone(),
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
        ctx.session_id.as_deref(), "chat:structured-loop", &ctx.events_tx,
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
        // Persist the plan as a JSON message so it survives session reload.
        // The UI renders it as a PlanBlock via tryRenderSpecialBlock.
        let plan_json = serde_json::json!({ "type": "plan", "plan": plan }).to_string();
        persist_message_only(
            &ctx.manager, &ctx.root, &ctx.agent_id,
            &ctx.agent_id, "user", &plan_json,
            ctx.session_id.as_deref(), false,
        ).await;
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

    // Agent plan was approved inline — start execution immediately.
    if let Ok(crate::engine::AgentOutcome::PlanApproved(ref plan)) = outcome {
        persist_and_emit_last_assistant_text(ctx, engine).await;
        emit_outcome_event(outcome.as_ref().unwrap(), &ctx.events_tx, &ctx.agent_id);
        engine.plan = Some(plan.clone());
        engine.plan_mode = false;
        engine.observations.clear();
        if engine.task.is_none() {
            engine.task = Some(format!("Execute the approved plan: {}", plan.summary));
        }
        // Use run_plan_execution helper (same logic as plan dispatch path)
        run_plan_execution(ctx, engine).await;
        return;
    }

    if let Ok(outcome) = &outcome {
        emit_outcome_event(outcome, &ctx.events_tx, &ctx.agent_id);
        // For text-only responses (AgentOutcome::None with streamed content),
        // the engine already persists the message to session files but doesn't
        // emit a ServerEvent::Message.  Emit one now so the UI can finalize
        // liveText → text.  (Don't persist again — the engine already did.)
        if matches!(outcome, crate::engine::AgentOutcome::None) {
            if let Some(text) = &engine.last_assistant_text {
                if !text.is_empty() {
                    let _ = ctx.events_tx.send(ServerEvent::Message {
                        from: ctx.agent_id.clone(),
                        to: "user".to_string(),
                        content: text.clone(),
                        session_id: ctx.session_id.clone(),
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
    let root = if req.project_root.is_empty() {
        std::env::current_dir().unwrap_or_default()
    } else {
        PathBuf::from(&req.project_root)
    };
    let project_root_str = root.to_string_lossy().to_string();

    // Determine if this chat belongs to a mission or skill — if so, route session
    // persistence to their session store, not the project's.
    let sessions_root_override: Option<std::path::PathBuf> = if let Some(ref mid) = req.mission_id {
        Some(crate::paths::mission_sessions_dir(mid))
    } else if let Some(ref skill) = req.skill_name {
        Some(crate::paths::skill_sessions_dir(skill))
    } else {
        None
    };

    // Session creator type — computed once, used when creating new sessions.
    let session_creator: &str = if req.mission_id.is_some() { "mission" }
        else if req.skill_name.is_some() { "skill" }
        else { "user" };

    // Auto-create a new session when none is provided, instead of falling
    // back to a hidden "default" session.
    // When a session_id IS provided (e.g. from TUI), ensure it exists in the
    // session store so the Web UI can list it.
    let session_id: Option<String> = if let Some(sid) = req.session_id.clone() {
        if let Some(ref alt_dir) = sessions_root_override {
            // Mission session — ensure it exists in the mission store.
            let store = crate::state_fs::SessionStore::with_sessions_dir(alt_dir.clone());
            let exists = store.list_sessions().ok()
                .map(|ss| ss.iter().any(|s| s.id == sid))
                .unwrap_or(false);
            if !exists {
                let now = crate::util::now_ts_secs();

                let meta = crate::state_fs::sessions::SessionMeta {
                    id: sid.clone(),
                    title: auto_session_title(&req.message),
                    created_at: now,
                    skill: None,
                    creator: session_creator.into(),
                };
                let _ = store.add_session(&meta);
            }
        } else if let Ok(ctx) = state.manager.get_or_create_project(root.clone()).await {
            let exists = ctx.sessions.list_sessions().ok()
                .map(|ss| ss.iter().any(|s| s.id == sid))
                .unwrap_or(false);
            if !exists {
                let now = crate::util::now_ts_secs();

                let meta = crate::state_fs::sessions::SessionMeta {
                    id: sid.clone(),
                    title: auto_session_title(&req.message),
                    created_at: now,
                    skill: None,
                    creator: session_creator.into(),
                };
                let _ = ctx.sessions.add_session(&meta);
            }
        }
        Some(sid)
    } else {
        let now = crate::util::now_ts_secs();
        let new_id = format!("sess-{}-{}", now, &uuid::Uuid::new_v4().to_string()[..8]);
        let creator = if req.mission_id.is_some() { "mission" } else if req.skill_name.is_some() { "skill" } else { "user" };
        if let Some(ref alt_dir) = sessions_root_override {
            let store = crate::state_fs::SessionStore::with_sessions_dir(alt_dir.clone());
            let meta = crate::state_fs::sessions::SessionMeta {
                id: new_id.clone(),
                title: auto_session_title(&req.message),
                created_at: now,
                skill: None,
                creator: session_creator.into(),
            };
            let _ = store.add_session(&meta);
        } else if let Ok(ctx) = state.manager.get_or_create_project(root.clone()).await {
            let meta = crate::state_fs::sessions::SessionMeta {
                id: new_id.clone(),
                title: auto_session_title(&req.message),
                created_at: now,
                skill: None,
                creator: session_creator.into(),
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
            let _req_mode = req.mode.clone();
            let req_model_id = req.model_id.clone();
            let req_images = req.images.clone();
            let sessions_override_for_spawn = sessions_root_override.clone();

            if was_busy {
                // When queued, only persist to disk — don't emit via SSE.
                if let Some(ref alt_dir) = sessions_root_override {
                    let store = crate::state_fs::SessionStore::with_sessions_dir(alt_dir.clone());
                    let sid = session_id.as_deref().unwrap_or("default");
                    let msg = crate::state_fs::sessions::ChatMsg {
                        agent_id: target_id.to_string(),
                        from_id: "user".to_string(),
                        to_id: target_id.to_string(),
                        content: clean_msg.clone(),
                        timestamp: crate::util::now_ts_secs(),
                        is_observation: false,
                    };
                    let _ = store.add_chat_message(sid, &msg);
                } else {
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
                }
            } else {
                // Emit + persist plain text. Images are ephemeral (sent inline
                // as base64 for the current turn only, not persisted to disk).
                if let Some(ref alt_dir) = sessions_root_override {
                    let store = crate::state_fs::SessionStore::with_sessions_dir(alt_dir.clone());
                    persist_and_emit_to_store(
                        &store,
                        &events_tx,
                        &target_id,
                        "user",
                        &target_id,
                        &clean_msg,
                        session_id.as_deref(),
                        false,
                    )
                    .await;
                } else {
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
            }

            // Register agent → session mapping BEFORE spawn so SSE events are
            // tagged correctly even while awaiting the agent lock.
            if let Some(sid) = &session_id {
                state.agent_sessions.write().unwrap()
                    .insert(target_id.clone(), sid.clone());
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
                    // Emit the queued user message via SSE now that it's being processed.
                    let _ = events_tx_clone.send(ServerEvent::Message {
                        from: "user".to_string(),
                        to: target_id_clone.clone(),
                        content: clean_msg_clone.clone(),
                        session_id: session_id.clone(),
                    });
                }

                // agent_sessions already registered before spawn — no need to repeat here.

                // Apply session-level model override if provided.
                if let Some(ref mid) = req_model_id {
                    if engine.model_manager.has_model(mid) {
                        engine.model_id = mid.clone();
                    }
                }

                // Set session_id on engine tools so subagent delegations inherit it.
                engine.tools.builtins.set_session_id(session_id.clone());

                // Route session persistence to mission directory when applicable.
                if let Some(ref alt_dir) = sessions_override_for_spawn {
                    engine.cfg.session_root = Some(alt_dir.clone());
                }

                // Clear mission-only restrictions — user-initiated chats should
                // never be restricted by permission tiers (those apply only to
                // automated scheduler runs via apply_permission_tier).
                engine.cfg.mission_allowed_tools = None;
                engine.cfg.bash_allow_prefixes = None;

                // --- Session promotion: mission → user ---
                // The mission scheduler never goes through chat_handler (it calls
                // dispatch_mission_prompt directly), so any request here with a
                // mission session is from a real user taking over the conversation.
                if sessions_override_for_spawn.is_some() {
                    if let Some(ref sid) = session_id {
                        if let Some(ref alt_dir) = sessions_override_for_spawn {
                            let store = crate::state_fs::SessionStore::with_sessions_dir(alt_dir.clone());
                            if let Ok(Some(mut meta)) = store.get_session_meta(sid) {
                                if meta.creator == "mission" {
                                    meta.creator = "user".to_string();
                                    let _ = store.update_session_meta(&meta);
                                }
                            }
                        }
                    }

                    // Reset tool_permission_mode from Auto (forced by mission scheduler)
                    // back to the configured default.
                    let cfg = state_clone.manager.get_config_snapshot().await;
                    engine.cfg.tool_permission_mode = cfg.agent.tool_permission_mode;

                    // Force system prompt rebuild to reflect new permission context.
                    engine.cached_system_prompt = None;
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
                    let sid = ctx.session_id.as_deref().unwrap_or("default");
                    let history_result = if let Some(ref alt_dir) = sessions_override_for_spawn {
                        let store = crate::state_fs::SessionStore::with_sessions_dir(alt_dir.clone());
                        store.get_chat_history(sid)
                    } else if let Ok(proj_ctx) = ctx.manager.get_or_create_project(ctx.root.clone()).await {
                        proj_ctx.sessions.get_chat_history(sid)
                    } else {
                        Err(anyhow::anyhow!("no session store"))
                    };
                    if let Ok(msgs) = history_result {
                        for m in &msgs {
                            if m.is_observation || m.from_id == "system" {
                                continue;
                            }
                            // Session owns the conversation — any non-user
                            // message is assistant context regardless of
                            // which agent produced it.
                            let role = if m.from_id == "user" {
                                "user"
                            } else {
                                "assistant"
                            };
                            let msg = crate::ollama::ChatMessage::new(role, &m.content);
                            engine.chat_history.push(msg);
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

                // Load session-bound skill if the session has one.
                if engine.active_skill.is_none() {
                    if let Some(sid) = &ctx.session_id {
                        let bound_skill_name = if let Some(ref alt_dir) = sessions_override_for_spawn {
                            let store = crate::state_fs::SessionStore::with_sessions_dir(alt_dir.clone());
                            match store.get_session_meta(sid) {
                                Ok(meta) => meta.and_then(|m| m.skill),
                                Err(e) => {
                                    tracing::warn!("Failed to read session meta for {}: {}", sid, e);
                                    None
                                }
                            }
                        } else if let Ok(proj_ctx) = ctx.manager.get_or_create_project(ctx.root.clone()).await {
                            match proj_ctx.sessions.get_session_meta(sid) {
                                Ok(meta) => meta.and_then(|m| m.skill),
                                Err(e) => {
                                    tracing::warn!("Failed to read session meta for {}: {}", sid, e);
                                    None
                                }
                            }
                        } else {
                            None
                        };
                        if let Some(ref skill_name) = bound_skill_name {
                            if let Some(skill) = manager.skill_manager.get_skill(skill_name).await {
                                tracing::info!("Session-bound skill activated: {}", skill.name);
                                engine.active_skill = Some(skill);
                            }
                        }
                    }
                }

                if clean_msg_clone.trim_start().starts_with('/') {
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
                // Deregister agent → session mapping after a short delay so the
                // SSE filter can still enrich the TurnComplete and idle AgentStatus
                // events with the correct session_id before they are streamed out.
                // Only remove if the session_id still matches — a newer run may have
                // re-registered the same agent with a different session.
                let sessions_ref = state_clone.agent_sessions.clone();
                let agent_key = target_id_clone.clone();
                let old_sid = session_id.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let mut map = sessions_ref.write().unwrap();
                    if let Some(current_sid) = map.get(&agent_key) {
                        if old_sid.as_ref() == Some(current_sid) {
                            map.remove(&agent_key);
                        }
                    }
                });
            });

            let status = if was_busy { "queued" } else { "started" };
            Json(serde_json::json!({ "status": status, "session_id": session_id_response })).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Recover a pending plan from persisted session messages after server restart.
/// Scans the session history backwards for the last plan message with status "planned".
async fn recover_plan_from_session(
    state: &Arc<ServerState>,
    root: &std::path::Path,
    agent_id: &str,
    session_id: Option<&str>,
) -> Option<crate::engine::Plan> {
    let sid = session_id.unwrap_or("default");
    let ctx = state.manager.get_or_create_project(root.to_path_buf()).await.ok()?;
    let messages = ctx.sessions.get_chat_history(sid).ok()?;
    // Scan backwards for the last plan message. Stop at the first plan found
    // regardless of status — if the most recent plan is "approved" or "completed",
    // there is no pending plan (don't keep scanning for older "planned" ones).
    for msg in messages.iter().rev() {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&msg.content) {
            if parsed.get("type").and_then(|v| v.as_str()) == Some("plan") {
                if let Some(plan_val) = parsed.get("plan") {
                    if let Ok(plan) = serde_json::from_value::<crate::engine::Plan>(plan_val.clone()) {
                        if plan.status == crate::engine::PlanStatus::Planned {
                            tracing::info!("Recovered pending plan from session history for {agent_id}");
                            return Some(plan);
                        }
                        // Most recent plan is not "planned" — no pending plan
                        return None;
                    }
                }
            }
        }
    }
    None
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
    // Fallback: after server restart the in-memory pending_plans map is empty.
    // Reconstruct from the last persisted plan message in the session.
    let plan = match plan {
        Some(p) => Some(p),
        None => recover_plan_from_session(&state, &root, &req.agent_id, session_id.as_deref()).await,
    };
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

    tokio::spawn(async move {
        // Register agent → session mapping so SSE events get tagged with
        // the correct session_id (same as chat_handler does).
        if let Some(ref sid) = session_id {
            state_clone.agent_sessions.write().unwrap()
                .insert(agent_id.clone(), sid.clone());
        }

        let mut engine = agent.lock().await;

        // Set the approved plan on the engine.
        engine.plan = Some(plan);
        engine.plan_mode = false;

        if clear_context {
            // Clear old messages on disk but keep plan messages,
            // so the session file starts fresh with just the plan.
            let sid = session_id.as_deref().unwrap_or("default");
            if let Ok(ctx) = manager.get_or_create_project(root_clone.clone()).await {
                if let Ok(msgs) = ctx.sessions.get_chat_history(sid) {
                    let plan_msgs: Vec<_> = msgs.into_iter().filter(|m| {
                        serde_json::from_str::<serde_json::Value>(&m.content)
                            .ok()
                            .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(|t| t == "plan"))
                            .unwrap_or(false)
                    }).collect();
                    let _ = ctx.sessions.rewrite_chat_history(sid, &plan_msgs);
                }
            }
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

        // Persist and emit the approved plan (after clear, so it's the first message).
        let plan_snapshot = engine.plan.clone().unwrap();
        engine.persist_and_emit_plan(plan_snapshot).await;

        // Build a ChatRunCtx so we can reuse run_plan_execution.
        let session_id_for_cleanup = session_id.clone();
        let ctx = ChatRunCtx {
            state: state_clone.clone(),
            manager,
            events_tx: events_tx.clone(),
            root: root_clone,
            agent_id: agent_id.clone(),
            session_id,
            clean_msg: String::new(),
            images: Vec::new(),
        };

        run_plan_execution(&ctx, &mut engine).await;

        // Mark plan as completed and persist so reload shows correct status.
        if let Some(ref mut plan) = engine.plan {
            if plan.status == crate::engine::PlanStatus::Executing
                || plan.status == crate::engine::PlanStatus::Approved
            {
                plan.status = crate::engine::PlanStatus::Completed;
                let plan_snapshot = plan.clone();
                engine.persist_and_emit_plan(plan_snapshot).await;
            }
        }

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

        // Deregister agent → session mapping after a short delay so the
        // TurnComplete and idle events still get enriched with session_id.
        // Only remove if the session_id still matches — a newer run may have
        // re-registered the same agent with a different session.
        let sessions_ref = state_clone.agent_sessions.clone();
        let agent_key = agent_id.clone();
        let old_sid = session_id_for_cleanup;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let mut map = sessions_ref.write().unwrap();
            if let Some(current_sid) = map.get(&agent_key) {
                if old_sid.as_ref() == Some(current_sid) {
                    map.remove(&agent_key);
                }
            }
        });
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
    let removed = match removed {
        Some(p) => Some(p),
        None => recover_plan_from_session(&state, &root, &req.agent_id, req.session_id.as_deref()).await,
    };

    if removed.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "No pending plan" })),
        )
            .into_response();
    }

    // Emit PlanUpdate with rejected status so the UI clears approval buttons.
    let mut rejected_plan = removed.unwrap();
    rejected_plan.status = crate::engine::PlanStatus::Rejected;
    let _ = state.events_tx.send(ServerEvent::PlanUpdate {
        agent_id: req.agent_id.clone(),
        plan: rejected_plan.clone(),
    });

    // Persist the rejected plan so recover_plan_from_session sees the updated
    // status on reload and doesn't resurface the old "planned" buttons.
    let plan_json = serde_json::json!({ "type": "plan", "plan": rejected_plan });
    persist_and_emit_message(
        &state.manager,
        &state.events_tx,
        &root,
        &req.agent_id,
        &req.agent_id,
        "user",
        &plan_json.to_string(),
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

/// Return any pending AskUser questions so the UI can restore the widget
/// after navigating away and back, or after a page refresh.
pub(crate) async fn pending_ask_user_handler(
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    let pending = state.pending_ask_user.lock().await;
    let items: Vec<serde_json::Value> = pending
        .iter()
        .map(|(qid, entry)| {
            serde_json::json!({
                "question_id": qid,
                "agent_id": entry.agent_id,
                "questions": entry.questions,
                "session_id": entry.session_id,
            })
        })
        .collect();
    Json(serde_json::json!(items))
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

/// Open a URL in the system's default browser.
fn open_in_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd").args(["/C", "start", url]).spawn()?;
    }
    Ok(())
}
