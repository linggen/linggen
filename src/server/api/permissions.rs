//! Session permission endpoints — see `doc/permission-spec.md`.

use crate::server::ServerState;
use axum::{
    extract::{Json, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

/// GET /api/sessions/permission?session_id=...&cwd=...
/// Returns the session's permission.json contents plus `effective_mode` for the given cwd.
pub(crate) async fn get_session_permission(
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let session_id = match params.get("session_id") {
        Some(id) => id,
        None => return (StatusCode::BAD_REQUEST, "Missing session_id".to_string()).into_response(),
    };
    let session_dir = crate::paths::global_sessions_dir().join(session_id);
    let perms = crate::engine::permission::SessionPermissions::load(&session_dir);

    let effective_mode = params.get("cwd").and_then(|cwd| {
        crate::engine::permission::effective_mode_for_path(
            &perms.path_modes,
            std::path::Path::new(cwd),
        )
    });

    let mut resp = match serde_json::to_value(&perms) {
        Ok(v) => v,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    if let Some(mode) = effective_mode {
        if let Some(m) = resp.as_object_mut() {
            m.insert(
                "effective_mode".to_string(),
                serde_json::Value::String(mode.to_string()),
            );
        }
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
    State(state): State<Arc<ServerState>>,
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

    let session_dir = crate::paths::global_sessions_dir().join(&req.session_id);
    let mut perms = SessionPermissions::load(&session_dir);
    perms.set_path_mode(&req.path, mode);
    perms.save(&session_dir);

    let _ = state
        .events_tx
        .send(crate::server::ServerEvent::StateUpdated);
    StatusCode::OK
}
