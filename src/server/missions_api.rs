use crate::project_store::missions;
use crate::server::{ServerEvent, ServerState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub(crate) struct ProjectQuery {
    project_root: String,
}

/// GET /api/missions?project_root=...
pub(crate) async fn list_missions(
    State(state): State<Arc<ServerState>>,
    Query(q): Query<ProjectQuery>,
) -> impl IntoResponse {
    // Run migration on access
    let _ = state.manager.store.migrate_old_missions(&q.project_root);

    match state.manager.store.list_all_missions(&q.project_root) {
        Ok(missions) => Json(serde_json::json!({ "missions": missions })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list missions: {}", e),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct CreateMissionRequest {
    project_root: String,
    schedule: String,
    agent_id: String,
    prompt: String,
    #[serde(default)]
    model: Option<String>,
}

/// POST /api/missions
pub(crate) async fn create_mission(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CreateMissionRequest>,
) -> impl IntoResponse {
    // Validate cron first
    if let Err(e) = missions::validate_cron(&req.schedule) {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }

    match state.manager.store.create_mission(
        &req.project_root,
        &req.schedule,
        &req.agent_id,
        &req.prompt,
        req.model,
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

/// GET /api/missions/:id?project_root=...
pub(crate) async fn get_mission(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Query(q): Query<ProjectQuery>,
) -> impl IntoResponse {
    match state.manager.store.get_mission_by_id(&q.project_root, &id) {
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
    project_root: String,
    #[serde(default)]
    schedule: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    model: Option<Option<String>>,
    #[serde(default)]
    enabled: Option<bool>,
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

    match state.manager.store.update_mission(
        &req.project_root,
        &id,
        req.schedule.as_deref(),
        req.agent_id.as_deref(),
        req.prompt.as_deref(),
        req.model,
        req.enabled,
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

#[derive(Deserialize)]
pub(crate) struct DeleteMissionQuery {
    project_root: String,
}

/// DELETE /api/missions/:id?project_root=...
pub(crate) async fn delete_mission(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Query(q): Query<DeleteMissionQuery>,
) -> impl IntoResponse {
    match state.manager.store.delete_mission(&q.project_root, &id) {
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

/// GET /api/missions/:id/runs?project_root=...
pub(crate) async fn list_mission_runs(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Query(q): Query<ProjectQuery>,
) -> impl IntoResponse {
    match state.manager.store.list_mission_runs(&q.project_root, &id) {
        Ok(runs) => Json(serde_json::json!({ "runs": runs })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list mission runs: {}", e),
        )
            .into_response(),
    }
}
