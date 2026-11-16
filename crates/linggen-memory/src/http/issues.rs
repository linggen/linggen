//! Review-queue issues — the audit stage's holding pen (see
//! `../../linggen/doc/memory-spec.md`).
//!
//! The dream audit queues what it cannot solve with confidence: uncertain
//! chain merges, stale status claims, contradictions touching user-voice
//! rows. Memory only bookkeeps — no LLM lives here. Resolution happens
//! where intelligence does: a host agent (`/linggen:solve` on Claude Code,
//! the memory app's Review inbox) reads the facts, gathers evidence, asks
//! the user when needed, writes the fix through the normal memory verbs,
//! then marks the issue resolved.
//!
//! Three endpoints under `/api/memory/`:
//!
//! - **`issues`** — list (default `status=open`) + the open count.
//! - **`issue_add`** — queue one item. Idempotent per `(kind, row_ids)`:
//!   the nightly audit re-detects the same suspects until they are fixed,
//!   and re-queuing must not grow the list.
//! - **`issue_resolve`** — close one item by id (`resolved` | `dismissed`).
//!
//! State is a JSON sidecar at `<data_dir>/memory/.issues.json` (`ops::issues`,
//! same pattern as `.days.json`) — not a LanceDB table, the frozen store
//! schema is untouched.

use super::envelope::{ok, ApiError};
use super::state::SharedState;
use crate::memory::types::short_id;
use crate::ops::issues::{load, save, IssueRecord, KINDS, OUTCOMES};
use axum::extract::State;
use axum::response::Response;
use axum::{Json, Router};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

pub fn router() -> Router<SharedState> {
    use axum::routing::post;
    Router::new()
        .route("/api/memory/issues", post(issues))
        .route("/api/memory/issue_add", post(issue_add))
        .route("/api/memory/issue_resolve", post(issue_resolve))
}

// ── Handlers ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct IssuesRequest {
    /// `open` (default) | `resolved` | `dismissed` | `all`.
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// POST /api/memory/issues — list the queue.
async fn issues(
    State(state): State<SharedState>,
    Json(req): Json<IssuesRequest>,
) -> Result<Response, ApiError> {
    let want = req.status.as_deref().unwrap_or("open");
    if !matches!(want, "open" | "resolved" | "dismissed" | "all") {
        return Err(ApiError::bad_request(format!(
            "invalid status {want:?}: expected open | resolved | dismissed | all"
        )));
    }
    let all = load(&state.data_dir).await.issues;
    let open = all.iter().filter(|i| i.status == "open").count();
    let limit = req.limit.unwrap_or(50);
    let items: Vec<&IssueRecord> = all
        .iter()
        .filter(|i| want == "all" || i.status == want)
        .take(limit)
        .collect();
    Ok(ok(json!({ "open_count": open, "issues": items })))
}

#[derive(Debug, Deserialize)]
struct IssueAddRequest {
    kind: String,
    #[serde(default)]
    row_ids: Vec<String>,
    note: String,
}

/// POST /api/memory/issue_add — queue one review item. Re-adding an open
/// item with the same `(kind, row_ids)` returns the existing record — the
/// nightly audit re-detects unfixed suspects and must not grow the queue.
async fn issue_add(
    State(state): State<SharedState>,
    Json(req): Json<IssueAddRequest>,
) -> Result<Response, ApiError> {
    let kind = req.kind.trim();
    if !KINDS.contains(&kind) {
        return Err(ApiError::bad_request(format!(
            "invalid kind {kind:?}: expected one of {KINDS:?}"
        )));
    }
    if req.note.trim().is_empty() {
        return Err(ApiError::bad_request("note is required".to_string()));
    }
    let mut rows = req.row_ids.clone();
    rows.sort();
    let mut all = load(&state.data_dir).await;
    if let Some(existing) = all.issues.iter().find(|i| {
        i.status == "open" && i.kind == kind && {
            let mut theirs = i.row_ids.clone();
            theirs.sort();
            theirs == rows
        }
    }) {
        return Ok(ok(json!({ "issue": existing, "deduped": true })));
    }
    let record = IssueRecord {
        id: short_id(),
        kind: kind.to_string(),
        row_ids: req.row_ids,
        note: req.note.trim().to_string(),
        created_at: Utc::now(),
        status: "open".to_string(),
        resolved_at: None,
        resolution: None,
    };
    all.issues.push(record.clone());
    save(&state.data_dir, &all)
        .await
        .map_err(|e| ApiError::internal(anyhow::anyhow!("persisting .issues.json: {e}")))?;
    Ok(ok(json!({ "issue": record, "deduped": false })))
}

#[derive(Debug, Deserialize)]
struct IssueResolveRequest {
    id: String,
    /// `resolved` | `dismissed`.
    outcome: String,
    /// Solver's one-line record of what was done.
    #[serde(default)]
    note: Option<String>,
}

/// POST /api/memory/issue_resolve — close one item by id. Closing an
/// already-closed item is success (idempotent for retrying agents).
async fn issue_resolve(
    State(state): State<SharedState>,
    Json(req): Json<IssueResolveRequest>,
) -> Result<Response, ApiError> {
    let outcome = req.outcome.trim();
    if !OUTCOMES.contains(&outcome) {
        return Err(ApiError::bad_request(format!(
            "invalid outcome {outcome:?}: expected resolved | dismissed"
        )));
    }
    let mut all = load(&state.data_dir).await;
    let Some(item) = all.issues.iter_mut().find(|i| i.id == req.id) else {
        return Err(ApiError::bad_request(format!(
            "no issue with id {:?}",
            req.id
        )));
    };
    if item.status != "open" {
        let already = item.clone();
        return Ok(ok(json!({ "issue": already, "already_closed": true })));
    }
    item.status = outcome.to_string();
    item.resolved_at = Some(Utc::now());
    if let Some(note) = req.note.filter(|n| !n.trim().is_empty()) {
        item.resolution = Some(note.trim().to_string());
    }
    let updated = item.clone();
    save(&state.data_dir, &all)
        .await
        .map_err(|e| ApiError::internal(anyhow::anyhow!("persisting .issues.json: {e}")))?;
    Ok(ok(json!({ "issue": updated, "already_closed": false })))
}
