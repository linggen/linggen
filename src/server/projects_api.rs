use crate::config::AgentSpec;
use crate::server::chat_helpers::sanitize_message_for_ui;
use crate::server::{ServerEvent, ServerState};
use crate::skills::Skill;
use axum::{
    extract::{Json, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Deserialize)]
pub(crate) struct ProjectQuery {
    project_root: String,
    /// Max items to return (default: all).
    #[serde(default)]
    limit: Option<usize>,
    /// Skip this many items from the start.
    #[serde(default)]
    offset: Option<usize>,
}

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

fn canonical_project_root(project_root: &str) -> PathBuf {
    let expanded = if project_root == "~" {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(project_root))
    } else if project_root.starts_with("~/") {
        dirs::home_dir().unwrap_or_default().join(&project_root[2..])
    } else {
        PathBuf::from(project_root)
    };
    expanded.canonicalize().unwrap_or(expanded)
}

fn normalize_agent_md_path(path: &str) -> Result<String, String> {
    let raw = path.trim().replace('\\', "/");
    if raw.is_empty() {
        return Err("path is required".to_string());
    }
    if raw.contains("..") {
        return Err("path must not contain '..'".to_string());
    }
    // Allow ~/... paths for global agents
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
/// project-relative paths (`agents/coder.md`) and global paths (`~/.linggen/agents/coder.md`).
fn resolve_agent_path(root: &std::path::Path, rel: &str) -> PathBuf {
    if rel.starts_with("~/") {
        let home = dirs::home_dir().unwrap_or_default();
        home.join(&rel[2..])
    } else {
        root.join(rel)
    }
}

pub(crate) async fn list_projects(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    match state.manager.store.list_projects() {
        Ok(projects) => Json(projects).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
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
            let global_dir = crate::paths::global_agents_dir();
            let home_dir = dirs::home_dir().unwrap_or_default();
            let items: Vec<AgentFileListItem> = entries
                .into_iter()
                .map(|entry| {
                    let path = if let Ok(rel) = entry.spec_path.strip_prefix(&root) {
                        // Project-local agent: show relative to project root
                        rel.to_string_lossy().to_string()
                    } else if let Ok(rel) = entry.spec_path.strip_prefix(&home_dir) {
                        // Global agent: show as ~/.linggen/agents/...
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
            let _ = global_dir; // suppress unused warning
            Json(items).into_response()
        }
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

pub(crate) async fn get_agent_file_api(Query(query): Query<AgentFileQuery>) -> impl IntoResponse {
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
    let parsed = AgentSpec::from_markdown_content(&content);
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
    let (spec, _) = match AgentSpec::from_markdown_content(&req.content) {
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

#[derive(Deserialize)]
pub(crate) struct AgentChildrenQuery {
    run_id: String,
    project_root: Option<String>,
}

pub(crate) async fn list_agent_children_api(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<AgentChildrenQuery>,
) -> impl IntoResponse {
    match state.manager.list_agent_children(&query.run_id, query.project_root.as_deref()).await {
        Ok(runs) => Json(runs).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct AgentContextQuery {
    run_id: String,
    view: Option<String>, // "summary" | "raw"
    project_root: Option<String>,
}

#[derive(Serialize)]
struct AgentContextSummary {
    message_count: usize,
    user_messages: usize,
    agent_messages: usize,
    system_messages: usize,
    started_at: u64,
    ended_at: Option<u64>,
}

#[derive(Serialize)]
struct AgentContextMessage {
    agent_id: String,
    from_id: String,
    to_id: String,
    content: String,
    timestamp: u64,
    is_observation: bool,
}

#[derive(Serialize)]
struct AgentContextResponse {
    run: crate::project_store::AgentRunRecord,
    summary: AgentContextSummary,
    messages: Option<Vec<AgentContextMessage>>,
}

pub(crate) async fn get_agent_context_api(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<AgentContextQuery>,
) -> impl IntoResponse {
    let run = match state.manager.get_agent_run(&query.run_id, query.project_root.as_deref()).await {
        Ok(Some(run)) => run,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let root = canonical_project_root(&run.repo_path);
    let ctx = match state.manager.get_or_create_project(root).await {
        Ok(ctx) => ctx,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let all_messages = match ctx
        .sessions
        .get_chat_history(&run.session_id)
    {
        Ok(messages) => messages,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let end_ts = run.ended_at.unwrap_or(u64::MAX);
    let messages: Vec<AgentContextMessage> = all_messages
        .into_iter()
        .filter(|m| m.timestamp >= run.started_at && m.timestamp <= end_ts)
        .map(|m| AgentContextMessage {
            agent_id: m.agent_id,
            from_id: m.from_id,
            to_id: m.to_id,
            content: m.content,
            timestamp: m.timestamp,
            is_observation: m.is_observation,
        })
        .collect();

    let user_messages = messages.iter().filter(|m| m.from_id == "user").count();
    let system_messages = messages.iter().filter(|m| m.from_id == "system").count();
    let agent_messages = messages
        .len()
        .saturating_sub(user_messages)
        .saturating_sub(system_messages);
    let summary = AgentContextSummary {
        message_count: messages.len(),
        user_messages,
        agent_messages,
        system_messages,
        started_at: run.started_at,
        ended_at: run.ended_at,
    };

    let is_raw = query
        .view
        .as_deref()
        .map(|v| v.eq_ignore_ascii_case("raw"))
        .unwrap_or(false);

    let ui_messages: Vec<AgentContextMessage> = messages
        .into_iter()
        .filter_map(|mut m| {
            let cleaned = sanitize_message_for_ui(&m.from_id, &m.content)?;
            m.content = cleaned;
            Some(m)
        })
        .collect();

    Json(AgentContextResponse {
        run,
        summary,
        messages: if is_raw { Some(ui_messages) } else { None },
    })
    .into_response()
}

pub(crate) async fn list_models_api(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let models_guard = state.manager.models.read().await;
    let models: Vec<_> = models_guard.list_models().into_iter().cloned().collect();
    drop(models_guard);
    Json(models).into_response()
}

pub(crate) async fn list_skills(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let skills: Vec<Skill> = state.skill_manager.list_skills().await;
    Json(skills).into_response()
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
    let _ = state.events_tx.send(crate::server::ServerEvent::StateUpdated);
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

/// Reload skills from disk and invalidate agent caches so they pick up changes.
pub(crate) async fn reload_skills(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let project_root = body.get("project_root").and_then(|v| v.as_str());
    let root_path = project_root.map(std::path::Path::new);
    if let Err(err) = state.skill_manager.load_all(root_path).await {
        tracing::warn!("Failed to reload skills: {err}");
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    // Invalidate agent caches so engines pick up new skill metadata.
    if let Some(root) = project_root {
        let root_buf = std::path::PathBuf::from(root);
        let _ = state.manager.invalidate_agent_cache(&root_buf, None).await;
    }
    let _ = state.events_tx.send(crate::server::ServerEvent::StateUpdated);
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

// ---------------------------------------------------------------------------
// Skill-file CRUD (mirrors agent-file endpoints)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct SkillFileQuery {
    project_root: String,
    path: String,
}

#[derive(Deserialize)]
pub(crate) struct UpsertSkillFileRequest {
    project_root: String,
    path: String,
    content: String,
}

#[derive(Deserialize)]
pub(crate) struct DeleteSkillFileRequest {
    project_root: String,
    path: String,
}

#[derive(Serialize)]
struct SkillFileListItem {
    name: String,
    path: String,
    source: String,
}

#[derive(Serialize)]
struct SkillFileResponse {
    path: String,
    content: String,
    valid: bool,
    error: Option<String>,
}

const PROJECT_SKILL_PREFIXES: &[&str] = &[".linggen/skills/", ".claude/skills/", ".codex/skills/"];

fn normalize_skill_md_path(path: &str) -> Result<String, String> {
    let raw = path.trim().replace('\\', "/");
    if raw.is_empty() {
        return Err("path is required".to_string());
    }
    if raw.starts_with('/') || raw.contains("..") {
        return Err("path must be a relative markdown path under a skills/ directory".to_string());
    }
    // Accept paths already under any of the 3 project skill dirs
    let rel = if PROJECT_SKILL_PREFIXES.iter().any(|p| raw.starts_with(p)) {
        raw
    } else {
        format!(".linggen/skills/{}", raw)
    };
    if !rel.to_ascii_lowercase().ends_with(".md") {
        return Err("skill files must end with .md".to_string());
    }
    if !rel
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '-' || c == '_' || c == '.')
    {
        return Err("path contains unsupported characters".to_string());
    }
    // Extract suffix after the matched prefix
    let suffix = PROJECT_SKILL_PREFIXES
        .iter()
        .find_map(|p| rel.strip_prefix(p))
        .unwrap_or("");
    if suffix.is_empty() || suffix.split('/').any(|seg| seg.is_empty()) {
        return Err("invalid skill markdown path".to_string());
    }
    Ok(rel)
}

pub(crate) async fn list_skill_files_api(
    Query(query): Query<ProjectQuery>,
) -> impl IntoResponse {
    let root = canonical_project_root(&query.project_root);
    let mut items: Vec<SkillFileListItem> = Vec::new();

    for prefix in PROJECT_SKILL_PREFIXES {
        let skills_dir = root.join(prefix);
        if !skills_dir.exists() {
            continue;
        }
        let entries = match std::fs::read_dir(&skills_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "md") {
                let rel = path
                    .strip_prefix(&root)
                    .unwrap_or(path.as_path())
                    .to_string_lossy()
                    .to_string();
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                items.push(SkillFileListItem {
                    name,
                    path: rel,
                    source: "project".to_string(),
                });
            }
        }
    }

    items.sort_by(|a, b| a.name.cmp(&b.name));
    Json(items).into_response()
}

pub(crate) async fn get_skill_file_api(
    Query(query): Query<SkillFileQuery>,
) -> impl IntoResponse {
    let root = canonical_project_root(&query.project_root);
    let rel = match normalize_skill_md_path(&query.path) {
        Ok(path) => path,
        Err(err) => return (StatusCode::BAD_REQUEST, err).into_response(),
    };
    let full_path = resolve_agent_path(&root, &rel);
    let content = match std::fs::read_to_string(&full_path) {
        Ok(content) => content,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    // Validate frontmatter
    let valid = content.starts_with("---")
        && content.splitn(3, "---").count() >= 3
        && serde_yml::from_str::<serde_yml::Value>(
            content.splitn(3, "---").nth(1).unwrap_or(""),
        )
        .is_ok();
    Json(SkillFileResponse {
        path: rel,
        content,
        valid,
        error: if valid {
            None
        } else {
            Some("Invalid YAML frontmatter".to_string())
        },
    })
    .into_response()
}

pub(crate) async fn upsert_skill_file_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<UpsertSkillFileRequest>,
) -> impl IntoResponse {
    let root = canonical_project_root(&req.project_root);
    let rel = match normalize_skill_md_path(&req.path) {
        Ok(path) => path,
        Err(err) => return (StatusCode::BAD_REQUEST, err).into_response(),
    };
    // Validate frontmatter minimally
    if !req.content.starts_with("---") {
        return (StatusCode::BAD_REQUEST, "Skill must start with YAML frontmatter").into_response();
    }
    let full_path = root.join(&rel);
    if let Some(parent) = full_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
        }
    }
    if let Err(err) = std::fs::write(&full_path, &req.content) {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    // Reload skills cache
    if let Err(err) = state.skill_manager.load_all(Some(&root)).await {
        tracing::warn!("Failed to reload skills after write: {}", err);
    }
    let _ = state.events_tx.send(ServerEvent::StateUpdated);
    Json(serde_json::json!({ "path": rel })).into_response()
}

pub(crate) async fn delete_skill_file_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<DeleteSkillFileRequest>,
) -> impl IntoResponse {
    let root = canonical_project_root(&req.project_root);
    let rel = match normalize_skill_md_path(&req.path) {
        Ok(path) => path,
        Err(err) => return (StatusCode::BAD_REQUEST, err).into_response(),
    };
    let full_path = root.join(&rel);
    if !full_path.exists() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(err) = std::fs::remove_file(&full_path) {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    // Reload skills cache
    if let Err(err) = state.skill_manager.load_all(Some(&root)).await {
        tracing::warn!("Failed to reload skills after delete: {}", err);
    }
    let _ = state.events_tx.send(ServerEvent::StateUpdated);
    StatusCode::OK.into_response()
}

pub(crate) async fn list_sessions(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ProjectQuery>,
) -> impl IntoResponse {
    let root = canonical_project_root(&query.project_root);
    match state.manager.get_or_create_project(root).await {
        Ok(ctx) => {
            let total = ctx.sessions.count_sessions();
            match ctx.sessions.list_sessions_paginated(query.limit, query.offset) {
                Ok(sessions) => {
                    let api_sessions: Vec<serde_json::Value> = sessions
                        .into_iter()
                        .map(|s| {
                            serde_json::json!({
                                "id": s.id,
                                "repo_path": query.project_root,
                                "title": s.title,
                                "created_at": s.created_at,
                                "skill": s.skill,
                                "creator": s.creator,
                            })
                        })
                        .collect();
                    Json(serde_json::json!({
                        "sessions": api_sessions,
                        "total": total,
                    }))
                    .into_response()
                }
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct CreateSessionRequest {
    /// Required for user/project sessions, optional for skill sessions.
    #[serde(default)]
    project_root: Option<String>,
    title: String,
    #[serde(default)]
    skill: Option<String>,
}

pub(crate) async fn create_session(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CreateSessionRequest>,
) -> impl IntoResponse {
    let id = format!(
        "sess-{}-{}",
        crate::util::now_ts_secs(),
        &uuid::Uuid::new_v4().to_string()[..8]
    );
    let meta = crate::state_fs::sessions::SessionMeta {
        id: id.clone(),
        title: req.title,
        created_at: crate::util::now_ts_secs(),
        skill: req.skill.clone(),
        creator: if req.skill.is_some() { "skill".into() } else { "user".into() },
    };

    // Skill sessions go under ~/.linggen/skills/{name}/sessions/
    if let Some(ref skill_name) = req.skill {
        let sessions_dir = crate::paths::skill_sessions_dir(skill_name);
        let store = crate::state_fs::sessions::SessionStore::with_sessions_dir(sessions_dir);
        return match store.add_session(&meta) {
            Ok(_) => Json(serde_json::json!({ "id": id })).into_response(),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
    }

    // Project sessions go under ~/.linggen/projects/{encoded}/sessions/
    let Some(ref project_root) = req.project_root else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let root = canonical_project_root(project_root);
    match state.manager.get_or_create_project(root).await {
        Ok(ctx) => {
            match ctx.sessions.add_session(&meta) {
                Ok(_) => Json(serde_json::json!({ "id": id })).into_response(),
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Resolve a session for the TUI (or any client) to use.
/// Returns the most recent empty session, or creates a new one.
#[derive(Deserialize)]
pub(crate) struct ResolveSessionRequest {
    project_root: String,
}

pub(crate) async fn resolve_session_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ResolveSessionRequest>,
) -> impl IntoResponse {
    let root = canonical_project_root(&req.project_root);
    match state.manager.get_or_create_project(root).await {
        Ok(ctx) => {
            // Check for the most recent session with no messages (empty).
            // Only check the latest few sessions — no need to scan all.
            if let Ok(sessions) = ctx.sessions.list_sessions_paginated(Some(10), None) {
                for s in &sessions {
                    if !ctx.sessions.session_has_messages(&s.id) {
                        return Json(serde_json::json!({
                            "id": s.id,
                            "title": s.title,
                            "reused": true,
                        }))
                        .into_response();
                    }
                }
            }
            // No empty session found — create a new one.
            let now = crate::util::now_ts_secs();
            let new_id = format!(
                "sess-{}-{}",
                now,
                &uuid::Uuid::new_v4().to_string()[..8]
            );
            let meta = crate::state_fs::sessions::SessionMeta {
                id: new_id.clone(),
                title: "New Chat".to_string(),
                created_at: now,
                skill: None,
                creator: "user".into(),
            };
            let _ = ctx.sessions.add_session(&meta);
            Json(serde_json::json!({
                "id": new_id,
                "title": "New Chat",
                "reused": false,
            }))
            .into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct RemoveSessionRequest {
    project_root: String,
    session_id: String,
}

pub(crate) async fn remove_session_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<RemoveSessionRequest>,
) -> impl IntoResponse {
    let root = canonical_project_root(&req.project_root);
    match state.manager.get_or_create_project(root).await {
        Ok(ctx) => match ctx.sessions.remove_session(&req.session_id) {
            Ok(_) => StatusCode::OK,
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
        },
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

// ---------------------------------------------------------------------------
// Skill session endpoints (sessions stored under ~/.linggen/skills/{name}/sessions/)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct SkillSessionQuery {
    skill: String,
}

pub(crate) async fn list_skill_sessions(
    Query(query): Query<SkillSessionQuery>,
) -> impl IntoResponse {
    let sessions_dir = crate::paths::skill_sessions_dir(&query.skill);
    let store = crate::state_fs::sessions::SessionStore::with_sessions_dir(sessions_dir);
    match store.list_sessions() {
        Ok(sessions) => {
            let api_sessions: Vec<serde_json::Value> = sessions
                .into_iter()
                .map(|s| {
                    serde_json::json!({
                        "id": s.id,
                        "title": s.title,
                        "created_at": s.created_at,
                        "skill": s.skill,
                        "creator": s.creator,
                    })
                })
                .collect();
            Json(serde_json::json!({ "sessions": api_sessions })).into_response()
        }
        Err(_) => Json(serde_json::json!({ "sessions": [] })).into_response(),
    }
}

/// GET /api/skill-sessions/state — return messages for a skill session.
pub(crate) async fn get_skill_session_state(
    Query(query): Query<SkillSessionStateQuery>,
) -> impl IntoResponse {
    let Some(skill) = query.skill.filter(|s| !s.is_empty()) else {
        return Json(serde_json::json!({ "messages": [] })).into_response();
    };
    let Some(session_id) = query.session_id.filter(|s| !s.is_empty()) else {
        return Json(serde_json::json!({ "messages": [] })).into_response();
    };

    let store = crate::state_fs::SessionStore::with_sessions_dir(
        crate::paths::skill_sessions_dir(&skill),
    );
    let messages = store
        .get_chat_history(&session_id)
        .unwrap_or_default();

    let mapped: Vec<serde_json::Value> = messages
        .into_iter()
        .filter(|m| !m.is_observation)
        .filter_map(|m| {
            let cleaned =
                crate::server::chat_helpers::sanitize_message_for_ui(&m.from_id, &m.content)?;
            Some(serde_json::json!([
                {
                    "id": format!("msg-{}", m.timestamp),
                    "from": m.from_id,
                    "to": m.to_id,
                    "ts": m.timestamp,
                    "task_id": null
                },
                cleaned
            ]))
        })
        .collect();

    Json(serde_json::json!({
        "active_task": null,
        "user_stories": null,
        "tasks": [],
        "messages": mapped
    }))
    .into_response()
}

#[derive(Deserialize)]
pub(crate) struct SkillSessionStateQuery {
    #[serde(default)]
    skill: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct RemoveSkillSessionRequest {
    skill: String,
    session_id: String,
}

pub(crate) async fn remove_skill_session_api(
    Json(req): Json<RemoveSkillSessionRequest>,
) -> impl IntoResponse {
    let sessions_dir = crate::paths::skill_sessions_dir(&req.skill);
    let store = crate::state_fs::sessions::SessionStore::with_sessions_dir(sessions_dir);
    match store.remove_session(&req.session_id) {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[derive(Deserialize)]
pub(crate) struct RenameSessionRequest {
    project_root: String,
    session_id: String,
    title: String,
}

pub(crate) async fn rename_session_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<RenameSessionRequest>,
) -> impl IntoResponse {
    let root = canonical_project_root(&req.project_root);
    match state.manager.get_or_create_project(root).await {
        Ok(ctx) => match ctx.sessions.rename_session(&req.session_id, &req.title) {
            Ok(_) => StatusCode::OK,
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
        },
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[derive(Deserialize)]
pub(crate) struct AddProjectRequest {
    path: String,
}

pub(crate) async fn add_project(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<AddProjectRequest>,
) -> impl IntoResponse {
    let path = PathBuf::from(&req.path);
    match state.manager.get_or_create_project(path).await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub(crate) async fn remove_project(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<AddProjectRequest>, // Reuse same struct for path
) -> impl IntoResponse {
    match state.manager.store.remove_project(&req.path) {
        Ok(_) => {
            // Also remove from active projects map
            let mut projects = state.manager.projects.lock().await;
            projects.remove(&req.path);
            StatusCode::OK
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

// ── Status endpoint ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct StatusQuery {
    project_root: String,
    session_id: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct StatusResponse {
    pub version: String,
    pub sessions: usize,
    pub total_runs: usize,
    pub completed_runs: usize,
    pub failed_runs: usize,
    pub cancelled_runs: usize,
    pub active_days: usize,
    pub first_run_at: Option<u64>,
    pub last_run_at: Option<u64>,
    pub model_usage: Vec<(String, usize)>,
    pub default_model: Option<String>,
    pub models: Vec<StatusModelInfo>,
    /// Accumulated prompt tokens this session (in-memory).
    pub session_prompt_tokens: usize,
    /// Accumulated completion tokens this session (in-memory).
    pub session_completion_tokens: usize,
}

#[derive(Serialize)]
pub(crate) struct StatusModelInfo {
    pub id: String,
    pub provider: String,
    pub model: String,
}

pub(crate) async fn get_status_api(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<StatusQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&query.project_root);

    // Gather runs
    let runs = state
        .manager
        .list_agent_runs(&root, None)
        .await
        .unwrap_or_default();

    let total_runs = runs.len();
    let mut completed = 0usize;
    let mut failed = 0usize;
    let mut cancelled = 0usize;
    let mut day_set = std::collections::HashSet::new();
    let mut first_run_at: Option<u64> = None;
    let mut last_run_at: Option<u64> = None;
    let mut model_count: HashMap<String, usize> = HashMap::new();

    for r in &runs {
        match r.status {
            crate::project_store::AgentRunStatus::Completed => completed += 1,
            crate::project_store::AgentRunStatus::Failed => failed += 1,
            crate::project_store::AgentRunStatus::Cancelled => cancelled += 1,
            _ => {}
        }
        let secs = r.started_at;
        let day = secs / 86400;
        day_set.insert(day);
        if first_run_at.is_none() || secs < first_run_at.unwrap() {
            first_run_at = Some(secs);
        }
        if last_run_at.is_none() || secs > last_run_at.unwrap() {
            last_run_at = Some(secs);
        }
        if let Some(kind) = &r.agent_kind {
            *model_count.entry(kind.clone()).or_default() += 1;
        }
    }

    let mut model_usage: Vec<(String, usize)> = model_count.into_iter().collect();
    model_usage.sort_by(|a, b| b.1.cmp(&a.1));

    // Sessions count
    let sessions = match state.manager.get_or_create_project(
        root.canonicalize().unwrap_or(root),
    ).await {
        Ok(ctx) => ctx.sessions.list_sessions().unwrap_or_default().len(),
        Err(_) => 0,
    };

    // Default model
    let config = state.manager.get_config_snapshot().await;
    let default_model = config.routing.default_models.first().cloned();

    // Available models
    let models_guard = state.manager.models.read().await;
    let models: Vec<StatusModelInfo> = models_guard
        .list_models()
        .iter()
        .map(|m| StatusModelInfo {
            id: m.id.clone(),
            provider: m.provider.clone(),
            model: m.model.clone(),
        })
        .collect();
    drop(models_guard);

    // Session token accumulation (in-memory)
    let (session_prompt_tokens, session_completion_tokens) = {
        let tokens = state.session_tokens.lock().await;
        if let Some(sid) = &query.session_id {
            tokens.get(sid).copied().unwrap_or((0, 0))
        } else {
            // Sum all sessions
            tokens.values().fold((0, 0), |acc, v| (acc.0 + v.0, acc.1 + v.1))
        }
    };

    Json(StatusResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        sessions,
        total_runs,
        completed_runs: completed,
        failed_runs: failed,
        cancelled_runs: cancelled,
        active_days: day_set.len(),
        first_run_at,
        last_run_at,
        model_usage,
        default_model,
        models,
        session_prompt_tokens,
        session_completion_tokens,
    })
    .into_response()
}

// ---------------------------------------------------------------------------
// Unified session delete — routes to the correct session store
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct DeleteUnifiedSessionRequest {
    session_id: String,
    /// For project sessions — which project owns it.
    #[serde(default)]
    project: Option<String>,
    /// For mission sessions — which mission owns it.
    #[serde(default)]
    mission_id: Option<String>,
    /// For skill sessions — which skill owns it.
    #[serde(default)]
    skill: Option<String>,
}

/// DELETE /api/sessions/all — delete a session from any store.
pub(crate) async fn delete_unified_session(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<DeleteUnifiedSessionRequest>,
) -> impl IntoResponse {
    let result = if let Some(ref mission_id) = req.mission_id {
        // Mission session
        let store = crate::state_fs::SessionStore::with_sessions_dir(
            crate::paths::mission_sessions_dir(mission_id),
        );
        store.remove_session(&req.session_id)
    } else if let Some(ref skill) = req.skill {
        // Skill session
        let store = crate::state_fs::SessionStore::with_sessions_dir(
            crate::paths::skill_sessions_dir(skill),
        );
        store.remove_session(&req.session_id)
    } else if let Some(ref project) = req.project {
        // Project session
        let root = canonical_project_root(project);
        match state.manager.get_or_create_project(root).await {
            Ok(ctx) => ctx.sessions.remove_session(&req.session_id),
            Err(e) => Err(e),
        }
    } else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    match result {
        Ok(_) => StatusCode::OK.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

// ---------------------------------------------------------------------------
// Unified session list — all sessions from all sources
// ---------------------------------------------------------------------------

/// GET /api/sessions/all — return sessions from projects, missions, and skills.
pub(crate) async fn list_all_sessions(
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    let mut all: Vec<serde_json::Value> = Vec::new();

    // 1. Project sessions
    if let Ok(projects) = state.manager.store.list_projects() {
        for project in &projects {
            let root = canonical_project_root(&project.path);
            if let Ok(ctx) = state.manager.get_or_create_project(root).await {
                if let Ok(sessions) = ctx.sessions.list_sessions() {
                    let proj_name = std::path::Path::new(&project.path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| project.path.clone());
                    for s in sessions {
                        all.push(serde_json::json!({
                            "id": s.id,
                            "title": s.title,
                            "created_at": s.created_at,
                            "creator": s.creator,
                            "project": project.path,
                            "project_name": proj_name,
                            "skill": s.skill,
                            "mission_id": serde_json::Value::Null,
                        }));
                    }
                }
            }
        }
    }

    // 2. Mission sessions
    let missions_dir = crate::paths::global_missions_dir();
    if missions_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&missions_dir) {
            for entry in entries.flatten() {
                if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    continue;
                }
                let mission_id = entry.file_name().to_string_lossy().to_string();
                if mission_id == "sessions" {
                    continue; // skip legacy global sessions dir
                }
                // Load mission config to get its project path
                let mission_store = crate::project_store::missions::MissionStore::new();
                let mission_project = mission_store.get_mission(&mission_id)
                    .ok()
                    .flatten()
                    .and_then(|m| m.project);
                let mission_project_name = mission_project.as_ref().and_then(|p| {
                    std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string())
                });
                let store = crate::state_fs::SessionStore::with_sessions_dir(
                    crate::paths::mission_sessions_dir(&mission_id),
                );
                if let Ok(sessions) = store.list_sessions() {
                    for s in sessions {
                        all.push(serde_json::json!({
                            "id": s.id,
                            "title": s.title,
                            "created_at": s.created_at,
                            "creator": s.creator,
                            "project": mission_project,
                            "project_name": mission_project_name,
                            "skill": s.skill,
                            "mission_id": mission_id,
                        }));
                    }
                }
            }
        }
    }

    // 3. Skill sessions
    let skills_dir = crate::paths::global_skills_dir();
    if skills_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&skills_dir) {
            for entry in entries.flatten() {
                if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    continue;
                }
                let skill_name = entry.file_name().to_string_lossy().to_string();
                let sessions_dir = crate::paths::skill_sessions_dir(&skill_name);
                if !sessions_dir.exists() {
                    continue;
                }
                let store = crate::state_fs::SessionStore::with_sessions_dir(sessions_dir);
                if let Ok(sessions) = store.list_sessions() {
                    for s in sessions {
                        all.push(serde_json::json!({
                            "id": s.id,
                            "title": s.title,
                            "created_at": s.created_at,
                            "creator": "skill",
                            "project": serde_json::Value::Null,
                            "project_name": serde_json::Value::Null,
                            "skill": skill_name,
                            "mission_id": serde_json::Value::Null,
                        }));
                    }
                }
            }
        }
    }

    // Sort by created_at descending (newest first)
    all.sort_by(|a, b| {
        let ta = a["created_at"].as_u64().unwrap_or(0);
        let tb = b["created_at"].as_u64().unwrap_or(0);
        tb.cmp(&ta)
    });

    Json(serde_json::json!({ "sessions": all })).into_response()
}
