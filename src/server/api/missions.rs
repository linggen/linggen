use crate::extensions::missions::{self, MissionDraft, MissionPermission};
use crate::server::{ServerEvent, ServerState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

/// Accepted values for permission.mode.
const VALID_MODES: &[&str] = &["read", "edit", "admin"];

fn validate_mode(mode: &str) -> Result<(), String> {
    if VALID_MODES.contains(&mode) {
        Ok(())
    } else {
        Err(format!(
            "Invalid permission.mode '{}'. Allowed: {}",
            mode,
            VALID_MODES.join(", ")
        ))
    }
}

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
    #[serde(default)]
    description: Option<String>,
    schedule: String,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    model: Option<String>,
    /// Working directory. Accepts legacy `project` alias during Phase 1.
    #[serde(default, alias = "project")]
    cwd: Option<String>,
    /// Default mode applied to every entry in `permission_paths` if those
    /// entries are bare strings. Web UI sends a single mode + a list of
    /// paths; the handler converts that into per-path grants. Allowed
    /// values: "read", "edit"/"write", "admin".
    #[serde(default)]
    permission_mode: Option<String>,
    #[serde(default)]
    permission_paths: Option<Vec<String>>,
    #[serde(default)]
    permission_warning: Option<String>,
    /// Legacy mission mode: "agent" | "app".
    /// Phase 1 compat: "app" rejected.
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    kickoff: Option<Vec<String>>,
    #[serde(default)]
    allowed_tools: Option<Vec<String>>,
}

