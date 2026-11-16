//! `POST /api/memory/session_start` — what every host loads when a session
//! begins, rendered once, in the daemon, so every host injects the same text:
//!
//! - **core** — who the user is (`tier = core`). Every session.
//! - **candidates** — given a `cwd`, the scopes a row written here may take
//!   (see `memory::scope`), as one line the model reads once.
//! - **index** — given a `cwd`, the summaries of `indexed` rows filed at that
//!   directory or a parent, nearest first, within [`INDEX_BUDGET_CHARS`].
//!   One line each; the agent reads a row in full with `memory_get`.
//!
//! Everything else surfaces by subject through per-turn recall. A host that
//! sends no `cwd` gets core alone; `budget_chars` (sent by 1.9.0-era hosts)
//! is ignored.

use super::envelope::{ok, ApiError};
use super::state::SharedState;
use crate::ops;
use crate::ops::session::SessionStartRequest;
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};

pub fn router() -> Router<SharedState> {
    Router::new().route("/api/memory/session_start", post(session_start))
}

async fn session_start(
    State(state): State<SharedState>,
    Json(req): Json<SessionStartRequest>,
) -> Result<Response, ApiError> {
    Ok(ok(ops::session::session_start(&state, req).await?))
}
