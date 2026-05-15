//! Session CRUD: list/create/resolve/remove/rename, plus skill-session
//! variants and the unified session list/delete endpoints.

use crate::server::ServerState;
use axum::{
    extract::{Json, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use std::sync::Arc;

use super::{canonical_project_root, ProjectQuery};

pub(crate) async fn list_sessions(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ProjectQuery>,
) -> impl IntoResponse {
    match state.manager.global_sessions.list_sessions() {
        Ok(all_sessions) => {
            // Filter by project_root: match sessions whose cwd or project starts with the query path.
            let canonical = canonical_project_root(&query.project_root);
            let canonical_str = canonical.to_string_lossy();
            let filtered: Vec<_> = all_sessions
                .into_iter()
                .filter(|s| {
                    s.cwd
                        .as_deref()
                        .map(|c| c.starts_with(canonical_str.as_ref()))
                        .unwrap_or(false)
                        || s.project
                            .as_deref()
                            .map(|p| p.starts_with(canonical_str.as_ref()))
                            .unwrap_or(false)
                })
                .collect();
            let total = filtered.len();
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
    // cwd resolution: skill's declared `cwd:` (if any) wins, otherwise the
    // iframe's project_root, otherwise the configured home_path. Mirrors how
    // mission frontmatter sets cwd; lets ling-mem boot at ~/.linggen instead
    // of the user's home dir.
    let mut cwd_str: Option<String> = None;
    if let Some(ref skill_name) = req.skill {
        if let Some(skill) = state.manager.skill_manager.get_skill(skill_name).await {
            if let Some(ref c) = skill.cwd {
                cwd_str = Some(c.clone());
            }
        }
    }
    let cwd_str = cwd_str.or_else(|| req.project_root.clone());
    let cwd = cwd_str
        .as_deref()
        .map(|p| canonical_project_root(p).to_string_lossy().to_string());
    let meta = crate::state_fs::sessions::SessionMeta {
        id: id.clone(),
        title: req.title,
        created_at: crate::util::now_ts_secs(),
        skill: req.skill.clone(),
        creator: if req.skill.is_some() {
            "skill".into()
        } else {
            "user".into()
        },
        cwd,
        project: None,
        project_name: None,
        mission_id: None,
        model_id: None,
        user_id: req.user_id,
        compact_threshold: None,
        compact_focus: None,
    };

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
    let now = crate::util::now_ts_secs();
    let new_id = format!("sess-{}-{}", now, &uuid::Uuid::new_v4().to_string()[..8]);
    let meta = crate::state_fs::sessions::SessionMeta {
        id: new_id.clone(),
        title: "New Chat".to_string(),
        created_at: now,
        skill: None,
        creator: "user".into(),
        cwd: Some(req.project_root.clone()),
        project: None,
        project_name: None,
        mission_id: None,
        model_id: None,
        user_id: None,
        compact_threshold: None,
        compact_focus: None,
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
        Ok(_) => {
            let _ = state.events_tx.send(crate::server::ServerEvent::StateUpdated);
            StatusCode::OK
        }
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

#[derive(Deserialize)]
pub(crate) struct SkillSessionStateQuery {
    #[serde(default)]
    skill: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
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

    let messages = state
        .manager
        .global_sessions
        .get_chat_history(&session_id)
        .unwrap_or_default();

    let mapped: Vec<serde_json::Value> = messages
        .into_iter()
        .filter(|m| !m.is_observation)
        .filter(|m| !m.content.contains("[HIDDEN]"))
        .filter_map(|m| {
            let cleaned =
                crate::engine::tool_render::sanitize_message_for_ui(&m.from_id, &m.content)?;
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
        Ok(_) => {
            let _ = state.events_tx.send(crate::server::ServerEvent::StateUpdated);
            StatusCode::OK
        }
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
    if let Some(ref title) = req.title {
        if state
            .manager
            .global_sessions
            .rename_session(&req.session_id, title)
            .is_err()
        {
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    }
    if let Some(ref model_id) = req.model_id {
        if let Ok(Some(mut meta)) = state
            .manager
            .global_sessions
            .get_session_meta(&req.session_id)
        {
            let new_val = if model_id.is_empty() {
                None
            } else {
                Some(model_id.clone())
            };
            if meta.model_id != new_val {
                meta.model_id = new_val;
                let _ = state.manager.global_sessions.update_session_meta(&meta);
            }
        }
    }
    let _ = state
        .events_tx
        .send(crate::server::ServerEvent::StateUpdated);
    StatusCode::OK
}

// ---------------------------------------------------------------------------
// Unified session delete + list — across all sources
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
        Ok(_) => {
            let _ = state.events_tx.send(crate::server::ServerEvent::StateUpdated);
            StatusCode::OK.into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

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
