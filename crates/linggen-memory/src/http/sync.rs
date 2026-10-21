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

use axum::{
    extract::{Query, State},
    response::Response,
    routing::get,
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::http::envelope::{ok, ApiError};
use crate::http::state::SharedState;
use crate::memory::changes::Entry;

const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 1000;

pub fn router() -> Router<SharedState> {
    Router::new().route("/api/sync/changes", get(changes))
}

#[derive(Deserialize)]
struct ChangesQuery {
    #[serde(default)]
    since: u64,
    limit: Option<usize>,
}

/// The embedding model this store's vectors come from.
pub fn model_id() -> &'static str {
    crate::embed::MODEL_REPO
}

async fn changes(
    State(state): State<SharedState>,
    Query(q): Query<ChangesQuery>,
) -> Result<Response, ApiError> {
    let log = state
        .store
        .changes()
        .ok_or_else(|| ApiError::internal(anyhow::anyhow!("no change log on this daemon")))?;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let (entries, next, more) = log.since(q.since, limit);
    let (rows, tombstones) = bodies(&state, &entries).await?;
    Ok(ok(json!({
        "model": model_id(),
        "head": log.head(),
        "next": next,
        "more": more,
        "rows": rows,
        "tombstones": tombstones,
        "days": read_sidecar(&crate::http::days::days_path(&state.data_dir)),
        "issues": read_sidecar(&crate::http::issues::issues_path(&state.data_dir)),
    })))
}

/// Each id's current row and table, or a tombstone when it is gone.
async fn bodies(
    state: &SharedState,
    entries: &[Entry],
) -> Result<(Vec<Value>, Vec<String>), ApiError> {
    let mut rows = Vec::new();
    let mut tombstones = Vec::new();
    for e in entries {
        match current(state, &e.id).await? {
            Some((table, row)) => rows.push(json!({ "table": table, "row": row })),
            None => tombstones.push(e.id.clone()),
        }
    }
    Ok((rows, tombstones))
}

async fn current(state: &SharedState, id: &str) -> Result<Option<(&'static str, Value)>, ApiError> {
    let err = |e: anyhow::Error| ApiError::internal(e);
    if let Some(row) = state.store.get(id).await.map_err(err)? {
        return Ok(Some((
            "semantic",
            serde_json::to_value(row).unwrap_or(Value::Null),
        )));
    }
    if let Some(row) = state.episodic.get(id).await.map_err(err)? {
        return Ok(Some((
            "episodic",
            serde_json::to_value(row).unwrap_or(Value::Null),
        )));
    }
    Ok(None)
}

fn read_sidecar(path: &std::path::Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}
