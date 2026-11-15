//! Memory's routes on the engine's port: `/mem/*` (memory's raw routes, the
//! prefix stripped) and `/mcp/memory` (memory's MCP server). Loopback callers
//! only — a paired device that passed the engine's LAN gate is still refused
//! here; the phone's door is the engine's stamped `/api/memory/<verb>`.
//!
//! Each request is handed to memory's runtime and answered there, so the
//! store's work never runs on the engine's threads.

use super::{HOST, MCP_PATH, PREFIX};
use axum::extract::{ConnectInfo, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::json;
use std::net::SocketAddr;
use tower::ServiceExt;

pub(crate) fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route(MCP_PATH, any(mcp))
        .route(PREFIX, any(raw))
        .route(&format!("{PREFIX}/{{*rest}}"), any(raw))
}

async fn raw(ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    let path = match req.uri().path().strip_prefix(PREFIX) {
        Some(rest) if !rest.is_empty() => rest.to_string(),
        _ => "/".to_string(),
    };
    forward(peer, req, &path).await
}

async fn mcp(ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    forward(peer, req, "/mcp").await
}

async fn forward(peer: SocketAddr, mut req: Request, path: &str) -> Response {
    if !peer.ip().is_loopback() {
        return refuse(
            StatusCode::FORBIDDEN,
            "memory's own routes answer this machine only",
            "LOOPBACK_ONLY",
        );
    }
    let Some(host) = HOST.get() else {
        return unavailable("memory is not hosted by this process");
    };
    let served = match &host.served {
        Ok(served) => served,
        Err(why) => return unavailable(why),
    };
    if let Err(e) = set_path(&mut req, path) {
        return refuse(StatusCode::BAD_REQUEST, &e, "BAD_PATH");
    }
    let router = served.router.clone();
    match host
        .runtime
        .spawn(async move { router.oneshot(req).await })
        .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(never)) => match never {},
        Err(e) => refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("memory request failed: {e}"),
            "INTERNAL",
        ),
    }
}

/// Point the request at memory's own path, keeping its query.
fn set_path(req: &mut Request, path: &str) -> Result<(), String> {
    let target = match req.uri().query() {
        Some(q) => format!("{path}?{q}"),
        None => path.to_string(),
    };
    *req.uri_mut() = target
        .parse()
        .map_err(|e| format!("bad path {target}: {e}"))?;
    Ok(())
}

fn unavailable(why: &str) -> Response {
    refuse(StatusCode::SERVICE_UNAVAILABLE, why, "MEMORY_UNAVAILABLE")
}

/// Memory's own error envelope, so every memory caller reads one shape.
fn refuse(status: StatusCode, error: &str, code: &str) -> Response {
    (
        status,
        Json(json!({ "ok": false, "error": error, "code": code })),
    )
        .into_response()
}
