//! `POST /api/debug/compact` — run the storage upkeep now: compact and prune
//! both tables, the pass `ops::upkeep` otherwise runs only after a
//! quiet spell. Hidden (no CLI verb); for measuring what a compaction does to
//! everything else while it runs. Memory's routes answer this machine only.

use super::envelope::{ok, ApiError};
use super::state::SharedState;
use crate::ops;
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::Router;

pub fn router() -> Router<SharedState> {
    Router::new().route("/api/debug/compact", post(compact))
}

async fn compact(State(state): State<SharedState>) -> Result<Response, ApiError> {
    Ok(ok(ops::compact::compact(&state).await?))
}
