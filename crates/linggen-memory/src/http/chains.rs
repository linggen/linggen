//! `POST /api/memory/chains` — the condense stage's chain scan. The scans
//! (`cited`, `marker`, `subject`) are `ops::chains`.

use super::envelope::{ok, ApiError};
use super::state::SharedState;
use crate::ops;
use crate::ops::chains::ChainsRequest;
use axum::extract::State;
use axum::response::Response;
use axum::{Json, Router};

pub fn router() -> Router<SharedState> {
    use axum::routing::post;
    Router::new().route("/api/memory/chains", post(chains))
}

async fn chains(
    State(state): State<SharedState>,
    Json(req): Json<ChainsRequest>,
) -> Result<Response, ApiError> {
    Ok(ok(ops::chains::chains(&state, req).await?))
}
