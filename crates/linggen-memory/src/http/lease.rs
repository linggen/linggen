//! The dream lock — one executor per day.
//!
//! The engine's dream mission and a Claude Code `/linggen:dream` could judge
//! the same day at once, promoting rows twice. So a dream takes a per-day
//! lease first, on the primary (a replica refuses — `replica::forward`):
//!
//! `POST /api/memory/dream_lease {day, holder, action: acquire|renew|release}`
//!
//! - `acquire` — granted when the day is free, already this holder's, or its
//!   lease ran out; otherwise refused (409) naming who holds it.
//! - `renew` — push the expiry out; only the holder may.
//! - `release` — free the day; only the holder may (an expired lease frees
//!   itself).
//!
//! A lease lasts `LEASE_FOR` and is renewed while a long pass runs.
//! `remember_day` also refuses a day another holder has leased.
//!
//! State: `<data_dir>/memory/.dream-lease.json`, `{day: {holder, expires_at}}`.

use axum::{extract::State, response::Response, routing::post, Json, Router};

use crate::http::envelope::{ok, ApiError};
use crate::http::state::SharedState;
use crate::ops;
use crate::ops::lease::LeaseRequest;

pub fn router() -> Router<SharedState> {
    Router::new().route("/api/memory/dream_lease", post(dream_lease))
}

async fn dream_lease(
    State(state): State<SharedState>,
    Json(req): Json<LeaseRequest>,
) -> Result<Response, ApiError> {
    Ok(ok(ops::lease::dream_lease(&state, req).await?))
}
