//! Settings → Memory → Primary: this machine's memory is the primary, or a
//! replica of another machine's. Memory owns the setting (`/api/sync/*`);
//! these routes call it in-process (`memory_host::local_call`) — the
//! primary's address never reaches a client, nor its token.
//!
//! - `GET /api/ling-mem/primary` — role, upstream, online, last sync, queued;
//!   for the owner (this machine, its own page, the owner's relay login).
//! - `PUT /api/ling-mem/primary {primary}` — someone at this Mac only.

use axum::{
    extract::ConnectInfo,
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::net::SocketAddr;

use crate::memory_host::caller::{self, Caller};
use crate::server::api::ling_mem_connect::{is_at_this_mac, refused};

async fn relay(method: Method, path: &str, body: Option<Value>) -> Response {
    let (status, body) = crate::memory_host::local_call(method, path, body).await;
    (status, Json(body)).into_response()
}

/// `GET /api/ling-mem/primary`
pub async fn get_primary(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if !matches!(caller::resolve(peer.ip(), &headers), Caller::Owner) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "ok": false, "error": "the owner's setting" })),
        )
            .into_response();
    }
    relay(Method::GET, "/api/sync/role", None).await
}

/// `PUT /api/ling-mem/primary`
pub async fn put_primary(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !is_at_this_mac(peer, &headers) {
        return refused();
    }
    let primary = body["primary"].as_str().unwrap_or("local").to_string();
    relay(
        Method::PUT,
        "/api/sync/primary",
        Some(json!({ "primary": primary })),
    )
    .await
}
