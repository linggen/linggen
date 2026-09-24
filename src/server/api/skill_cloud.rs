//! A skill page's view of its cloud (`CloudConfig`):
//!
//!   GET  /api/skill-cloud/{skill}       signed in?, the meter's reading
//!   POST /api/skill-cloud/{skill}/sync  keep the save in step now — the
//!                                       page calls it on open and after a
//!                                       change it made itself (a board won)

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;

use crate::account::cloud::{last_reading, meter_reading};
use crate::account::cloud_save::{sync, target, Synced};
use crate::engine::skill::Skill;
use crate::server::{ServerEvent, ServerState};

/// What a skill turn — or a page's tool call — answers when the skill keeps
/// a cloud and nobody is signed in.
pub(crate) use crate::engine::skill::tools::AUTH_REQUIRED;

/// Tell every surface a pull replaced a skill's save (`SkillSaveChanged`):
/// its page reads again instead of showing the files it had.
pub(crate) fn announce(
    events_tx: &tokio::sync::broadcast::Sender<ServerEvent>,
    skill: &str,
    done: &Synced,
) {
    if !done.pulled() {
        return;
    }
    let _ = events_tx.send(ServerEvent::SkillSaveChanged {
        skill: skill.to_string(),
        version: done.version,
        conflicts: done
            .conflicts
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
    });
}

async fn cloud_skill(state: &Arc<ServerState>, name: &str) -> Result<Skill, Response> {
    let not_found = || (StatusCode::NOT_FOUND, "no cloud for that skill").into_response();
    let skill = state.skills.get_skill(name).await.ok_or_else(not_found)?;
    if skill.cloud.is_none() {
        return Err(not_found());
    }
    Ok(skill)
}

pub async fn get_cloud(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
) -> Response {
    let skill = match cloud_skill(&state, &name).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let signed_in = crate::account::resolve_token().is_some();
    let meter_name = skill.cloud.as_ref().and_then(|c| c.meter.clone());
    let meter = match (&meter_name, signed_in) {
        (Some(m), true) => meter_reading(m).await.ok().or_else(|| last_reading(m)),
        _ => None,
    };
    Json(serde_json::json!({ "signed_in": signed_in, "meter": meter })).into_response()
}

pub async fn post_sync(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
) -> Response {
    let skill = match cloud_skill(&state, &name).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    if crate::account::resolve_token().is_none() {
        return (StatusCode::UNAUTHORIZED, "signed out").into_response();
    }
    let Some(save) = target(&skill) else {
        return (StatusCode::NOT_FOUND, "that skill keeps no save").into_response();
    };
    match sync(&save).await {
        Ok(done) => {
            announce(&state.events_tx, &save.skill, &done);
            Json(serde_json::json!({
                "done": done.action,
                "version": done.version,
                "conflicts": done.conflicts,
            }))
            .into_response()
        }
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}")).into_response(),
    }
}
