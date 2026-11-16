//! `POST /api/memory/stats` — one-call store-state summary.
//!
//! The single source every surface renders from: the console header,
//! the memory app's dashboard, and the Linggen engine's mission-report
//! footer. The summary itself is `ops::stats`.

use super::envelope::{ok, ApiError};
use super::state::SharedState;
use crate::ops;
use axum::extract::State;
use axum::response::Response;
use axum::Router;

pub fn router() -> Router<SharedState> {
    use axum::routing::post;
    Router::new().route("/api/memory/stats", post(stats))
}

async fn stats(State(state): State<SharedState>) -> Result<Response, ApiError> {
    Ok(ok(ops::stats::stats(&state).await?))
}
