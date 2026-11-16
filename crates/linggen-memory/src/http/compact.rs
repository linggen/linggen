//! `POST /api/debug/compact` — run the storage upkeep now: compact and prune
//! both tables, the pass `ops::upkeep` otherwise runs only after a
//! quiet spell. Hidden (no CLI verb); for measuring what a compaction does to
//! everything else while it runs. Memory's routes answer this machine only.

use super::envelope::{ok, ApiError};
use super::state::SharedState;
use crate::memory::maintenance::PRUNE_OLDER_THAN_DAYS;
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use serde_json::json;

pub fn router() -> Router<SharedState> {
    Router::new().route("/api/debug/compact", post(compact))
}

async fn compact(State(state): State<SharedState>) -> Result<Response, ApiError> {
    let started = std::time::Instant::now();
    let prune = chrono::Duration::days(PRUNE_OLDER_THAN_DAYS);
    for store in [&state.store, &state.episodic] {
        store
            .optimize_storage(prune)
            .await
            .map_err(ApiError::internal)?;
    }
    Ok(ok(json!({
        "compacted": [state.store.table_name(), state.episodic.table_name()],
        "ms": started.elapsed().as_millis() as u64,
    })))
}
