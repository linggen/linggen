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
use std::net::SocketAddr;

use crate::http::envelope::{ok, ApiError};
use crate::http::local::require_local;
use crate::http::state::SharedState;
use crate::http::verb;
use crate::ops;
use crate::ops::primary::SetPrimary;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/api/sync/role", get(role))
        // The same facts at the memory-verb path, for MCP's `memory_upkeep` op=role.
        .route("/api/memory/role", axum::routing::post(role))
        .route("/api/sync/primary", put(set_primary))
}

async fn role(State(state): State<SharedState>) -> Response {
    verb::bare(state, "role").await
}

async fn set_primary(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<SetPrimary>,
) -> Result<Response, ApiError> {
    require_local(peer.ip(), &headers, state.port)?;
    Ok(ok(ops::primary::set_primary(&state, body).await?))
}
