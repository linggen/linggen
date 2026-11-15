//! `/api/sync/role` and `/api/sync/primary` — whether this daemon is the
//! primary or a replica of another machine's (`crate::replica`).
//!
//! - `GET /api/sync/role` — role, upstream, approved, online, last sync,
//!   queued writes, and why a sync did not happen: facts the console, the
//!   engine's Settings and `/linggen:config` show as-is. Never the token.
//! - `PUT /api/sync/primary {primary: "local" | "host:port"}` — this machine
//!   only (`local::require_local`). A remote primary is asked to connect;
//!   someone clicks Allow there, and this daemon starts following it.

use axum::{
    extract::{ConnectInfo, State},
    http::HeaderMap,
    response::Response,
    routing::{get, put},
    Json, Router,
};
use serde::Deserialize;
use std::net::SocketAddr;

use crate::http::envelope::{ok, ApiError};
use crate::http::local::require_local;
use crate::http::state::SharedState;
use crate::replica::{link, outbox, state::normalize_primary};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/api/sync/role", get(role))
        // The same facts at the memory-verb path, for `/mcp`'s `memory_role`.
        .route("/api/memory/role", axum::routing::post(role))
        .route("/api/sync/primary", put(set_primary))
}

fn view(state: &SharedState) -> serde_json::Value {
    state.replica.snapshot().view(outbox::len(&state.data_dir))
}

async fn role(State(state): State<SharedState>) -> Result<Response, ApiError> {
    Ok(ok(view(&state)))
}

#[derive(Deserialize)]
struct SetPrimary {
    primary: String,
}

async fn set_primary(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<SetPrimary>,
) -> Result<Response, ApiError> {
    require_local(peer.ip(), &headers, state.port)?;
    let wanted = body.primary.trim();
    if wanted.is_empty() || wanted.eq_ignore_ascii_case("local") {
        link::become_primary(&state);
        return Ok(ok(view(&state)));
    }
    let primary = normalize_primary(wanted)
        .ok_or_else(|| ApiError::bad_request("an address like 192.168.1.5:9528, or local"))?;
    link::ask(&state, primary)
        .await
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    Ok(ok(view(&state)))
}
