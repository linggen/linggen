use crate::config::AgentSpec;
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
    crate::util::resolve_path(&expanded)
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
    // Clear per-session engines so they get recreated with new skills.
    state.manager.session_engines.lock().await.clear();
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
    match state.manager.global_sessions.list_sessions() {
        Ok(all_sessions) => {
            // Filter by project_root: match sessions whose cwd or project starts with the query path.
            let canonical = canonical_project_root(&query.project_root);
            let canonical_str = canonical.to_string_lossy();
            let filtered: Vec<_> = all_sessions.into_iter().filter(|s| {
                s.cwd.as_deref().map(|c| c.starts_with(canonical_str.as_ref())).unwrap_or(false)
                    || s.project.as_deref().map(|p| p.starts_with(canonical_str.as_ref())).unwrap_or(false)
            }).collect();
            let total = filtered.len();
            // Apply pagination
            let offset = query.offset.unwrap_or(0);
            let limit = query.limit.unwrap_or(50);
            let paginated: Vec<_> = filtered.into_iter().skip(offset).take(limit).collect();
            let api_sessions: Vec<serde_json::Value> = paginated
                .into_iter()
                .map(|s| {
                    serde_json::json!({
                        "id": s.id,
                        "repo_path": s.cwd.as_deref().unwrap_or(&query.project_root),
                        "title": s.title,
                        "created_at": s.created_at,
                        "skill": s.skill,
                        "creator": s.creator,
                        "project": s.project,
                        "project_name": s.project_name,
                        "cwd": s.cwd,
                        "mission_id": s.mission_id,
                        "model_id": s.model_id,
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

#[derive(Deserialize)]
pub(crate) struct CreateSessionRequest {
    /// Required for user/project sessions, optional for skill sessions.
    #[serde(default)]
    project_root: Option<String>,
    title: String,
    #[serde(default)]
    skill: Option<String>,
    /// User ID of the session creator (injected by peer.rs).
    #[serde(default)]
    user_id: Option<String>,
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
    let cwd = req.project_root.as_deref()
        .map(|p| canonical_project_root(p).to_string_lossy().to_string());
    let meta = crate::state_fs::sessions::SessionMeta {
        id: id.clone(),
        title: req.title,
        created_at: crate::util::now_ts_secs(),
        skill: req.skill.clone(),
        creator: if req.skill.is_some() { "skill".into() } else { "user".into() },
        cwd, project: None, project_name: None, mission_id: None, model_id: None, user_id: req.user_id,
    };

    // All sessions go to the global flat store
    match state.manager.global_sessions.add_session(&meta) {
        Ok(_) => Json(serde_json::json!({ "id": id })).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Resolve a session for a client to use.
/// Returns the most recent empty session, or creates a new one.
#[derive(Deserialize)]
pub(crate) struct ResolveSessionRequest {
    project_root: String,
}

pub(crate) async fn resolve_session_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ResolveSessionRequest>,
) -> impl IntoResponse {
    let store = &state.manager.global_sessions;
    // Check for the most recent session with no messages (empty).
    if let Ok(sessions) = store.list_sessions_paginated(Some(10), None) {
        for s in &sessions {
            if !store.session_has_messages(&s.id) {
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
        cwd: Some(req.project_root.clone()), project: None, project_name: None, mission_id: None, model_id: None, user_id: None,
    };
    let _ = store.add_session(&meta);
    Json(serde_json::json!({
        "id": new_id,
        "title": "New Chat",
        "reused": false,
    }))
    .into_response()
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
    state.manager.remove_session_engine(&req.session_id).await;
    match state.manager.global_sessions.remove_session(&req.session_id) {
        Ok(_) => StatusCode::OK,
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
    State(state): State<Arc<ServerState>>,
    Query(query): Query<SkillSessionQuery>,
) -> impl IntoResponse {
    match state.manager.global_sessions.list_sessions() {
        Ok(sessions) => {
            let api_sessions: Vec<serde_json::Value> = sessions
                .into_iter()
                .filter(|s| s.skill.as_deref() == Some(&query.skill))
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
    State(state): State<Arc<ServerState>>,
    Query(query): Query<SkillSessionStateQuery>,
) -> impl IntoResponse {
    let Some(_skill) = query.skill.filter(|s| !s.is_empty()) else {
        return Json(serde_json::json!({ "messages": [] })).into_response();
    };
    let Some(session_id) = query.session_id.filter(|s| !s.is_empty()) else {
        return Json(serde_json::json!({ "messages": [] })).into_response();
    };

    let messages = state.manager.global_sessions
        .get_chat_history(&session_id)
        .unwrap_or_default();

    let mapped: Vec<serde_json::Value> = messages
        .into_iter()
        .filter(|m| !m.is_observation)
        .filter(|m| !m.content.contains("[HIDDEN]"))
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
    State(state): State<Arc<ServerState>>,
    Json(req): Json<RemoveSkillSessionRequest>,
) -> impl IntoResponse {
    state.manager.remove_session_engine(&req.session_id).await;
    match state.manager.global_sessions.remove_session(&req.session_id) {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[derive(Deserialize)]
pub(crate) struct RenameSessionRequest {
    project_root: String,
    session_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    model_id: Option<String>,
}

pub(crate) async fn rename_session_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<RenameSessionRequest>,
) -> impl IntoResponse {
    // Update title if provided
    if let Some(ref title) = req.title {
        if let Err(_) = state.manager.global_sessions.rename_session(&req.session_id, title) {
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    }
    // Update model_id if provided
    if let Some(ref model_id) = req.model_id {
        if let Ok(Some(mut meta)) = state.manager.global_sessions.get_session_meta(&req.session_id) {
            let new_val = if model_id.is_empty() { None } else { Some(model_id.clone()) };
            if meta.model_id != new_val {
                meta.model_id = new_val;
                let _ = state.manager.global_sessions.update_session_meta(&meta);
            }
        }
    }
    let _ = state.events_tx.send(crate::server::ServerEvent::StateUpdated);
    StatusCode::OK
}

// ---------------------------------------------------------------------------
// Session permission endpoints (permission-spec.md)
// ---------------------------------------------------------------------------

/// GET /api/sessions/permission?session_id=...&cwd=...
/// Returns the session's permission.json contents plus `effective_mode` for the given cwd.
pub(crate) async fn get_session_permission(
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let session_id = match params.get("session_id") {
        Some(id) => id,
        None => return (StatusCode::BAD_REQUEST, "Missing session_id".to_string()).into_response(),
    };
    let session_dir = crate::paths::global_sessions_dir().join(session_id);
    let perms = crate::engine::permission::SessionPermissions::load(&session_dir);

    // Compute effective mode for cwd if provided.
    let effective_mode = params.get("cwd").and_then(|cwd| {
        crate::engine::permission::effective_mode_for_path(
            &perms.path_modes,
            std::path::Path::new(cwd),
        )
    });

    // Build response with both perms and effective_mode.
    let mut resp = match serde_json::to_value(&perms) {
        Ok(v) => v,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    if let Some(mode) = effective_mode {
        resp.as_object_mut().map(|m| m.insert(
            "effective_mode".to_string(),
            serde_json::Value::String(mode.to_string()),
        ));
    }
    // Include zone so UI can disable mode switching for system paths.
    if let Some(cwd) = params.get("cwd") {
        let zone = crate::engine::permission::path_zone(std::path::Path::new(cwd));
        let zone_str = match zone {
            crate::engine::permission::PathZone::Home => "home",
            crate::engine::permission::PathZone::Temp => "temp",
            crate::engine::permission::PathZone::System => "system",
        };
        resp.as_object_mut().map(|m| m.insert(
            "zone".to_string(),
            serde_json::Value::String(zone_str.to_string()),
        ));
    }
    match serde_json::to_string(&resp) {
        Ok(json) => (StatusCode::OK, json).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct UpdatePermissionRequest {
    session_id: String,
    path: String,
    mode: String,
}

/// PATCH /api/sessions/permission
/// Updates the mode for a specific path in the session's permission.json.
pub(crate) async fn update_session_permission(
    State(state): State<std::sync::Arc<crate::server::ServerState>>,
    Json(req): Json<UpdatePermissionRequest>,
) -> impl IntoResponse {
    use crate::engine::permission::{PermissionMode, SessionPermissions};

    let mode = match req.mode.as_str() {
        "chat" => PermissionMode::Chat,
        "read" => PermissionMode::Read,
        "edit" => PermissionMode::Edit,
        "admin" => PermissionMode::Admin,
        _ => return StatusCode::BAD_REQUEST,
    };

    // Block edit/admin mode on system zone paths (per permission-spec.md).
    if mode > PermissionMode::Read {
        let expanded = if req.path.starts_with("~/") {
            dirs::home_dir()
                .map(|h| h.join(&req.path[2..]))
                .unwrap_or_else(|| PathBuf::from(&req.path))
        } else {
            PathBuf::from(&req.path)
        };
        let zone = crate::engine::permission::path_zone(&expanded);
        if zone == crate::engine::permission::PathZone::System {
            return StatusCode::FORBIDDEN;
        }
    }

    let session_dir = crate::paths::global_sessions_dir().join(&req.session_id);
    let mut perms = SessionPermissions::load(&session_dir);
    perms.set_path_mode(&req.path, mode);
    perms.save(&session_dir);

    // Notify UI of state change
    let _ = state.events_tx.send(crate::server::ServerEvent::StateUpdated);
    StatusCode::OK
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
    let sessions = state.manager.global_sessions.count_sessions();

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

/// DELETE /api/sessions/all — delete a session from the global store.
pub(crate) async fn delete_unified_session(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<DeleteUnifiedSessionRequest>,
) -> impl IntoResponse {
    state.manager.remove_session_engine(&req.session_id).await;
    match state.manager.global_sessions.remove_session(&req.session_id) {
        Ok(_) => StatusCode::OK.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

// ---------------------------------------------------------------------------
// Unified session list — all sessions from all sources
// ---------------------------------------------------------------------------

/// GET /api/sessions/all — return all sessions from the global flat store.
pub(crate) async fn list_all_sessions(
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    match state.manager.global_sessions.list_sessions() {
        Ok(sessions) => {
            let all: Vec<serde_json::Value> = sessions
                .into_iter()
                .map(|s| {
                    serde_json::json!({
                        "id": s.id,
                        "title": s.title,
                        "created_at": s.created_at,
                        "creator": s.creator,
                        "project": s.project,
                        "project_name": s.project_name,
                        "skill": s.skill,
                        "mission_id": s.mission_id,
                        "cwd": s.cwd,
                        "model_id": s.model_id,
                    })
                })
                .collect();
            Json(serde_json::json!({ "sessions": all })).into_response()
        }
        Err(_) => Json(serde_json::json!({ "sessions": [] })).into_response(),
    }
}

// ── User profile (from linggen.dev) ─────────────────────────────────────

/// GET /api/user/me — fetch the authenticated user's profile from linggen.dev.
/// Reads the API token from `~/.linggen/remote.toml` and proxies to the relay.
pub(crate) async fn get_user_me() -> impl IntoResponse {
    let config = match crate::cli::login::load_remote_config() {
        Some(c) => c,
        None => return (StatusCode::NOT_FOUND, "Not logged in").into_response(),
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let resp = client
        .get(format!("{}/api/auth/me", config.relay_url))
        .header("Authorization", format!("Bearer {}", config.api_token))
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            match r.json::<serde_json::Value>().await {
                Ok(body) => {
                    // Update remote.toml with fresh user info so UserContext stays current
                    let new_name = body.get("display_name").and_then(|v| v.as_str()).map(|s| s.to_string());
                    let new_avatar = body.get("avatar_url").and_then(|v| v.as_str()).map(|s| s.to_string());
                    if new_name != config.user_name || new_avatar != config.avatar_url {
                        let mut updated = config;
                        updated.user_name = new_name;
                        updated.avatar_url = new_avatar;
                        if let Ok(toml_str) = toml::to_string_pretty(&updated) {
                            let path = crate::paths::linggen_home().join("remote.toml");
                            let _ = std::fs::write(&path, &toml_str);
                        }
                    }
                    Json(body).into_response()
                }
                Err(_) => (StatusCode::BAD_GATEWAY, "Invalid response").into_response(),
            }
        }
        Ok(r) => (StatusCode::from_u16(r.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY), "Auth failed").into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("Relay error: {}", e)).into_response(),
    }
}

/// GET /api/auth/login — redirect to linggen.dev OAuth with callback to this server.
pub(crate) async fn auth_login(
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let host = params.get("host").cloned().unwrap_or_else(|| {
        let port = params.get("port").and_then(|p| p.parse::<u16>().ok()).unwrap_or(9898);
        format!("localhost:{}", port)
    });
    let callback = format!("http://{}/api/auth/callback", host);
    let state = uuid::Uuid::new_v4().to_string();
    let prompt = params.get("prompt").cloned().unwrap_or_default();
    let url = format!(
        "https://linggen.dev/auth/link?callback={}&state={}&prompt={}",
        urlencoding::encode(&callback),
        urlencoding::encode(&state),
        urlencoding::encode(&prompt),
    );
    axum::response::Redirect::temporary(&url).into_response()
}

/// GET /api/auth/callback — receives token from linggen.dev OAuth redirect.
pub(crate) async fn auth_callback(
    State(state): State<Arc<ServerState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let token = match params.get("token") {
        Some(t) if t.starts_with("usr_") => t.clone(),
        _ => {
            return axum::response::Html(
                "<html><body><h2>Authentication failed</h2><p>No valid token received.</p></body></html>"
                    .to_string(),
            )
            .into_response()
        }
    };

    // Save config (same as `ling login`)
    let instance_id = crate::cli::login::get_or_create_instance_id().unwrap_or_else(|_| "unknown".into());
    let instance_name = gethostname::gethostname().to_string_lossy().to_string();

    // Register instance with linggen.dev
    let client = reqwest::Client::new();
    let user_id = match client
        .post("https://linggen.dev/api/instances")
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "instance_id": instance_id,
            "name": instance_name,
        }))
        .send()
        .await
    {
        Ok(resp) => resp.json::<serde_json::Value>().await.ok()
            .and_then(|v| v.get("user_id").and_then(|u| u.as_str()).map(|s| s.to_string())),
        Err(_) => None,
    };

    let config = crate::cli::login::RemoteConfig {
        relay_url: "https://linggen.dev".to_string(),
        api_token: token,
        instance_name,
        instance_id,
        user_id,
        user_name: None,
        avatar_url: None,
    };
    let path = crate::paths::linggen_home().join("remote.toml");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let toml_str = toml::to_string_pretty(&config).unwrap_or_default();
    let _ = std::fs::write(&path, &toml_str);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }

    // Restart relay to pick up the new config
    let _ = state.events_tx.send(crate::server::ServerEvent::StateUpdated);

    axum::response::Html(
        r#"<html><body><h2>Authenticated!</h2><p>You can close this tab.</p><script>window.opener&&window.opener.postMessage({type:'linggen-auth-done'},'*');window.close()</script></body></html>"#.to_string()
    ).into_response()
}

/// POST /api/auth/logout — remove remote.toml to log out.
pub(crate) async fn auth_logout() -> impl IntoResponse {
    let path = crate::paths::linggen_home().join("remote.toml");
    if path.exists() {
        let _ = std::fs::remove_file(&path);
        Json(serde_json::json!({ "ok": true })).into_response()
    } else {
        Json(serde_json::json!({ "ok": true, "message": "Not logged in" })).into_response()
    }
}

/// GET /api/room-config — get local room config (shared models, allowed tools).
pub(crate) async fn get_room_config() -> impl IntoResponse {
    let config = crate::server::rtc::room_config::load_room_config();
    Json(serde_json::json!({
        "shared_models": config.shared_models,
        "allowed_tools": config.allowed_tools,
        "allowed_skills": config.allowed_skills,
        "room_enabled": config.room_enabled,
        "auto_connect": config.auto_connect,
    }))
}

/// POST /api/room-config — update local room config (shared models).
pub(crate) async fn update_room_config(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Load current config as base, then merge incoming fields
    let mut config = crate::server::rtc::room_config::load_room_config();
    let was_enabled = config.room_enabled;

    if let Some(v) = body.get("shared_models") {
        config.shared_models = serde_json::from_value(v.clone()).unwrap_or_default();
    }
    if let Some(v) = body.get("allowed_tools") {
        config.allowed_tools = serde_json::from_value(v.clone()).unwrap_or_default();
    }
    if let Some(v) = body.get("allowed_skills") {
        config.allowed_skills = serde_json::from_value(v.clone()).unwrap_or_default();
    }
    if let Some(v) = body.get("room_enabled") {
        config.room_enabled = v.as_bool().unwrap_or(true);
    }
    if let Some(v) = body.get("auto_connect") {
        config.auto_connect = v.as_bool().unwrap_or(true);
    }
    if let Some(v) = body.get("token_budget_room_daily") {
        config.token_budget_room_daily = v.as_i64();
    }
    if let Some(v) = body.get("token_budget_consumer_daily") {
        config.token_budget_consumer_daily = v.as_i64();
    }
    if let Err(e) = crate::server::rtc::room_config::save_room_config(&config) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": format!("{e}") }))).into_response();
    }

    // Sync room status to linggen.dev DB
    let room_enabled_changed = was_enabled != config.room_enabled;
    if room_enabled_changed {
        let status = if config.room_enabled { "available" } else { "disabled" };
        if let Some(remote) = crate::cli::login::load_remote_config() {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default();
            let _ = client
                .patch(format!("{}/api/rooms", remote.relay_url))
                .bearer_auth(&remote.api_token)
                .json(&serde_json::json!({ "status": status }))
                .send()
                .await;
        }
    }

    // If room was just disabled, kick all consumer peers and disconnect proxy rooms
    if was_enabled && !config.room_enabled {
        let _ = state.events_tx.send(crate::server::ServerEvent::RoomDisabled);
        crate::server::rtc::proxy_room::disconnect_all_proxy_rooms(state).await;
    }

    Json(serde_json::json!({ "ok": true })).into_response()
}

/// POST /api/proxy/connect — connect to a proxy room as a linggen consumer.
/// Body: { "instance_id": "...", "owner_name": "Tom" }
pub(crate) async fn connect_proxy_room_api(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let instance_id = match body.get("instance_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "instance_id required" }))).into_response(),
    };
    let owner_name = body.get("owner_name").and_then(|v| v.as_str()).map(|s| s.to_string());
    let room_name = body.get("room_name").and_then(|v| v.as_str()).map(|s| s.to_string());

    match crate::server::rtc::proxy_room::connect_proxy_room(state, &instance_id, owner_name, room_name).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": format!("{e}") }))).into_response(),
    }
}

/// POST /api/proxy/disconnect — disconnect from proxy room(s).
/// Body: { "instance_id": "..." } for per-room, or empty/omitted for all.
pub(crate) async fn disconnect_proxy_room_api(
    State(state): State<Arc<ServerState>>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let instance_id = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("instance_id").and_then(|id| id.as_str()).map(String::from));

    match instance_id {
        Some(id) => crate::server::rtc::proxy_room::disconnect_proxy_room_by_instance(state, &id).await,
        None => crate::server::rtc::proxy_room::disconnect_all_proxy_rooms(state).await,
    }
    Json(serde_json::json!({ "ok": true }))
}

/// GET /api/proxy/status — list active proxy room connections.
pub(crate) async fn proxy_status_api(
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    let connections = state.proxy_connections.list().await;
    Json(serde_json::json!({ "connections": connections }))
}

/// GET /api/token-usage — get current token usage from persistent store.
pub(crate) async fn token_usage_api(
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    let store = state.token_usage.lock().await;
    let room_cfg = crate::server::rtc::room_config::load_room_config();
    Json(serde_json::json!({
        "room_total": store.get_usage("").1,
        "room_budget": room_cfg.token_budget_room_daily,
        "consumer_budget": room_cfg.token_budget_consumer_daily,
    }))
}

/// Proxy linggen.dev room APIs — forwards GET/POST/PATCH/DELETE to /api/rooms/*.
/// Uses the API token from remote.toml for auth.
pub(crate) async fn proxy_rooms(
    method: axum::http::Method,
    axum::extract::Path(path): axum::extract::Path<String>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let config = match crate::cli::login::load_remote_config() {
        Some(c) => c,
        None => return (StatusCode::UNAUTHORIZED, "Not logged in to linggen.dev").into_response(),
    };

    let url = format!("{}/api/rooms/{}", config.relay_url, path);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default();

    let mut req = match method {
        axum::http::Method::GET => client.get(&url),
        axum::http::Method::POST => client.post(&url),
        axum::http::Method::PATCH => client.patch(&url),
        axum::http::Method::DELETE => client.delete(&url),
        _ => return (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
    };

    req = req.bearer_auth(&config.api_token);
    if !body.is_empty() {
        // Auto-inject instance_id for room creation/update if not already present
        if let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(&body) {
            if json.get("instance_id").is_none() || json["instance_id"].is_null() {
                json["instance_id"] = serde_json::Value::String(config.instance_id.clone());
            }
            req = req.header("Content-Type", "application/json").json(&json);
        } else {
            req = req.header("Content-Type", "application/json").body(body.to_vec());
        }
    }

    match req.send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            match resp.json::<serde_json::Value>().await {
                Ok(body) => (status, Json(body)).into_response(),
                Err(_) => (status, "{}").into_response(),
            }
        }
        Err(e) => (StatusCode::BAD_GATEWAY, format!("Relay error: {}", e)).into_response(),
    }
}
