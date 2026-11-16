//! `POST /api/memory/chains` — the condense stage's chain scan. The scans
//! (`cited`, `marker`, `subject`) are `ops::chains`.

use super::state::SharedState;
use super::verb;
use crate::ops::chains::ChainsRequest;
use axum::extract::{Request, State};
use axum::response::Response;
use axum::Router;

pub fn router() -> Router<SharedState> {
    use axum::routing::post;
    Router::new().route("/api/memory/chains", post(chains))
}

async fn chains(State(state): State<SharedState>, req: Request) -> Response {
    verb::serve::<ChainsRequest>(state, "chains", req).await
}
