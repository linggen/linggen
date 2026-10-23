//! Session CRUD: list/create/resolve/remove/rename, plus skill-session
//! variants and the unified session list/delete endpoints.

use crate::server::chat::run_calls;
use crate::server::ServerState;
use axum::{
    extract::{Json, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use std::sync::Arc;

use super::canonical_project_root;

#[derive(Deserialize)]
pub(crate) struct CreateSessionRequest {
    /// The folder the person chose for this session (a picked project);
    /// absent for a plain New chat, which starts in the default workspace.
    #[serde(default)]
    project_root: Option<String>,
    title: String,
    #[serde(default)]
    runtime: crate::state_fs::sessions::SessionRuntime,
    #[serde(default)]
    user_type: String,
    #[serde(default)]
    skill: Option<String>,
    /// User ID of the session creator (injected by peer.rs).
    #[serde(default)]
    user_id: Option<String>,
    /// Tools no member may use here (`SessionMeta::withheld_tools`).
    #[serde(default)]
    withheld_tools: Vec<String>,
    /// The values the session is bound to, by the skill's declared session
    /// params (`SessionMeta::params`). Only with `skill`; checked here.
    #[serde(default)]
    params: crate::engine::skill::params::SessionParams,
}

/// Why a session can't be bound to `params`: no skill to declare them, or
/// one that doesn't declare (or accept) them.
fn refuse_params(
    params: &crate::engine::skill::params::SessionParams,
    skill: Option<&crate::engine::skill::Skill>,
) -> Option<String> {
    if params.is_empty() {
        return None;
    }
    let Some(skill) = skill else {
        return Some("session params need a skill that declares them".into());
    };
    crate::engine::skill::params::check(&skill.session_params, params).err()
}

/// Where a new session works, fixed when it is made: the bound skill's
/// declared `cwd:`, else the folder the request chose for it, else where a
/// session like it starts — a skill's own folder, or the default workspace
/// (`paths::default_workspace`). Nothing a surface sends later moves it; only
/// a `cd` does.
fn session_cwd(skill: Option<&crate::engine::skill::Skill>, project_root: Option<&str>) -> String {
    let given = |p: &&str| !p.trim().is_empty();
    let declared = skill.and_then(|s| s.cwd.as_deref()).filter(given);
    let dir = match declared.or(project_root.filter(given)) {
        Some(p) => canonical_project_root(p),
        None => match skill {
            Some(s) => s.working_dir(),
            None => crate::util::resolve_path(&crate::paths::ensure_default_workspace()),
        },
    };
    dir.to_string_lossy().to_string()
}

pub(crate) async fn create_session(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CreateSessionRequest>,
) -> impl IntoResponse {
    if !req.runtime.is_linggen() {
        if req.user_type == "consumer" {
            return StatusCode::FORBIDDEN.into_response();
        }
        if req.skill.is_some() || !req.params.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                "Claude sessions cannot bind a Linggen skill.",
            )
                .into_response();
        }
        let mut meta = crate::server::claude::new_meta(
            req.title,
            session_cwd(None, req.project_root.as_deref()),
            None,
        );
        meta.user_id = req.user_id;
        meta.withheld_tools = req.withheld_tools;
        if let Err(error) = state.manager.global_sessions.add_session(&meta) {
            tracing::error!("Could not create Claude session: {error}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        let _ = state
            .events_tx
            .send(crate::server::ServerEvent::StateUpdated);
        return Json(serde_json::json!({"id": meta.id, "cwd": meta.cwd, "runtime": meta.runtime, "agents": meta.agents})).into_response();
    }
    let id = format!(
        "sess-{}-{}",
        crate::util::now_ts_secs(),
        &uuid::Uuid::new_v4().to_string()[..8]
    );
    let skill = match req.skill.as_deref() {
        Some(name) => state.manager.skills.get_skill(name).await,
        None => None,
    };
    if let Some(why) = refuse_params(&req.params, skill.as_ref()) {
        return (StatusCode::BAD_REQUEST, why).into_response();
    }
    let cwd = session_cwd(skill.as_ref(), req.project_root.as_deref());
    let is_skill = req.skill.is_some();
    let meta = crate::state_fs::sessions::SessionMeta {
        runtime: Default::default(),
        claude_permission_mode: Default::default(),
        parent_session_id: None,
        id: id.clone(),
        title: req.title,
        created_at: crate::util::now_ts_secs(),
        updated_at: 0,
        skill: req.skill.clone(),
        creator: if is_skill {
            "skill".into()
        } else {
            "user".into()
        },
        cwd: cwd.clone(),
        project: None,
        project_name: None,
        mission_id: None,
        agents: crate::server::chat::members::default_members(skill.as_ref()),
        legacy: Default::default(),
        user_id: req.user_id,
        compact_threshold: None,
        compact_focus: None,
        // Skill sessions carry the skill name as canonical title; lock it.
        // User-created chats arrive with a time-based placeholder ("Chat May
        // 22, 3:20 PM") that should be overwritten by the auto-rename hook
        // once the user's first message lands.
        title_locked: is_skill,
        withheld_tools: req.withheld_tools,
        params: req.params,
    };

    match state.manager.global_sessions.add_session(&meta) {
        Ok(_) => Json(serde_json::json!({ "id": id, "cwd": cwd })).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct SessionImageQuery {
    session_id: String,
    name: String,
}

/// GET /api/sessions/images?session_id=&name= — a picture a person's
/// message carried, as kept beside the transcript (`ChatMsg::images`).
pub(crate) async fn get_session_image(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<SessionImageQuery>,
) -> impl IntoResponse {
    use crate::state_fs::session_images::media_type;
    let store = &state.manager.global_sessions;
    let Some(path) = store.image_path(&query.session_id, &query.name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            [
                (axum::http::header::CONTENT_TYPE, media_type(&query.name)),
                (
                    axum::http::header::CACHE_CONTROL,
                    "private, max-age=31536000, immutable",
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
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
                        "updated_at": s.updated_at,
                        "skill": s.skill,
                        "creator": s.creator,
                        // A skill embed has no other way to learn its session's
                        // members and their models: page_state omits
                        // all_sessions for embed views, and no GET returns a
                        // single session's meta. The embed reads this to echo
                        // the model on every send — without it the engine
                        // falls to the global default.
                        "agents": s.agents,
                        // The values the session is bound to (`params`): a
                        // page resumes only a session bound as it is.
                        "params": s.params,
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

    let running = run_calls::session_running(&state.manager, &session_id).await;
    let mapped = run_calls::with_run_calls(
        messages,
        running,
        |m, calls| {
            if m.is_observation || crate::server::resident::is_engine_prompt(&m.from_id, &m.content)
            {
                return None;
            }
            // Silence keeps its calls, never its word.
            let silent = crate::server::resident::is_silent_reply(&m.from_id, &m.content);
            if silent && calls.is_empty() {
                return None;
            }
            let cleaned = match silent {
                true => String::new(),
                false => {
                    crate::engine::tool_render::sanitize_message_for_ui(&m.from_id, &m.content)?
                }
            };
            let meta = serde_json::json!({
                "id": format!("msg-{}", m.timestamp),
                "from": m.from_id,
                "to": m.to_id,
                "ts": m.timestamp,
                "task_id": null,
                "client_id": m.client_id,
                "via": m.via,
                "via_note": m.via_note
            });
            let meta = crate::state_fs::session_images::with_urls(meta, &session_id, &m.images);
            Some(serde_json::json!([
                run_calls::with_tools(meta, m.run_id.as_deref(), calls),
                cleaned
            ]))
        },
        |run| serde_json::json!([run.meta(), ""]),
    );

    let refs = crate::engine::answer_refs::view::refs_for(
        &state.manager.global_sessions,
        &session_id,
        mapped.iter().filter_map(|row| row[1].as_str()),
    );
    Json(serde_json::json!({
        "active_task": null,
        "user_stories": null,
        "tasks": [],
        "messages": mapped,
        "refs": refs
    }))
    .into_response()
}

#[derive(Deserialize)]
pub(crate) struct RemoveSkillSessionRequest {
    session_id: String,
}

pub(crate) async fn remove_skill_session_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<RemoveSkillSessionRequest>,
) -> impl IntoResponse {
    state.manager.remove_session_engine(&req.session_id).await;
    match state
        .manager
        .global_sessions
        .remove_session(&req.session_id)
    {
        Ok(_) => {
            let _ = state
                .events_tx
                .send(crate::server::ServerEvent::StateUpdated);
            StatusCode::OK
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[derive(Deserialize)]
pub(crate) struct RenameSessionRequest {
    session_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    model_id: Option<String>,
    /// The member whose model `model_id` sets; absent: the member the Mac
    /// answers with by default (the one the model picker shows).
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    claude_permission_mode: Option<crate::state_fs::sessions::ClaudePermissionMode>,
}

pub(crate) async fn rename_session_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<RenameSessionRequest>,
) -> impl IntoResponse {
    if let Ok(Some(mut meta)) = state
        .manager
        .global_sessions
        .get_session_meta(&req.session_id)
    {
        if !meta.runtime.is_linggen() {
            if req
                .agent_id
                .as_deref()
                .is_some_and(|id| id != crate::server::claude::AGENT_ID)
            {
                return StatusCode::BAD_REQUEST;
            }
            if let Some(model) = &req.model_id {
                if crate::server::claude::resolve_model(&state, Some(model))
                    .await
                    .is_err()
                {
                    return StatusCode::BAD_REQUEST;
                }
                meta.set_member_model(
                    crate::server::claude::AGENT_ID,
                    (!model.is_empty()).then(|| model.clone()),
                );
            }
            if let Some(mode) = req.claude_permission_mode {
                meta.claude_permission_mode = mode;
            }
            if let Some(title) = req.title {
                meta.title = title;
                meta.title_locked = true;
            }
            if state
                .manager
                .global_sessions
                .update_session_meta(&meta)
                .is_err()
            {
                return StatusCode::INTERNAL_SERVER_ERROR;
            }
            let _ = state
                .events_tx
                .send(crate::server::ServerEvent::StateUpdated);
            return StatusCode::OK;
        }
        if req.claude_permission_mode.is_some() {
            return StatusCode::BAD_REQUEST;
        }
    }
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
            let members =
                crate::server::chat::members::of_session(&state.manager, &req.session_id).await;
            let member = req
                .agent_id
                .clone()
                .filter(|a| !a.trim().is_empty())
                .unwrap_or_else(|| crate::server::chat::members::default_responder(&members));
            meta.agents = members;
            if meta.set_member_model(&member, new_val.clone()) {
                let _ = state.manager.global_sessions.update_session_meta(&meta);
            }
            // On a mission session the model picker is the only model control
            // the user is shown, so it has to be the mission's model — not a
            // per-session override the next scheduled run overwrites (the
            // scheduler stamps `mission.model` onto every session it opens).
            // Anything else is a knob that moves and changes nothing.
            if let Some(mission_id) = meta.mission_id.clone() {
                let draft = crate::extensions::missions::MissionDraft {
                    model: Some(new_val),
                    ..Default::default()
                };
                if let Err(e) = state.missions.update_mission(&mission_id, draft) {
                    tracing::warn!("mission '{mission_id}' model not updated: {e}");
                }
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
}

/// DELETE /api/sessions/all — delete a session from the global store.
pub(crate) async fn delete_unified_session(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<DeleteUnifiedSessionRequest>,
) -> impl IntoResponse {
    let claude = state
        .manager
        .global_sessions
        .get_session_meta(&req.session_id)
        .ok()
        .flatten()
        .is_some_and(|m| !m.runtime.is_linggen());
    let _sdk_turn = if claude {
        Some(crate::server::claude::quiesce(&state, &req.session_id).await)
    } else {
        None
    };
    state.manager.remove_session_engine(&req.session_id).await;
    match state
        .manager
        .global_sessions
        .remove_session(&req.session_id)
    {
        Ok(_) => {
            let _ = state
                .events_tx
                .send(crate::server::ServerEvent::StateUpdated);
            StatusCode::OK.into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::refuse_params;
    use crate::engine::skill::params::SessionParams;

    fn skill(extra: &str) -> crate::engine::skill::Skill {
        let text = format!("---\nname: game\ndescription: d\n{extra}---\nBody.");
        crate::extensions::skills::parse_skill_text(
            &text,
            crate::engine::skill::SkillSource::Global,
        )
        .unwrap()
    }

    /// A session is bound only to what its skill declares and accepts;
    /// binding nothing is always fine.
    #[test]
    fn a_session_binds_only_declared_accepted_values() {
        let game = skill("session-params:\n  save:\n    pattern: \"[a-z0-9-]{1,32}\"\n");
        let save = |v: &str| SessionParams::from([("save".to_string(), v.to_string())]);
        assert!(refuse_params(&SessionParams::new(), None).is_none());
        assert!(refuse_params(&save("lx-gu3"), Some(&game)).is_none());
        assert!(refuse_params(&save("lx gu3"), Some(&game)).is_some());
        assert!(
            refuse_params(&save("lx"), None).is_some(),
            "no skill, no params"
        );
        assert!(
            refuse_params(&save("lx"), Some(&skill(""))).is_some(),
            "undeclared"
        );
    }
}
