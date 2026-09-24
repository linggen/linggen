//! A skill page's declared door to its own scripts:
//!
//!   POST /api/skills/{skill}/tools/{tool}   body: the tool's args as JSON
//!   → { exit_code, stdout, stderr }
//!
//! Instead of a page writing raw shell to `/api/bash`, it names a tool; the
//! engine knows what may run. The run itself — lock, grant, cloud gate, push —
//! is `engine::skill::tools`; this is its HTTP shape.

use axum::{
    extract::{Path, State},
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

pub async fn run_tool(
    State(state): State<Arc<ServerState>>,
    Path((skill_name, tool_name)): Path<(String, String)>,
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
    match tools::run(&skill, &tool_name, &args).await {
        Ok(result) => Json(tools::output(result)).into_response(),
        Err(refusal) => refuse(status_of(&refusal), refusal.to_string()),
    }
}
