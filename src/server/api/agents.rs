use crate::extensions::agents::parse_agent_markdown;
use crate::server::chat::helpers::{emit_queue_updated, queue_key};
use crate::server::{AgentStatusKind, ServerEvent, ServerState};
use crate::state_fs::StateFile;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

use super::{canonical_project_root, ProjectQuery};

async fn first_patch_agent(state: &Arc<ServerState>, root: &PathBuf) -> Option<String> {
    // All agents are patch-capable; pick the first.
    let entries = state.manager.list_agent_specs(root).await.ok()?;
    entries.into_iter().next().map(|entry| entry.agent_id)
}

#[derive(Deserialize)]
pub(crate) struct TaskRequest {
    project_root: String,
    agent_id: String,
    task: String,
}

pub(crate) async fn set_task(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<TaskRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);

    match state
        .manager
        .get_or_create_session_agent("default", &root, &req.agent_id)
        .await
    {
        Ok(agent) => {
            let mut engine = agent.lock().await;
            engine.set_task(req.task.clone());

            // Persist planning task — all agents can finalize now.
            {
                if let Ok(ctx) = state.manager.get_or_create_project(root).await {
                    let planning_task = StateFile::PmTask {
                        id: format!("plan-{}", crate::util::now_ts_secs()),
                        status: "active".to_string(),
                        assigned_tasks: Vec::new(),
                    };
                    let _ = ctx
                        .state_fs
                        .write_file("active.md", &planning_task, &req.task);
                    let _ = state.events_tx.send(ServerEvent::StateUpdated);
                }
            }

            StatusCode::OK.into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct RunRequest {
    project_root: String,
    agent_id: String,
    session_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CancelRunRequest {
    run_id: String,
}

#[derive(Serialize)]
struct CancelRunResponse {
    status: String,
}

pub(crate) async fn run_agent(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<RunRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);
    let agent_id = req.agent_id.clone();
    let session_id = req.session_id.clone();
    let events_tx = state.events_tx.clone();
    let manager = state.manager.clone();
    let state_clone = state.clone();

    match state
        .manager
        .get_or_create_session_agent(req.session_id.as_deref().unwrap_or("default"), &root, &req.agent_id)
        .await
    {
        Ok(agent) => {
            tokio::spawn(async move {
                let run_id = match manager
                    .begin_agent_run(
                        &root,
                        session_id.as_deref(),
                        &agent_id,
                        None,
                        Some("api/run".to_string()),
                    )
                    .await
                {
                    Ok(id) => id,
                    Err(_) => format!("run-{}-fallback", agent_id),
                };
                state_clone
                    .send_agent_status(
                        agent_id.clone(),
                        AgentStatusKind::Working,
                        Some("Running".to_string()),
                        None,
                        None,
                    )
                    .await;
                let mut engine = agent.lock().await;
                engine.set_parent_agent(None);
                engine.set_run_id(Some(run_id.clone()));
                let run_result = engine.run_agent_loop(session_id.as_deref()).await;
                engine.set_run_id(None);
                let outcome = match run_result {
                    Ok(outcome) => {
                        let _ = manager
                            .finish_agent_run(
                                &run_id,
                                crate::engine::agent::AgentRunStatus::Completed,
                                None,
                            )
                            .await;
                        outcome
                    }
                    Err(err) => {
                        let msg = err.to_string();
                        let status = if msg.to_lowercase().contains("cancel") {
                            crate::engine::agent::AgentRunStatus::Cancelled
                        } else {
                            crate::engine::agent::AgentRunStatus::Failed
                        };
                        let _ = manager.finish_agent_run(&run_id, status, Some(msg)).await;
                        crate::engine::AgentOutcome::None
                    }
                };

                let _ = events_tx.send(ServerEvent::Outcome {
                    agent_id: agent_id.clone(),
                    outcome,
                    session_id: session_id.clone(),
                });
                state_clone
                    .send_agent_status(
                        agent_id.clone(),
                        AgentStatusKind::Idle,
                        Some("Idle".to_string()),
                        None,
                        None,
                    )
                    .await;
            });

            Json(serde_json::json!({ "status": "started" })).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct CancelToolRequest {
    block_id: String,
}

pub(crate) async fn cancel_tool_execution(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CancelToolRequest>,
) -> impl IntoResponse {
    let triggered = state.manager.trigger_tool_cancel(&req.block_id);
    Json(serde_json::json!({
        "status": if triggered { "cancelled" } else { "not_found" }
    }))
}

pub(crate) async fn cancel_agent_run(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CancelRunRequest>,
) -> impl IntoResponse {
    match state.manager.cancel_run_tree(&req.run_id).await {
        Ok(runs) => {
            // Cancel any pending AskUser questions for the cancelled runs so
            // the tool unblocks immediately. Dropping the sender causes the
            // oneshot receiver to return Err, which is handled gracefully.
            // Scope by (agent, session) — an agent-only match would nuke the
            // same agent's prompt in an unrelated session; entries without a
            // session fall back to the agent match. Each removal is announced
            // with WidgetResolved: removal alone left the prompt on screen,
            // since only that broadcast dismisses the widget.
            {
                let cancelled_sessions: std::collections::HashSet<(String, String)> = runs
                    .iter()
                    .map(|r| (r.agent_id.clone(), r.session_id.clone()))
                    .collect();
                let cancelled_agents: std::collections::HashSet<String> =
                    runs.iter().map(|r| r.agent_id.clone()).collect();
                let mut resolved: Vec<(String, Option<String>)> = Vec::new();
                let mut pending = state.pending_ask_user.lock().await;
                pending.retain(|qid, entry| {
                    let hit = match entry.session_id.as_ref() {
                        Some(sid) => cancelled_sessions
                            .contains(&(entry.agent_id.clone(), sid.clone())),
                        None => cancelled_agents.contains(&entry.agent_id),
                    };
                    if hit {
                        resolved.push((qid.clone(), entry.session_id.clone()));
                    }
                    !hit
                });
                drop(pending);
                for (widget_id, session_id) in resolved {
                    let _ = state
                        .events_tx
                        .send(ServerEvent::WidgetResolved { widget_id, session_id });
                }
            }

            for run in &runs {
                // Statuses are keyed `session_id|agent_id`; an Idle without the
                // session misses the keyed Busy entry and the chat's thinking
                // ticker stays on screen after the cancel.
                let session_id = Some(run.session_id.clone()).filter(|s| !s.is_empty());
                state
                    .send_agent_status_with_ids(
                        run.agent_id.clone(),
                        AgentStatusKind::Idle,
                        Some("Cancelled".to_string()),
                        None,
                        session_id,
                        Some(run.run_id.clone()),
                        run.parent_run_id.clone(),
                    )
                    .await;

                // Drain queued messages for this agent so they don't get stuck.
                // Without this, queued messages survive cancellation and block
                // new messages (the UI shows "agent is busy" permanently).
                let key = queue_key(&run.repo_path, &run.session_id, &run.agent_id);
                {
                    let mut guard = state.queued_chats.lock().await;
                    guard.remove(&key);
                }
                emit_queue_updated(&state, &run.repo_path, &run.session_id, &run.agent_id)
                    .await;
            }
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            Json(CancelRunResponse {
                status: "ok".to_string(),
            })
            .into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct ClearQueueRequest {
    project_root: String,
    session_id: String,
    agent_id: String,
}

/// Drop all messages queued behind a busy agent without cancelling its
/// in-flight run. Wired to the chat input's "Dismiss queue" button —
/// previously that only cleared the local UI store, leaving the server
/// queue intact and causing dismissed messages to fire later.
pub(crate) async fn clear_queued_messages(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ClearQueueRequest>,
) -> impl IntoResponse {
    let key = queue_key(&req.project_root, &req.session_id, &req.agent_id);
    {
        let mut guard = state.queued_chats.lock().await;
        guard.remove(&key);
    }
    emit_queue_updated(&state, &req.project_root, &req.session_id, &req.agent_id).await;
    Json(serde_json::json!({ "status": "ok" }))
}

// ---------------------------------------------------------------------------
// Agent CRUD: list/get/upsert/delete agent .md spec files
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct AgentsQuery {
    project_root: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct AgentFileQuery {
    project_root: String,
    path: String,
}

#[derive(Deserialize)]
pub(crate) struct UpsertAgentFileRequest {
    project_root: String,
    path: String,
    content: String,
}

#[derive(Deserialize)]
pub(crate) struct DeleteAgentFileRequest {
    project_root: String,
    path: String,
}

#[derive(Serialize)]
struct AgentFileListItem {
    agent_id: String,
    name: String,
    description: String,
    path: String,
}

#[derive(Serialize)]
struct AgentFileResponse {
    path: String,
    content: String,
    valid: bool,
    error: Option<String>,
}

#[derive(Serialize)]
struct AgentFileWriteResponse {
    path: String,
    agent_id: String,
}

fn normalize_agent_md_path(path: &str) -> Result<String, String> {
    let raw = path.trim().replace('\\', "/");
    if raw.is_empty() {
        return Err("path is required".to_string());
    }
    if raw.contains("..") {
        return Err("path must not contain '..'".to_string());
    }
    // Allow ~/... paths for global agents.
    if raw.starts_with("~/") {
        if !raw.to_ascii_lowercase().ends_with(".md") {
            return Err("agent files must end with .md".to_string());
        }
        return Ok(raw);
    }
    if raw.starts_with('/') {
        return Err("path must be a relative markdown path under agents/".to_string());
    }
    let rel = if raw.starts_with("agents/") {
        raw
    } else {
        format!("agents/{}", raw)
    };
    if !rel.to_ascii_lowercase().ends_with(".md") {
        return Err("agent files must end with .md".to_string());
    }
    if !rel
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '-' || c == '_' || c == '.')
    {
        return Err("path contains unsupported characters".to_string());
    }
    let suffix = rel.strip_prefix("agents/").unwrap_or("");
    if suffix.is_empty() || suffix.split('/').any(|seg| seg.is_empty()) {
        return Err("invalid agent markdown path".to_string());
    }
    Ok(rel)
}

/// Resolve an agent path to an absolute filesystem path. Handles both
/// project-relative paths (`agents/coder.md`) and global paths
/// (`~/.linggen/agents/coder.md`).
fn resolve_agent_path(root: &std::path::Path, rel: &str) -> PathBuf {
    if rel.starts_with("~/") {
        let home = dirs::home_dir().unwrap_or_default();
        home.join(&rel[2..])
    } else {
        root.join(rel)
    }
}

pub(crate) async fn list_agents_api(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<AgentsQuery>,
) -> impl IntoResponse {
    let root = query
        .project_root
        .as_deref()
        .map(canonical_project_root)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    match state.manager.list_agents(&root).await {
        Ok(agents) => Json(agents).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(crate) async fn list_agent_files_api(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ProjectQuery>,
) -> impl IntoResponse {
    let root = canonical_project_root(&query.project_root);
    match state.manager.list_agent_specs(&root).await {
        Ok(entries) => {
            let home_dir = dirs::home_dir().unwrap_or_default();
            let global_agents_dir = crate::paths::global_agents_dir();
            let items: Vec<AgentFileListItem> = entries
                .into_iter()
                .map(|entry| {
                    // Global agents (`~/.linggen/agents/...`) must be labeled
                    // with the `~/` form so `get_agent_file_api` resolves them
                    // via `resolve_agent_path`'s home branch instead of
                    // prefixing `agents/` and 404-ing. Check global FIRST —
                    // when project_root happens to be HOME, the project-strip
                    // would otherwise win and produce an unresolvable
                    // `.linggen/agents/...` path.
                    let path = if entry.spec_path.starts_with(&global_agents_dir) {
                        if let Ok(rel) = entry.spec_path.strip_prefix(&home_dir) {
                            format!("~/{}", rel.to_string_lossy())
                        } else {
                            entry.spec_path.to_string_lossy().to_string()
                        }
                    } else if let Ok(rel) = entry.spec_path.strip_prefix(&root) {
                        rel.to_string_lossy().to_string()
                    } else if let Ok(rel) = entry.spec_path.strip_prefix(&home_dir) {
                        format!("~/{}", rel.to_string_lossy())
                    } else {
                        entry.spec_path.to_string_lossy().to_string()
                    };
                    AgentFileListItem {
                        agent_id: entry.agent_id,
                        name: entry.spec.name,
                        description: entry.spec.description,
                        path,
                    }
                })
                .collect();
            Json(items).into_response()
        }
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

pub(crate) async fn get_agent_file_api(
    Query(query): Query<AgentFileQuery>,
) -> impl IntoResponse {
    let root = canonical_project_root(&query.project_root);
    let rel = match normalize_agent_md_path(&query.path) {
        Ok(path) => path,
        Err(err) => return (StatusCode::BAD_REQUEST, err).into_response(),
    };
    let full_path = resolve_agent_path(&root, &rel);
    let content = match std::fs::read_to_string(&full_path) {
        Ok(content) => content,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let parsed = parse_agent_markdown(&content);
    Json(AgentFileResponse {
        path: rel,
        content,
        valid: parsed.is_ok(),
        error: parsed.err().map(|e| e.to_string()),
    })
    .into_response()
}

pub(crate) async fn upsert_agent_file_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<UpsertAgentFileRequest>,
) -> impl IntoResponse {
    let root = canonical_project_root(&req.project_root);
    let rel = match normalize_agent_md_path(&req.path) {
        Ok(path) => path,
        Err(err) => return (StatusCode::BAD_REQUEST, err).into_response(),
    };
    let (spec, _) = match parse_agent_markdown(&req.content) {
        Ok(parsed) => parsed,
        Err(err) => return (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    };
    let full_path = resolve_agent_path(&root, &rel);
    if let Some(parent) = full_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
        }
    }
    if let Err(err) = std::fs::write(&full_path, &req.content) {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    if let Err(err) = state.manager.invalidate_agent_cache(&root, None).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    let _ = state.events_tx.send(ServerEvent::StateUpdated);
    Json(AgentFileWriteResponse {
        path: rel,
        agent_id: spec.name.trim().to_lowercase(),
    })
    .into_response()
}

pub(crate) async fn delete_agent_file_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<DeleteAgentFileRequest>,
) -> impl IntoResponse {
    let root = canonical_project_root(&req.project_root);
    let rel = match normalize_agent_md_path(&req.path) {
        Ok(path) => path,
        Err(err) => return (StatusCode::BAD_REQUEST, err).into_response(),
    };
    let full_path = resolve_agent_path(&root, &rel);
    if !full_path.exists() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(err) = std::fs::remove_file(&full_path) {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    if let Err(err) = state.manager.invalidate_agent_cache(&root, None).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    let _ = state.events_tx.send(ServerEvent::StateUpdated);
    StatusCode::OK.into_response()
}

#[derive(Deserialize)]
pub(crate) struct AgentRunsQuery {
    project_root: String,
    session_id: Option<String>,
}

pub(crate) async fn list_agent_runs_api(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<AgentRunsQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&query.project_root);
    match state
        .manager
        .list_agent_runs(&root, query.session_id.as_deref())
        .await
    {
        Ok(runs) => Json(runs).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Reload agents from disk by invalidating the agent cache.
pub(crate) async fn reload_agents(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let project_root = body.get("project_root").and_then(|v| v.as_str());
    if let Some(root) = project_root {
        let root_buf = std::path::PathBuf::from(root);
        let _ = state.manager.invalidate_agent_cache(&root_buf, None).await;
    }
    let _ = state.events_tx.send(ServerEvent::StateUpdated);
    Json(serde_json::json!({ "ok": true })).into_response()
}

