use crate::project_store::missions::{self, MISSION_AGENT_ID};
use crate::server::{ServerEvent, ServerState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

/// GET /api/missions — list all global missions.
pub(crate) async fn list_missions(
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    match state.manager.missions.list_all_missions() {
        Ok(missions) => {
            Json(serde_json::json!({ "missions": missions })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list missions: {}", e),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct CreateMissionRequest {
    #[serde(default)]
    name: Option<String>,
    schedule: String,
    prompt: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    project: Option<String>,
    /// Permission tier: "readonly", "standard", "full".
    #[serde(default)]
    permission_tier: Option<String>,
}

/// POST /api/missions
pub(crate) async fn create_mission(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CreateMissionRequest>,
) -> impl IntoResponse {
    // Missions require the "mission" skill to be installed.
    if state.skill_manager.get_skill("mission").await.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            "The \"mission\" skill is not installed. Run `ling init` to install default skills.".to_string(),
        ).into_response();
    }

    if let Err(e) = missions::validate_cron(&req.schedule) {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }

    let project = req.project;

    match state.manager.missions.create_mission(
        req.name,
        &req.schedule,
        &req.prompt,
        req.model,
        project,
        req.permission_tier,
    ) {
        Ok(mission) => {
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            Json(mission).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create mission: {}", e),
        )
            .into_response(),
    }
}

/// GET /api/missions/:id
pub(crate) async fn get_mission(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.manager.missions.get_mission(&id) {
        Ok(Some(mission)) => Json(mission).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get mission: {}", e),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct UpdateMissionRequest {
    #[serde(default)]
    name: Option<Option<String>>,
    #[serde(default)]
    schedule: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    model: Option<Option<String>>,
    #[serde(default)]
    project: Option<Option<String>>,
    #[serde(default)]
    enabled: Option<bool>,
    /// Permission tier: "readonly", "standard", "full".
    #[serde(default)]
    permission_tier: Option<String>,
}

/// PUT /api/missions/:id
pub(crate) async fn update_mission(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateMissionRequest>,
) -> impl IntoResponse {
    if let Some(ref s) = req.schedule {
        if let Err(e) = missions::validate_cron(s) {
            return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
        }
    }

    match state.manager.missions.update_mission(
        &id,
        req.name,
        req.schedule.as_deref(),
        req.prompt.as_deref(),
        req.model,
        req.project,
        req.enabled,
        req.permission_tier,
    ) {
        Ok(mission) => {
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            Json(mission).into_response()
        }
        Err(e) => {
            let status = if e.to_string().contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, format!("Failed to update mission: {}", e)).into_response()
        }
    }
}

/// DELETE /api/missions/:id
pub(crate) async fn delete_mission(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.manager.missions.delete_mission(&id) {
        Ok(()) => {
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete mission: {}", e),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct TriggerMissionRequest {
    #[serde(default)]
    project_root: Option<String>,
}

/// POST /api/missions/:id/trigger — run a mission immediately
pub(crate) async fn trigger_mission(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<TriggerMissionRequest>,
) -> impl IntoResponse {
    if state.skill_manager.get_skill("mission").await.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            "The \"mission\" skill is not installed. Run `ling init` to install default skills.".to_string(),
        ).into_response();
    }

    let mission = match state.manager.missions.get_mission(&id) {
        Ok(Some(m)) => m,
        Ok(None) => return (StatusCode::NOT_FOUND, "Mission not found".to_string()).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    // Determine project root: from request, mission, or current dir
    let project_path = req
        .project_root
        .or_else(|| mission.project.clone())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().to_string_lossy().to_string());

    let root = std::path::PathBuf::from(&project_path);

    let _ = state.events_tx.send(ServerEvent::MissionTriggered {
        mission_id: mission.id.clone(),
        agent_id: MISSION_AGENT_ID.to_string(),
        project_root: project_path.clone(),
    });

    state
        .manager
        .update_agent_activity(&project_path, MISSION_AGENT_ID)
        .await;

    // Pre-create the session so we can return its ID immediately
    let session_id = crate::server::mission_scheduler::create_mission_session(&mission);

    // Persist the initial user message so the UI has content when it loads the session
    if let Some(ref sid) = session_id {
        let message = format!("[Mission: {}]\n\n{}", mission.id, mission.prompt);
        let _ = state.manager.global_sessions.add_chat_message(
            sid,
            &crate::state_fs::sessions::ChatMsg {
                agent_id: MISSION_AGENT_ID.to_string(),
                from_id: "user".to_string(),
                to_id: MISSION_AGENT_ID.to_string(),
                content: message,
                timestamp: crate::util::now_ts_secs(),
                is_observation: false,
            },
        );
    }

    let state_clone = state.clone();
    let session_id_clone = session_id.clone();

    tokio::spawn(async move {
        crate::server::mission_scheduler::dispatch_mission_prompt_public(
            state_clone,
            root,
            &project_path,
            &mission,
            session_id_clone,
        )
        .await;
    });

    Json(serde_json::json!({ "ok": true, "message": "Mission triggered", "session_id": session_id })).into_response()
}

/// GET /api/missions/sessions/state?mission_id=xxx&session_id=xxx — read mission session messages
pub(crate) async fn get_mission_session_state(
    State(state): State<Arc<ServerState>>,
    Query(q): Query<MissionSessionQuery>,
) -> impl IntoResponse {
    let Some(session_id) = q.session_id.filter(|s| !s.is_empty()) else {
        return Json(serde_json::json!({ "messages": [] })).into_response();
    };
    let Some(_mission_id) = q.mission_id.filter(|s| !s.is_empty()) else {
        return Json(serde_json::json!({ "messages": [] })).into_response();
    };

    let messages = state.manager.global_sessions
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
pub(crate) struct MissionSessionQuery {
    #[serde(default)]
    mission_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
}

/// GET /api/missions/:id/sessions — list sessions for a specific mission
pub(crate) async fn list_mission_sessions(
    State(state): State<Arc<ServerState>>,
    Path(mission_id): Path<String>,
) -> impl IntoResponse {
    let sessions: Vec<_> = state.manager.global_sessions.list_sessions()
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.mission_id.as_deref() == Some(&mission_id))
        .collect();
    Json(serde_json::json!({ "sessions": sessions })).into_response()
}

/// DELETE /api/missions/:mission_id/sessions/:session_id — delete a mission session and its run entry
pub(crate) async fn delete_mission_session(
    State(state): State<Arc<ServerState>>,
    Path((mission_id, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    state.manager.remove_session_engine(&session_id).await;
    if let Err(e) = state.manager.global_sessions.remove_session(&session_id) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete session: {}", e),
        )
            .into_response();
    }
    // Also remove the corresponding run entry from runs.jsonl
    let _ = state
        .manager
        .missions
        .remove_run_by_session(&mission_id, &session_id);
    Json(serde_json::json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
pub(crate) struct PaginationQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

/// GET /api/missions/:id/runs?limit=N&offset=N
pub(crate) async fn list_mission_runs(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Query(page): Query<PaginationQuery>,
) -> impl IntoResponse {
    match state
        .manager
        .missions
        .list_mission_runs_paginated(&id, page.limit, page.offset)
    {
        Ok(runs) => Json(serde_json::json!({ "runs": runs })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list mission runs: {}", e),
        )
            .into_response(),
    }
}
