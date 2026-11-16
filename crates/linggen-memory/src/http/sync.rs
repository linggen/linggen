//! `GET /api/sync/changes?since=<cursor>&limit=<n>` — what a replica pulls
//! from its primary.
//!
//! The answer names every row changed after `since` (from the change log,
//! `memory::changes`) with its CURRENT body, vector included, and the table
//! it lives in now; a row that no longer exists comes back as a tombstone.
//! It also carries the two small sidecars a replica mirrors whole (the day
//! records and the review queue), and the embedding model, so a replica with
//! a different model refuses to mix vectors.
//!
//! Who may ask: the gate's rule — this machine, or a paired device.

use crate::http::envelope::{ok, ApiError};
use crate::http::state::SharedState;
use crate::ops;
use crate::ops::sync::{ChangesQuery, RowsQuery};
use axum::{
    extract::{Query, State},
    response::Response,
    routing::get,
    Router,
};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/api/sync/changes", get(changes))
        .route("/api/sync/rows", get(rows))
}

/// `GET /api/sync/rows?ids=a,b` — these rows as they are now (or tombstones):
/// how a replica restores rows after the primary refused a queued write.
async fn rows(
    State(state): State<SharedState>,
    Query(q): Query<RowsQuery>,
) -> Result<Response, ApiError> {
    Ok(ok(ops::sync::rows(&state, q).await?))
}

async fn changes(
    State(state): State<SharedState>,
    Query(q): Query<ChangesQuery>,
) -> Result<Response, ApiError> {
    Ok(ok(ops::sync::changes(&state, q).await?))
}
