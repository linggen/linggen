//! Per-day dream state — the single source of truth for the dream
//! pipeline's calendar (see `../../linggen/doc/memory-spec.md` §2).
//!
//! Three endpoints under `/api/memory/`:
//!
//! - **`days`** — per-day rollup merging the stored day records with live
//!   episodic row counts. Drives the memory app's calendar, the dream
//!   mission's worklist ("undreamed days, oldest first"), and the CLI's
//!   `ling-mem days`. Each day carries per-verb flags (`scanned`,
//!   `dreamed`); the top level carries `first_unscanned` /
//!   `first_undreamed` plus past-day summary counts (`total_days`,
//!   `scanned_days`, `dreamed_days`) for status surfaces.
//! - **`remember_day`** — stamp a day `remembered_at` after a remember
//!   pass judged its rows. Called by whichever agent ran the pass
//!   (Linggen mission, skill page, CC/Codex host agent).
//! - **`sweep`** — the forget stage: mechanical, zero-LLM eviction of
//!   episodic rows that are past TTL, belong to a remembered day, and
//!   were created before that day's `remembered_at` (i.e. were judged).
//!   Rows arriving after the stamp survive and re-pend their day.
//!
//! State is **stored, not derived**, because remembering keeps rows —
//! "judged" can no longer be inferred from row absence. The store is a
//! small JSON sidecar at `<data_dir>/memory/.days.json` (`ops::days`, same
//! pattern as `.config.json`); it is not a LanceDB table, so the frozen
//! store schema is untouched.
//!
//! Day attribution: a row belongs to the **local calendar day** of its
//! `effective_timestamp()` (`occurred_at ?? created_at`) — the calendar
//! is a human-facing surface, so local days, not UTC.

use super::state::SharedState;
use super::verb;
use crate::ops::days::{DaysRequest, HarvestDayRequest, RememberDayRequest, SweepRequest};
use axum::extract::{Request, State};
use axum::response::Response;
use axum::Router;

pub fn router() -> Router<SharedState> {
    use axum::routing::post;
    Router::new()
        .route("/api/memory/days", post(days))
        .route("/api/memory/remember_day", post(remember_day))
        .route("/api/memory/harvest_day", post(harvest_day))
        .route("/api/memory/sweep", post(sweep))
}

async fn days(State(state): State<SharedState>, req: Request) -> Response {
    verb::serve::<DaysRequest>(state, "days", req).await
}

async fn remember_day(State(state): State<SharedState>, req: Request) -> Response {
    verb::serve::<RememberDayRequest>(state, "remember_day", req).await
}

async fn harvest_day(State(state): State<SharedState>, req: Request) -> Response {
    verb::serve::<HarvestDayRequest>(state, "harvest_day", req).await
}

async fn sweep(State(state): State<SharedState>, req: Request) -> Response {
    verb::serve::<SweepRequest>(state, "sweep", req).await
}
