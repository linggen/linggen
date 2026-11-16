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
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::http::envelope::{ok, ApiError};
use crate::http::state::SharedState;
use crate::ops::lease::{busy, decide, Verdict};

pub fn router() -> Router<SharedState> {
    Router::new().route("/api/memory/dream_lease", post(dream_lease))
}

#[derive(Deserialize)]
struct LeaseRequest {
    day: String,
    holder: String,
    #[serde(default = "default_action")]
    action: String,
}

fn default_action() -> String {
    "acquire".into()
}

async fn dream_lease(
    State(state): State<SharedState>,
    Json(req): Json<LeaseRequest>,
) -> Result<Response, ApiError> {
    if !matches!(req.action.as_str(), "acquire" | "renew" | "release") {
        return Err(ApiError::bad_request("action is acquire, renew or release"));
    }
    if req.holder.trim().is_empty()
        || chrono::NaiveDate::parse_from_str(&req.day, "%Y-%m-%d").is_err()
    {
        return Err(ApiError::bad_request(
            "day is YYYY-MM-DD and holder is required",
        ));
    }
    let verdict = decide(
        &state.data_dir,
        &req.day,
        req.holder.trim(),
        &req.action,
        Utc::now(),
    )
    .map_err(|e| ApiError::internal(anyhow::anyhow!(e)))?;
    match verdict {
        Verdict::Held(l) => Err(busy(&req.day, &l)),
        Verdict::Granted(l) => Ok(ok(json!({ "day": req.day, "lease": l }))),
        Verdict::Released => Ok(ok(json!({ "day": req.day, "released": true }))),
    }
}
