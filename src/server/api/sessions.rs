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
    /// Required for user/project sessions, optional for skill sessions.
    #[serde(default)]
    project_root: Option<String>,
    title: String,
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
    let skill = match req.skill.as_deref() {
        Some(name) => state.manager.skills.get_skill(name).await,
        None => None,
    };
    if let Some(why) = refuse_params(&req.params, skill.as_ref()) {
        return (StatusCode::BAD_REQUEST, why).into_response();
    }
    let cwd_str: Option<String> = skill.as_ref().and_then(|s| s.cwd.clone());
    let cwd_str = cwd_str.or_else(|| req.project_root.clone());
    let cwd = cwd_str
        .as_deref()
        .map(|p| canonical_project_root(p).to_string_lossy().to_string());
    let is_skill = req.skill.is_some();
    let meta = crate::state_fs::sessions::SessionMeta {
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
        cwd,
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
        Ok(_) => Json(serde_json::json!({ "id": id })).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
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
            Some(serde_json::json!([
                run_calls::with_tools(meta, calls),
                cleaned
            ]))
        },
        |run| serde_json::json!([run.meta(), ""]),
    );

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
