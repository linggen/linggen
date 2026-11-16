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

use super::state::SharedState;
use super::verb;
use crate::ops::issues::{IssueAddRequest, IssueResolveRequest, IssuesRequest};
use axum::extract::{Request, State};
use axum::response::Response;
use axum::Router;

pub fn router() -> Router<SharedState> {
    use axum::routing::post;
    Router::new()
        .route("/api/memory/issues", post(issues))
        .route("/api/memory/issue_add", post(issue_add))
        .route("/api/memory/issue_resolve", post(issue_resolve))
}

async fn issues(State(state): State<SharedState>, req: Request) -> Response {
    verb::serve::<IssuesRequest>(state, "issues", req).await
}

async fn issue_add(State(state): State<SharedState>, req: Request) -> Response {
    verb::serve::<IssueAddRequest>(state, "issue_add", req).await
}

async fn issue_resolve(State(state): State<SharedState>, req: Request) -> Response {
    verb::serve::<IssueResolveRequest>(state, "issue_resolve", req).await
}