/// POST /api/missions
pub(crate) async fn create_mission(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CreateMissionRequest>,
) -> impl IntoResponse {
    if let Err(e) = missions::validate_cron(&req.schedule) {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }
    if let Some(ref m) = req.permission_mode {
        if let Err(e) = validate_mode(m) {
            return (StatusCode::BAD_REQUEST, e).into_response();
        }
    }

    // Legacy mode handling — kept to absorb old UI payloads.
    let mode = req.mode.as_deref().unwrap_or("agent");
    if mode == "app" {
        return (
            StatusCode::BAD_REQUEST,
            "mode: app is no longer supported".to_string(),
        )
            .into_response();
    }

    let prompt = req.prompt.unwrap_or_default();
    let prompt_is_empty = prompt.trim().is_empty();

    if mode == "agent" && prompt_is_empty {
        return (
            StatusCode::BAD_REQUEST,
            "Mission requires a prompt body".to_string(),
        )
            .into_response();
    }

    // Resolve permission block: convert (mode, paths) from the API into
    // per-path PathGrants. If the request omits permission entirely, the
    // mission gets an empty permission block (no grants — caller must list
    // paths explicitly).
    let permission = build_permission(
        req.permission_mode.as_deref(),
        req.permission_paths.clone(),
        req.permission_warning.clone(),
    );

    let draft = MissionDraft {
        name: req.name,
        description: req.description,
        schedule: Some(req.schedule),
        prompt: if prompt_is_empty { None } else { Some(prompt) },
        enabled: Some(true),
        cwd: Some(req.cwd.clone()),
        model: Some(req.model),
        kickoff: req.kickoff,
        allowed_tools: req.allowed_tools,
        permission: Some(permission),
        project: Some(req.cwd),
        ..Default::default()
    };

    match state.manager.missions.create_mission(draft) {
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

fn build_permission(
    mode: Option<&str>,
    paths: Option<Vec<String>>,
    warning: Option<String>,
) -> Option<MissionPermission> {
    use crate::extensions::skills::PathGrant;
    // Web UI still sends (mode, paths[]) as parallel fields. Apply the
    // single mode to every path to produce per-path grants. If both are
    // absent, return None — caller decides whether to materialize a
    // default empty block.
    let path_list = paths.unwrap_or_default();
    if mode.is_none() && path_list.is_empty() && warning.is_none() {
        return None;
    }
    let m = mode.unwrap_or("admin").to_string();
    Some(MissionPermission {
        paths: path_list
            .into_iter()
            .map(|p| PathGrant {
                path: p,
                mode: m.clone(),
            })
            .collect(),
        warning,
    })
}

#[derive(Deserialize)]
pub(crate) struct UpdateMissionRequest {
    #[serde(default)]
    name: Option<Option<String>>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    schedule: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    model: Option<Option<String>>,
    #[serde(default, alias = "project")]
    cwd: Option<Option<String>>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    permission_mode: Option<String>,
    #[serde(default)]
    permission_paths: Option<Vec<String>>,
    #[serde(default)]
    permission_warning: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    kickoff: Option<Vec<String>>,
    #[serde(default)]
    allowed_tools: Option<Vec<String>>,
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
    if let Some(ref m) = req.permission_mode {
        if let Err(e) = validate_mode(m) {
            return (StatusCode::BAD_REQUEST, e).into_response();
        }
    }
    if req.mode.as_deref() == Some("app") {
        return (
            StatusCode::BAD_REQUEST,
            "mode: app is no longer supported".to_string(),
        )
            .into_response();
    }

    // Unwrap Option<Option<String>> for name; other Option<Option<T>> fields
    // are passed through directly.
    let name_draft = req.name;

    // Resolve permission update only if caller provided any permission fields.
    let permission_update: Option<Option<MissionPermission>> =
        if req.permission_mode.is_some()
            || req.permission_paths.is_some()
            || req.permission_warning.is_some()
        {
            Some(build_permission(
                req.permission_mode.as_deref(),
                req.permission_paths,
                req.permission_warning,
            ))
        } else {
            None
        };

    // cwd alias: update cwd and the legacy `project` field together so both
    // stay in sync through the Phase 1 migration window.
    let cwd_pair = req.cwd.clone();

    let draft = MissionDraft {
        name: name_draft.and_then(|n| n),
        description: req.description,
        schedule: req.schedule,
        prompt: req.prompt,
        enabled: req.enabled,
        cwd: cwd_pair.clone(),
        model: req.model,
        kickoff: req.kickoff,
        allowed_tools: req.allowed_tools,
        permission: permission_update,
        project: cwd_pair,
        ..Default::default()
    };

    match state.manager.missions.update_mission(&id, draft) {
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
pub(crate) struct MissionFileQuery {
    id: String,
}

#[derive(Deserialize)]
pub(crate) struct MissionFileWriteRequest {
    id: String,
    content: String,
}

/// GET /api/mission-file?id=<id> — raw mission.md content.
/// Powers the Web UI's raw-markdown mission editor (CM6Editor + live preview).
pub(crate) async fn get_mission_file(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<MissionFileQuery>,
) -> impl IntoResponse {
    match state.manager.missions.read_mission_raw(&query.id) {
        Ok(Some(content)) => Json(serde_json::json!({
            "id": query.id,
            "content": content,
        }))
        .into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "Mission not found".to_string()).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read mission file: {}", e),
        )
            .into_response(),
    }
}

/// POST /api/mission-file — upsert a mission by raw mission.md content.
/// The mission id is taken from the request body and used as the directory
/// name under `~/.linggen/missions/<id>/mission.md`. Frontmatter `name`
/// is decorative — the id is the source of truth for routing.
pub(crate) async fn upsert_mission_file(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<MissionFileWriteRequest>,
) -> impl IntoResponse {
    let id = req.id.trim();
    if id.is_empty() {
        return (StatusCode::BAD_REQUEST, "id is required".to_string()).into_response();
    }
    if id.contains('/') || id.contains('\\') || id == "." || id == ".." {
        return (StatusCode::BAD_REQUEST, "invalid mission id".to_string()).into_response();
    }
    match state.manager.missions.write_mission_raw(id, &req.content) {
        Ok(mission) => {
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            Json(mission).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
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
    // Refresh from disk so a manual trigger picks up in-flight edits to
    // mission.md (kickoff, body, allowed-tools, etc.) without a daemon
    // restart. Falls back to the cached copy if the file is gone.
    let mission = match state.manager.missions.reload_one(&id) {
        Some(m) => m,
        None => match state.manager.missions.get_mission(&id) {
            Ok(Some(m)) => m,
            Ok(None) => return (StatusCode::NOT_FOUND, "Mission not found".to_string()).into_response(),
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
    };

    // Determine project root: from request, mission cwd (or legacy project), or env cwd.
    // Expand `~` and `$VAR` so `cwd: ~/.linggen` resolves to an absolute path
    // the agent's Bash tool can spawn in.
    let raw_cwd = req
        .project_root
        .or_else(|| mission.cwd.clone())
        .or_else(|| mission.project.clone())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().to_string_lossy().to_string());
    let root = crate::util::resolve_path(std::path::Path::new(&raw_cwd));
    let project_path = root.to_string_lossy().to_string();

    state
        .manager
        .update_agent_activity(&project_path, &mission.agent_id)
        .await;

    // Pre-create the session BEFORE emitting `MissionTriggered`. The UI
    // needs the session id immediately so the new session row appears in
    // the sidebar at click time, not at completion time.
    let session_id = crate::extensions::missions::scheduler::create_mission_session(&mission);

    // Persist the first kickoff message as a user turn so the UI has something
    // to show when it loads the session. The mission body lives in the system
    // prompt (via active_mission, set by the scheduler) — duplicating it here
    // would mean the agent sees the instructions twice and the transcript is
    // cluttered. The remaining kickoff items are seeded into the engine's
    // kickoff_queue by the scheduler and drain one-per-assistant-final-reply.
    if let Some(ref sid) = session_id {
        let kickoff = crate::extensions::missions::scheduler::mission_kickoff_messages(&mission);
        if let Some(first) = kickoff.first() {
            let _ = state.manager.global_sessions.add_chat_message(
                sid,
                &crate::state_fs::sessions::ChatMsg {
                    agent_id: mission.agent_id.clone(),
                    from_id: "user".to_string(),
                    to_id: mission.agent_id.clone(),
                    content: first.clone(),
                    timestamp: crate::util::now_ts_secs(),
                    is_observation: false,
                },
            );
        }
    }

    // Now emit MissionTriggered with the real session id, and a
    // `StateUpdated` to nudge the session list to refresh. The UI
    // listens for both: MissionTriggered tells the active workspace to
    // route into this session; StateUpdated triggers a sessions re-fetch
    // so the new row shows up in the sidebar immediately.
    let _ = state.events_tx.send(ServerEvent::MissionTriggered {
        mission_id: mission.id.clone(),
        agent_id: mission.agent_id.clone(),
        project_root: project_path.clone(),
        session_id: session_id.clone(),
    });
    let _ = state.events_tx.send(ServerEvent::StateUpdated);

    let state_clone = state.clone();
    let session_id_clone = session_id.clone();

    tokio::spawn(async move {
        crate::extensions::missions::scheduler::dispatch_mission_prompt_public(
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
pub(crate) struct MissionSessionQuery {
    #[serde(default)]
    mission_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
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

