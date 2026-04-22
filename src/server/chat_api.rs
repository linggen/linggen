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

fn default_user_type() -> String { "owner".to_string() }

#[derive(Deserialize)]
pub(crate) struct ChatRequest {
    project_root: String,
    agent_id: String,
    message: String,
    session_id: Option<String>,
    /// User type: "owner" (default) or "consumer" (proxy room).
    /// Injected server-side by peer.rs. Missing = owner (local HTTP requests).
    #[serde(default = "default_user_type")]
    user_type: String,
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
    /// User ID of the session creator (linggen.dev user_id).
    /// Injected by peer.rs for both owner and consumer connections.
    user_id: Option<String>,
    #[serde(default)]
    images: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct PlanActionRequest {
    project_root: String,
    agent_id: String,
    session_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct EditPlanRequest {
    project_root: String,
    agent_id: String,
    session_id: Option<String>,
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
    // project_root is not needed — chat history is stored globally by session ID.
    // Skipping canonicalize avoids failures when the project directory is deleted.
    match state.manager.global_sessions.clear_chat_history(&session_id) {
        Ok(removed) => {
            // Clear in-memory chat history for this session's engine
            {
                let engines = state.manager.session_engines.lock().await;
                if let Some(engine_mutex) = engines.get(&session_id) {
                    let mut engine = engine_mutex.lock().await;
                    engine.chat_history.clear();
                    engine.observations.clear();
                }
            }
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            Json(serde_json::json!({ "removed": removed })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct SystemPromptQuery {
    project_root: String,
    agent_id: String,
    #[serde(default)]
    session_id: Option<String>,
}

pub(crate) async fn get_system_prompt_api(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<SystemPromptQuery>,
) -> impl IntoResponse {
    let sid = query.session_id.as_deref().unwrap_or("default");
    // Skill iframes call this without project_root (they clear selectedProjectRoot
    // so API calls use the session's cwd). Derive the root from session meta
    // when the query omits it.
    let root_str = if query.project_root.trim().is_empty() {
        state.manager.global_sessions.get_session_meta(sid)
            .ok()
            .flatten()
            .and_then(|m| m.cwd.clone().or(m.project.clone()))
            .unwrap_or_else(|| "/".to_string())
    } else {
        query.project_root.clone()
    };
    let root = match PathBuf::from(&root_str).canonicalize() {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    // Build the prompt on a throwaway engine instead of locking the live one.
    // The live engine holds its lock for the duration of an agent turn, which
    // can be minutes — long enough for the control-channel RPC (30s) to time
    // out and leave the Copy-System-Prompt button silently unusable while the
    // agent is working. A fresh engine yields the same prompt (same spec,
    // same skill, same memory, same tool schemas) without touching live state.
    let mut engine = match state.manager.spawn_delegation_engine(&root, &query.agent_id).await {
        Ok(e) => e,
        Err(_) => return (StatusCode::NOT_FOUND, format!("Agent '{}' not found", query.agent_id)).into_response(),
    };

    // Apply session-bound skill so the exported prompt matches what the model
    // actually sees during a chat turn. Without this, the export shows a
    // "cold engine" view without SKILL.md content or app-skill tool hints.
    if let Ok(Some(meta)) = state.manager.global_sessions.get_session_meta(sid) {
        if let Some(skill_name) = meta.skill {
            if let Some(skill) = state.manager.skill_manager.get_skill(&skill_name).await {
                engine.active_skill = Some(skill);
            }
        }
    }

    let (messages, allowed_tools, _) = engine.prepare_loop_messages("(export)", true);
    let system_prompt = messages.first()
        .map(|m| m.content.clone())
        .unwrap_or_default();
    // Tool schemas delivered to the model via the native function-calling
    // `tools` API parameter — not embedded in the system prompt text. Expose
    // them alongside so the debug export shows the full model-facing surface.
    let tools = engine.tools.oai_tool_definitions(allowed_tools.as_ref());
    Json(serde_json::json!({
        "system_prompt": system_prompt,
        "tools": tools,
    })).into_response()
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

    match state.manager.get_or_create_session_agent(&session_id, &root, &agent_id).await {
        Ok(agent_mutex) => {
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
                if let Err(e) = state.manager.global_sessions.rewrite_chat_history(&session_id, &chat_msgs) {
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
    policy: crate::engine::session_policy::SessionPolicy,
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
            // Check session policy skill allowlist
            if !ctx.policy.is_skill_allowed(&skill.name) {
                let err_msg = format!(
                    "Skill '{}' is not available in this room.",
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
                                session_id: ctx.session_id.clone(),
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
                                session_id: ctx.session_id.clone(),
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
            ctx.session_id.clone(),
        )
        .await;

    let interrupt_key = wire_interrupt_channel(ctx, engine).await;
    wire_ask_user_bridge(&ctx.state, engine, ctx.session_id.clone());

    // Skill permission approval — prompt user if skill declares permission requirements.
    // Consumer sessions are locked — skills run with the permissions already set by SessionPolicy.
    if !engine.session_permissions.policy.is_locked() {
        if let Some(ref skill) = engine.active_skill {
            if let Some(ref perm) = skill.permission {
                use crate::engine::permission::PermissionMode;
                let mode = match perm.mode.as_str() {
                    "edit" => PermissionMode::Edit,
                    "admin" => PermissionMode::Admin,
                    _ => PermissionMode::Read,
                };
                let paths_str = perm.paths.join(", ");
                let mut question_text = format!(
                    "Skill \"{}\" requests {} mode on: {}",
                    skill.name, perm.mode, paths_str
                );
                if let Some(ref warning) = perm.warning {
                    question_text.push_str(&format!("\n⚠️ {}", warning));
                }

                let question = crate::engine::tools::AskUserQuestion {
                    question: question_text,
                    header: "Permission".to_string(),
                    options: vec![
                        crate::engine::tools::AskUserOption {
                            label: "Approve".to_string(),
                            description: Some(format!("Grant {} mode on {}", perm.mode, paths_str)),
                            preview: None,
                        },
                        crate::engine::tools::AskUserOption {
                            label: "Run in current mode".to_string(),
                            description: Some("Skill runs with existing permissions (may fail)".to_string()),
                            preview: None,
                        },
                        crate::engine::tools::AskUserOption {
                            label: "Cancel".to_string(),
                            description: Some("Don't run this skill".to_string()),
                            preview: None,
                        },
                    ],
                    multi_select: false,
                };

                match engine.ask_permission_raw(&skill.name, question).await {
                    Some(crate::engine::permission::PermissionAction::AllowOnce) => {
                        // "Approve" — grant the requested permissions and lock
                        // the session to `strict` policy. The skill declared
                        // the paths it needs; anything outside that scope
                        // should be silently denied (so the model sees an
                        // error and adapts) rather than prompting the user on
                        // every speculative Glob/Grep. Spec: permission-spec.md
                        // → "A policy where either lever is not `ask` is
                        // locked — the agent never prompts the user".
                        for path in &perm.paths {
                            engine.session_permissions.set_path_mode(path, mode.clone());
                        }
                        // Skill may declare a policy (SKILL.md frontmatter
                        // `permission.policy`); default to `strict` so the
                        // skill's scope is enforced without user prompts.
                        engine.session_permissions.policy = perm
                            .policy
                            .as_deref()
                            .map(crate::engine::permission::PermissionPolicy::from_preset)
                            .unwrap_or_else(crate::engine::permission::PermissionPolicy::strict);
                        if let Some(ref sdir) = engine.session_dir {
                            engine.session_permissions.save(sdir);
                        }
                    }
                    Some(crate::engine::permission::PermissionAction::AllowSession) => {
                        // "Run in current mode" — proceed without grants.
                    }
                    _ => {
                        // "Cancel" or timeout — abort skill.
                        let msg = format!("Skill '{}' cancelled — permission not granted.", skill.name);
                        persist_and_emit_message(
                            &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                            &ctx.agent_id, "user", &msg, ctx.session_id.as_deref(), false,
                        ).await;
                        unwire_interrupt_channel(ctx, engine, &interrupt_key).await;
                        return;
                    }
                }
            }
        }
    }

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
            emit_outcome_event(outcome, &ctx.events_tx, &ctx.agent_id, ctx.session_id.as_deref());
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
            // Check session policy skill allowlist
            if !ctx.policy.is_skill_allowed(&skill.name) {
                let err_msg = format!(
                    "Skill '{}' is not available in this room.",
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
            ctx.session_id.clone(),
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
            emit_outcome_event(outcome, &ctx.events_tx, &ctx.agent_id, ctx.session_id.as_deref());
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
            ctx.session_id.clone(),
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
            emit_outcome_event(out, &ctx.events_tx, &ctx.agent_id, ctx.session_id.as_deref());
            if let crate::engine::AgentOutcome::Plan(ref plan) = out {
                // Store pending plan — user approves via PlanBlock buttons in UI.
                ctx.manager
                    .set_pending_plan(
                        &ctx.root.to_string_lossy(),
                        &ctx.agent_id,
                        ctx.session_id.as_deref(),
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
            ctx.session_id.clone(),
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
            emit_outcome_event(out, &ctx.events_tx, &ctx.agent_id, ctx.session_id.as_deref());
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
        .send_agent_status(ctx.agent_id.clone(), AgentStatusKind::Thinking, Some("Thinking".to_string()), None, ctx.session_id.clone())
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
    // Per-turn semantic retrieval against skill memory will hook in here once
    // a `provides: [memory]` skill is active and its `Memory.search` handler
    // is reachable via the memory dispatch layer (see memory-spec.md).
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
        emit_outcome_event(ok_outcome, &ctx.events_tx, &ctx.agent_id, ctx.session_id.as_deref());
        ctx.manager
            .set_pending_plan(
                &ctx.root.to_string_lossy(),
                &ctx.agent_id,
                ctx.session_id.as_deref(),
                plan.clone(),
            )
            .await;
        let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
        return;
    }

    // Agent plan was approved inline — start execution immediately.
    if let Ok(crate::engine::AgentOutcome::PlanApproved(ref plan)) = outcome {
        persist_and_emit_last_assistant_text(ctx, engine).await;
        emit_outcome_event(outcome.as_ref().unwrap(), &ctx.events_tx, &ctx.agent_id, ctx.session_id.as_deref());
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
        emit_outcome_event(outcome, &ctx.events_tx, &ctx.agent_id, ctx.session_id.as_deref());
        // Note: persist_assistant_message() (in engine/context.rs) already emits
        // an AgentEvent::Message which the bridge converts to ServerEvent::Message.
        // No additional Message event needed here — emitting one would duplicate
        // the assistant response for WebRTC consumers.
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
    let root = {
        let expanded = if req.project_root.is_empty() || req.project_root == "~" {
            dirs::home_dir().unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
        } else if req.project_root.starts_with("~/") {
            dirs::home_dir().unwrap_or_default().join(&req.project_root[2..])
        } else {
            PathBuf::from(&req.project_root)
        };
        crate::util::resolve_path(&expanded)
    };
    let project_root_str = root.to_string_lossy().to_string();

    // Session creator type — computed once, used when creating new sessions.
    let session_creator: &str = if req.mission_id.is_some() { "mission" }
        else if req.skill_name.is_some() { "skill" }
        else { "user" };

    let global_sessions = &state.manager.global_sessions;

    // Auto-create a new session when none is provided.
    // When a session_id IS provided, ensure it exists in the
    // session store so the Web UI can list it.
    let session_id: Option<String> = if let Some(sid) = req.session_id.clone() {
        // Ensure session exists in global store
        let exists = matches!(global_sessions.get_session_meta(&sid), Ok(Some(_)));
        if !exists {
            let now = crate::util::now_ts_secs();
            let meta = crate::state_fs::sessions::SessionMeta {
                id: sid.clone(),
                title: auto_session_title(&req.message),
                created_at: now,
                skill: req.skill_name.clone(),
                creator: session_creator.into(),
                cwd: Some(project_root_str.clone()),
                project: None, project_name: None,
                mission_id: req.mission_id.clone(),
                model_id: req.model_id.clone(),
                user_id: req.user_id.clone(),
            };
            let _ = global_sessions.add_session(&meta);
        }
        Some(sid)
    } else {
        let now = crate::util::now_ts_secs();
        let new_id = format!("sess-{}-{}", now, &uuid::Uuid::new_v4().to_string()[..8]);
        let meta = crate::state_fs::sessions::SessionMeta {
            id: new_id.clone(),
            title: auto_session_title(&req.message),
            created_at: now,
            skill: req.skill_name.clone(),
            creator: session_creator.into(),
            cwd: Some(project_root_str.clone()),
            project: None, project_name: None,
            mission_id: req.mission_id.clone(),
            model_id: req.model_id.clone(),
            user_id: req.user_id.clone(),
        };
        let _ = global_sessions.add_session(&meta);
        // Emit session_created so the unified session list updates in real-time
        let _ = state.events_tx.send(ServerEvent::SessionCreated {
            session_id: new_id.clone(),
            title: auto_session_title(&req.message),
            creator: session_creator.into(),
            project: Some(project_root_str.clone()),
            project_name: std::path::Path::new(&project_root_str)
                .file_name()
                .map(|n| n.to_string_lossy().to_string()),
            skill: req.skill_name.clone(),
            mission_id: req.mission_id.clone(),
        });

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

    match state.manager.get_or_create_session_agent(&effective_session_id, &root, &target_id).await {
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

                // Cancel any pending AskUser for this agent+session so the tool
                // unblocks immediately and the loop can pick up the new message.
                {
                    let mut pending = state.pending_ask_user.lock().await;
                    pending.retain(|_, entry| {
                        !(entry.agent_id == target_id
                            && entry.session_id.as_deref() == Some(&effective_session_id))
                    });
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
            let req_user_type = req.user_type;
            let req_model_id = req.model_id.clone();
            let req_images = req.images.clone();
            if was_busy {
                // Don't persist queued messages yet — they'll be persisted
                // when dequeued and processed (avoids showing them in chat
                // before the agent sees them).
            } else {
                // Emit + persist plain text. Images are ephemeral (sent inline
                // as base64 for the current turn only, not persisted to disk).
                persist_and_emit_to_store(
                    global_sessions,
                    &events_tx,
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
                    // Persist + emit the queued user message now that it's being processed.
                    persist_and_emit_to_store(
                        &state_clone.manager.global_sessions,
                        &events_tx_clone,
                        &target_id_clone,
                        "user",
                        &target_id_clone,
                        &clean_msg_clone,
                        session_id.as_deref(),
                        false,
                    )
                    .await;
                }

                // Apply session-level model override if provided.
                // When no override is sent (user selected "Default"), reset
                // model_id to the configured default so fallback state from a
                // previous turn doesn't persist.
                if let Some(ref mid) = req_model_id {
                    if engine.model_manager.has_model(mid) {
                        engine.model_id = mid.clone();
                        // Persist model choice to session metadata
                        if let Some(ref sid) = session_id {
                            if let Ok(Some(mut meta)) = state.manager.global_sessions.get_session_meta(sid) {
                                if meta.model_id.as_deref() != Some(mid) {
                                    meta.model_id = Some(mid.clone());
                                    let _ = state.manager.global_sessions.update_session_meta(&meta);
                                }
                            }
                        }
                    } else {
                        // The UI asked for a model that's no longer in config
                        // (renamed / removed / not yet reloaded). Fall back to
                        // the default instead of silently running on stale
                        // `engine.model_id`, and clear the stale pinning so the
                        // session stops requesting the dead id.
                        tracing::warn!(
                            "Session '{}' requested model '{}' which is not configured — falling back to default '{}'",
                            session_id.as_deref().unwrap_or("?"),
                            mid,
                            engine.default_model_id
                        );
                        engine.model_id = engine.default_model_id.clone();
                        if let Some(ref sid) = session_id {
                            if let Ok(Some(mut meta)) = state.manager.global_sessions.get_session_meta(sid) {
                                if meta.model_id.is_some() {
                                    meta.model_id = None;
                                    let _ = state.manager.global_sessions.update_session_meta(&meta);
                                }
                            }
                        }
                    }
                } else {
                    engine.model_id = engine.default_model_id.clone();
                }

                // Set session_id on engine tools so subagent delegations inherit it.
                engine.tools.builtins.set_session_id(session_id.clone());

                // Clear mission-only restrictions — user-initiated chats should
                // never be restricted by permission tiers (those apply only to
                // automated scheduler runs via apply_permission_tier).
                engine.cfg.mission_allowed_tools = None;
                engine.cfg.bash_allow_prefixes = None;

                // --- Session promotion: mission → user ---
                // The mission scheduler never goes through chat_handler (it calls
                // dispatch_mission_prompt directly), so any request here with a
                // mission session is from a real user taking over the conversation.
                if let Some(ref sid) = session_id {
                    if let Ok(Some(mut meta)) = state_clone.manager.global_sessions.get_session_meta(sid) {
                        if meta.creator == "mission" {
                            meta.creator = "user".to_string();
                            let _ = state_clone.manager.global_sessions.update_session_meta(&meta);

                            // Reset tool_permission_mode from Auto (forced by mission scheduler)
                            // back to the configured default.
                            let cfg = state_clone.manager.get_config_snapshot().await;
                            engine.cfg.tool_permission_mode = cfg.agent.tool_permission_mode;

                            // Force system prompt rebuild to reflect new permission context.
                            engine.cached_system_prompt = None;
                        }
                    }
                }

                // Apply session policy for proxy room consumers.
                let policy = crate::engine::session_policy::SessionPolicy::from_user_type(
                    &req_user_type,
                );
                policy.apply(&mut engine);

                let model_label = &engine.model_id;
                state_clone
                    .send_agent_status(
                        target_id_clone.clone(), AgentStatusKind::ModelLoading,
                        Some(format!("Loading model: {model_label}")),
                        None,
                        session_id.clone(),
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
                    policy,
                };

                // Restore chat history from session store when the engine was
                // freshly created (e.g. after model change invalidated cache).
                if engine.chat_history.is_empty() {
                    let sid = ctx.session_id.as_deref().unwrap_or("default");
                    let history_result = state.manager.global_sessions.get_chat_history(sid);
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
                        let bound_skill_name = match state.manager.global_sessions.get_session_meta(sid) {
                            Ok(meta) => meta.and_then(|m| m.skill),
                            Err(e) => {
                                tracing::warn!("Failed to read session meta for {}: {}", sid, e);
                                None
                            }
                        };
                        if let Some(ref skill_name) = bound_skill_name {
                            if ctx.policy.is_skill_allowed(skill_name) {
                                if let Some(skill) = manager.skill_manager.get_skill(skill_name).await {
                                    tracing::info!("Session-bound skill activated: {}", skill.name);
                                    // Ensure session_dir is populated so skill-permission
                                    // saves persist. Otherwise run_agent_loop later loads
                                    // permission.json from disk and clobbers in-memory
                                    // grants we're about to apply.
                                    if engine.session_dir.is_none() {
                                        if let Some(ref sid) = ctx.session_id {
                                            engine.session_dir =
                                                Some(crate::paths::global_sessions_dir().join(sid));
                                        }
                                    }
                                    // Apply skill's declared permission. Launching a skill's app
                                    // is implicit approval of its declared permissions — without
                                    // this, the skill runs at the session's default mode and its
                                    // SKILL.md permission block has no effect.
                                    if !engine.session_permissions.policy.is_locked() {
                                        if let Some(ref perm) = skill.permission {
                                            use crate::engine::permission::PermissionMode;
                                            let mode = match perm.mode.as_str() {
                                                "edit" => PermissionMode::Edit,
                                                "admin" => PermissionMode::Admin,
                                                _ => PermissionMode::Read,
                                            };
                                            let mut changed = false;
                                            for path in &perm.paths {
                                                let current = crate::engine::permission::effective_mode_for_path(
                                                    &engine.session_permissions.path_modes,
                                                    std::path::Path::new(path),
                                                );
                                                if current.as_ref() != Some(&mode) {
                                                    engine.session_permissions.set_path_mode(path, mode.clone());
                                                    changed = true;
                                                }
                                            }
                                            // Apply the skill's declared policy (or `strict` by
                                            // default) so out-of-scope actions deny silently
                                            // instead of prompting. See permission-spec.md §
                                            // Session policy.
                                            let want = perm
                                                .policy
                                                .as_deref()
                                                .map(crate::engine::permission::PermissionPolicy::from_preset)
                                                .unwrap_or_else(crate::engine::permission::PermissionPolicy::strict);
                                            if engine.session_permissions.policy != want {
                                                engine.session_permissions.policy = want;
                                                changed = true;
                                            }
                                            if changed {
                                                if let Some(ref sdir) = engine.session_dir {
                                                    engine.session_permissions.save(sdir);
                                                }
                                            }

                                            // NOTE: we intentionally do NOT seed session cwd to
                                            // perm.paths[0]. Earlier versions did this to make
                                            // Bash permission checks resolve to the granted path,
                                            // but that broke relative-path tools (Edit, Write)
                                            // because the model's mental model of cwd stays at
                                            // the workspace root. Bash permission is now handled
                                            // by `check_permission`'s 4c rule (scans the command
                                            // string for granted paths), which works regardless
                                            // of cwd. See permission-spec.md.
                                        }
                                    }
                                    engine.active_skill = Some(skill);
                                }
                            } else {
                                tracing::info!("Session-bound skill '{}' blocked by policy", skill_name);
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
                    session_id: session_id.clone(),
                    run_id: None,
                    parent_run_id: None,
                });
                state_clone
                    .send_agent_status(
                        target_id_clone.clone(), AgentStatusKind::Idle,
                        Some("Idle".to_string()),
                        None,
                        session_id.clone(),
                    )
                    .await;
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
    _root: &std::path::Path,
    agent_id: &str,
    session_id: Option<&str>,
) -> Option<crate::engine::Plan> {
    let sid = session_id.unwrap_or("default");
    let messages = state.manager.global_sessions.get_chat_history(sid).ok()?;
    // Scan backwards for the last plan message. Stop at the first plan found
    // regardless of status — if the most recent plan is "approved" or "completed",
    // there is no pending plan (don't keep scanning for older "planned" ones).
    for msg in messages.iter().rev() {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&msg.content) {
            if parsed.get("type").and_then(|v| v.as_str()) == Some("plan") {
                if let Some(plan_val) = parsed.get("plan") {
                    if let Ok(plan) = serde_json::from_value::<crate::engine::Plan>(plan_val.clone()) {
                        if plan.status == crate::engine::PlanStatus::Planned {
                            tracing::info!("[plan] Recovered pending plan from session history for {agent_id}");
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
    let plan = state
        .manager
        .take_pending_plan(&root_str, &req.agent_id, req.session_id.as_deref())
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

    let sid = session_id.as_deref().unwrap_or("default");
    let agent = match state.manager.get_or_create_session_agent(sid, &root, &req.agent_id).await {
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
        let mut engine = agent.lock().await;

        // Set the approved plan on the engine.
        engine.plan = Some(plan);
        engine.plan_mode = false;

        engine.observations.clear();
        engine.task = Some(format!(
            "Execute the approved plan: {}",
            engine.plan.as_ref().map(|p| p.summary.as_str()).unwrap_or("Plan")
        ));

        // Emit PlanUpdate SSE event and update the existing plan message in
        // messages.jsonl (instead of appending a duplicate).
        let plan_snapshot = engine.plan.clone().unwrap();
        engine.persist_and_emit_plan(plan_snapshot.clone()).await;
        let plan_json = serde_json::json!({ "type": "plan", "plan": plan_snapshot });
        let sid = session_id.as_deref().unwrap_or("default");
        let updated_msg = crate::state_fs::sessions::ChatMsg {
            agent_id: agent_id.clone(),
            from_id: agent_id.clone(),
            to_id: "user".to_string(),
            content: plan_json.to_string(),
            timestamp: crate::util::now_ts_secs(),
            is_observation: false,
        };
        if !manager.update_last_plan_message(sid, &updated_msg).await {
            // Fallback: append if no existing plan message found
            manager.add_chat_message(&root_clone, sid, &updated_msg).await;
        }

        // Build a ChatRunCtx so we can reuse run_plan_execution.
        // Plan execution only calls run_agent_loop (no skill dispatch), so
        // ctx.policy is not consulted. Engine-level restrictions (consumer_allowed_tools,
        // locked) are already set from the original chat request.
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
            policy: crate::engine::session_policy::SessionPolicy::default(),
        };

        run_plan_execution(&ctx, &mut engine).await;

        // Mark plan as completed and update the existing plan message in messages.jsonl.
        if let Some(ref mut plan) = engine.plan {
            if plan.status == crate::engine::PlanStatus::Executing
                || plan.status == crate::engine::PlanStatus::Approved
            {
                plan.status = crate::engine::PlanStatus::Completed;
                let plan_snapshot = plan.clone();
                engine.persist_and_emit_plan(plan_snapshot.clone()).await;
                let plan_json = serde_json::json!({ "type": "plan", "plan": plan_snapshot });
                let completed_msg = crate::state_fs::sessions::ChatMsg {
                    agent_id: agent_id.clone(),
                    from_id: agent_id.clone(),
                    to_id: "user".to_string(),
                    content: plan_json.to_string(),
                    timestamp: crate::util::now_ts_secs(),
                    is_observation: false,
                };
                let sid = session_id_for_cleanup.as_deref().unwrap_or("default");
                ctx.manager.update_last_plan_message(sid, &completed_msg).await;
            }
        }

        // Emit TurnComplete so the Web UI has a single finalizer.
        let _ = events_tx.send(ServerEvent::TurnComplete {
            agent_id: agent_id.clone(),
            duration_ms: None,
            context_tokens: None,
            parent_id: None,
            session_id: session_id_for_cleanup.clone(),
            run_id: None,
            parent_run_id: None,
        });
        state_clone
            .send_agent_status(agent_id.clone(), AgentStatusKind::Idle, Some("Idle".to_string()), None, session_id_for_cleanup.clone())
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
        .take_pending_plan(&root_str, &req.agent_id, req.session_id.as_deref())
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
        session_id: req.session_id.clone(),
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
        .edit_pending_plan(&root_str, &req.agent_id, req.session_id.as_deref(), &req.text)
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
            let session_id = entry.session_id.clone();
            if entry.sender.send(req.answers).is_ok() {
                // Broadcast so all clients (including remote) dismiss the widget.
                let _ = state.events_tx.send(crate::server::ServerEvent::WidgetResolved {
                    widget_id: req.question_id,
                    session_id,
                });
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
pub fn open_in_browser(url: &str) -> std::io::Result<()> {
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
