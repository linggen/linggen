//! A skill page's declared door to its own scripts:
//!
//!   POST /api/skills/{skill}/tools/{tool}[?session=<id>]
//!   body: the tool's args as JSON → { exit_code, stdout, stderr }
//!
//! `session` runs the tool as that session of the skill would: with the
//! values it is bound to (`{{session.<name>}}`). A session of another skill
//! is refused.
//!
//! Instead of a page writing raw shell to `/api/bash`, it names a tool; the
//! engine knows what may run. The run itself — lock, grant, cloud gate, push —
//! is `engine::skill::tools`; this is its HTTP shape.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::Value;
use std::sync::Arc;

use crate::engine::skill::tools::{self, Refusal};
use crate::server::ServerState;

fn refuse(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, msg.into()).into_response()
}

fn status_of(refusal: &Refusal) -> StatusCode {
    match refusal {
        Refusal::NoSkill(_) | Refusal::NoTool(_) => StatusCode::NOT_FOUND,
        Refusal::NotRunnable(_) | Refusal::BadCall(_) => StatusCode::BAD_REQUEST,
        Refusal::SignedOut => StatusCode::UNAUTHORIZED,
        Refusal::BeyondGrant(_) | Refusal::NotForPet(_) => StatusCode::FORBIDDEN,
        Refusal::Crashed(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[derive(serde::Deserialize)]
pub struct DoorQuery {
    #[serde(default)]
    session: Option<String>,
}

/// The values the named session is bound to, when it is a session of
/// `skill`; no session named, none.
fn bound_params(
    state: &ServerState,
    skill: &str,
    session: Option<&str>,
) -> Result<crate::engine::skill::params::SessionParams, String> {
    let Some(sid) = session.filter(|s| !s.is_empty()) else {
        return Ok(Default::default());
    };
    state
        .manager
        .global_sessions
        .params_for(sid, skill)
        .ok_or_else(|| format!("'{sid}' is not a session of '{skill}'"))
}

pub async fn run_tool(
    State(state): State<Arc<ServerState>>,
    Path((skill_name, tool_name)): Path<(String, String)>,
    Query(query): Query<DoorQuery>,
    body: Option<Json<Value>>,
) -> Response {
    let args = body
        .map(|Json(v)| v)
        .unwrap_or_else(|| Value::Object(Default::default()));
    if !args.is_object() {
        return refuse(
            StatusCode::BAD_REQUEST,
            "the body is the tool's args, as an object",
        );
    }
    let Some(skill) = state.skills.get_skill(&skill_name).await else {
        let refusal = Refusal::no_skill(&skill_name);
        return refuse(status_of(&refusal), refusal.to_string());
    };
    let params = match bound_params(&state, &skill_name, query.session.as_deref()) {
        Ok(p) => p,
        Err(why) => return refuse(StatusCode::BAD_REQUEST, why),
    };
    match tools::run(&skill, &tool_name, &args, &params).await {
        Ok(result) => Json(tools::output(result)).into_response(),
        Err(refusal) => refuse(status_of(&refusal), refusal.to_string()),
    }
}
