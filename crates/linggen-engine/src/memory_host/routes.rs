//! Memory's routes on the engine's port: `/mem/*` (memory's raw routes, the
//! prefix stripped) and `/mcp/memory` (memory's MCP server).
//!
//! Who gets through (`caller`, after the engine's LAN gate):
//! - this machine: everything;
//! - an approved machine (a `machine` row's token): `/mcp/memory`, for a
//!   machine with only a plugin, and the replica protocol under `/mem` —
//!   join, pull, primary state, the push of queued writes ([`machine_may`]);
//! - a phone: nothing here (403) — its door is the stamped
//!   `/api/memory/<verb>`;
//! - no token: only the two connect routes a machine asks with.
//!
//! Each request is handed to memory's runtime and answered there, so the
//! store's work never runs on the engine's threads.

use super::caller::{self, Caller};
use super::{HOST, MCP_PATH, PREFIX};
use axum::extract::{ConnectInfo, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::json;
use std::net::SocketAddr;
use tower::ServiceExt;

pub fn routes<S>() -> Router<S>
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
    forward(peer, req, &path, Door::Raw).await
}

async fn mcp(ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    forward(peer, req, "/mcp", Door::Mcp).await
}

/// The paths under [`PREFIX`] the engine's LAN gate leaves open: a machine
/// asking to connect has no token yet.
pub fn is_open_path(path: &str) -> bool {
    path.strip_prefix(PREFIX).is_some_and(is_connect_open)
}

/// Which door a request came in by.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Door {
    /// `/mem/*` — memory's own routes, the prefix stripped.
    Raw,
    /// `/mcp/memory` — memory's MCP server.
    Mcp,
}

/// What a caller may do at a door: go through (naming the approved machine,
/// for presence), or be refused with a reason.
#[derive(Debug, PartialEq)]
enum Admit {
    Through(Option<String>),
    Refused(StatusCode, &'static str, &'static str),
}

/// Memory's routes a machine reaches over the network: the replica protocol
/// (join, pull, primary state, push of queued writes). Everything else under
/// `/mem` — settings, the listener, the schema step, Allow / Deny — is this
/// machine's alone.
fn machine_may(path: &str) -> bool {
    is_connect_open(path)
        || matches!(
            path,
            "/api/sync/changes" | "/api/sync/rows" | "/api/sync/role"
        )
        || ling_mem::replica::forward::WRITES.contains(&path)
}

/// The two connect routes an asking machine uses before it holds a token.
fn is_connect_open(path: &str) -> bool {
    path == "/api/connect/request" || path.starts_with("/api/connect/status/")
}

fn admit(caller: &Caller, door: Door, path: &str) -> Admit {
    match caller {
        Caller::Owner => Admit::Through(None),
        Caller::Machine(d) if door == Door::Mcp || machine_may(path) => {
            Admit::Through(Some(d.id.clone()))
        }
        Caller::Machine(_) => Admit::Refused(
            StatusCode::FORBIDDEN,
            "an approved machine reaches the replica protocol and /mcp/memory, not this",
            "MACHINE_NOT_ALLOWED",
        ),
        Caller::Phone(_) => Admit::Refused(
            StatusCode::FORBIDDEN,
            "a phone reaches memory at /api/memory/<verb>, and sees its own person's rows",
            "PHONE_NOT_ALLOWED",
        ),
        Caller::Unknown if door == Door::Raw && is_connect_open(path) => Admit::Through(None),
        Caller::Unknown => Admit::Refused(
            StatusCode::UNAUTHORIZED,
            "memory answers this machine and the machines it approved",
            "PAIRING_REQUIRED",
        ),
    }
}

async fn forward(peer: SocketAddr, mut req: Request, path: &str, door: Door) -> Response {
    let machine = match admit(&caller::resolve(peer.ip(), req.headers()), door, path) {
        Admit::Through(machine) => machine,
        Admit::Refused(status, why, code) => return refuse(status, why, code),
    };
    let Some(host) = HOST.get() else {
        return unavailable("memory is not hosted by this process");
    };
    let served = match &host.served {
        Ok(served) => served,
        Err(why) => return unavailable(why),
    };
    if let Some(id) = machine {
        let now = std::time::Instant::now();
        served.state.presence.record(&id, peer.ip(), path, now);
    }
    if let Err(e) = set_path(&mut req, path) {
        return refuse(StatusCode::BAD_REQUEST, &e, "BAD_PATH");
    }
    let req = only_the_peer(req, peer);
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

/// The request as memory's router should see it: the engine's own route
/// state (its `{*rest}` path parameter above all, which memory's `Path`
/// extractors would read as one argument too many) left behind, the
/// caller's socket address kept.
fn only_the_peer(req: Request, peer: SocketAddr) -> Request {
    let (mut parts, body) = req.into_parts();
    parts.extensions = Default::default();
    parts.extensions.insert(ConnectInfo(peer));
    Request::from_parts(parts, body)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_fs::devices::{DeviceKind, PairedDevice};
    use axum::body::Body;
    use axum::http::Request;

    async fn status_from(peer: &str, path: &str) -> StatusCode {
        let mut req = Request::get(path).body(Body::empty()).unwrap();
        let peer: SocketAddr = peer.parse().unwrap();
        req.extensions_mut().insert(ConnectInfo(peer));
        routes::<()>().oneshot(req).await.unwrap().status()
    }

    fn device(kind: DeviceKind) -> PairedDevice {
        PairedDevice {
            id: "row".into(),
            name: "row".into(),
            secret: "s".into(),
            created_at: 0,
            kind: Some(kind),
            account: None,
            device_id: None,
            settings: Default::default(),
            relay_grant: None,
            superseded_grant: None,
            ..Default::default()
        }
    }

    fn through(a: Admit) -> bool {
        matches!(a, Admit::Through(_))
    }

    fn refused_with(a: Admit) -> Option<StatusCode> {
        match a {
            Admit::Refused(status, ..) => Some(status),
            Admit::Through(_) => None,
        }
    }

    /// Without a token, off this machine: only the asking routes; nothing of
    /// memory itself (the engine's gate stops these first; this is the
    /// second fence).
    #[tokio::test]
    async fn without_a_token_only_the_connect_routes_answer() {
        for path in ["/mem/api/memory/list", "/mem", "/mcp/memory"] {
            assert_eq!(
                status_from("192.168.1.5:50000", path).await,
                StatusCode::UNAUTHORIZED,
                "{path}"
            );
        }
        // Past the fence, no memory hosted in this test: 503.
        for path in ["/mem/api/connect/request", "/mem/api/connect/status/x"] {
            assert_eq!(
                status_from("192.168.1.5:50000", path).await,
                StatusCode::SERVICE_UNAVAILABLE,
                "{path}"
            );
        }
        assert_eq!(
            status_from("127.0.0.1:50000", "/mem/api/health").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    /// The engine's `{*rest}` parameter must not reach memory's router: its
    /// `Path` extractors would read it as one argument too many.
    #[test]
    fn memory_sees_the_peer_and_none_of_the_engines_route_state() {
        let mut req = Request::get("/x").body(Body::empty()).unwrap();
        req.extensions_mut().insert(42u8);
        let peer: SocketAddr = "192.168.1.5:1".parse().unwrap();
        let req = only_the_peer(req, peer);
        assert!(req.extensions().get::<u8>().is_none());
        let seen = req.extensions().get::<ConnectInfo<SocketAddr>>().unwrap();
        assert_eq!(seen.0, peer);
    }

    #[test]
    fn a_machine_reaches_the_replica_protocol_and_the_mcp_server() {
        let mac = Caller::Machine(device(DeviceKind::Machine));
        assert_eq!(
            admit(&mac, Door::Mcp, "/mcp"),
            Admit::Through(Some("row".into()))
        );
        for path in [
            "/api/connect/request",
            "/api/connect/status/abc",
            "/api/sync/changes",
            "/api/sync/rows",
            "/api/sync/role",
            "/api/memory/add",
            "/api/memory/update",
            "/api/memory/issue_add",
        ] {
            assert!(through(admit(&mac, Door::Raw, path)), "{path}");
        }
        for path in [
            "/api/config",
            "/api/sync/primary",
            "/api/connect/pending",
            "/api/connect/abc/allow",
            "/api/connect/devices",
            "/api/schema/apply",
            "/api/memory/accounts",
            "/api/debug/compact",
        ] {
            assert_eq!(
                refused_with(admit(&mac, Door::Raw, path)),
                Some(StatusCode::FORBIDDEN),
                "{path}"
            );
        }
    }

    #[test]
    fn a_phone_is_refused_at_every_door_here() {
        let phone = Caller::Phone(device(DeviceKind::Phone));
        assert_eq!(
            refused_with(admit(&phone, Door::Mcp, "/mcp")),
            Some(StatusCode::FORBIDDEN)
        );
        for path in [
            "/api/connect/request",
            "/api/sync/changes",
            "/api/sync/role",
            "/api/memory/add",
            "/api/memory/search",
        ] {
            assert_eq!(
                refused_with(admit(&phone, Door::Raw, path)),
                Some(StatusCode::FORBIDDEN),
                "{path}"
            );
        }
    }

    #[test]
    fn this_machine_reaches_everything_and_no_one_reaches_only_the_asking() {
        assert!(through(admit(&Caller::Owner, Door::Raw, "/api/config")));
        assert!(through(admit(&Caller::Owner, Door::Mcp, "/mcp")));
        assert!(through(admit(
            &Caller::Unknown,
            Door::Raw,
            "/api/connect/request"
        )));
        assert_eq!(
            refused_with(admit(&Caller::Unknown, Door::Mcp, "/mcp")),
            Some(StatusCode::UNAUTHORIZED)
        );
        assert!(is_open_path("/mem/api/connect/request"));
        assert!(is_open_path("/mem/api/connect/status/abc"));
        assert!(!is_open_path("/mem/api/connect/pending"));
        assert!(!is_open_path("/api/connect/request"));
        assert!(!is_open_path("/mcp/memory"));
    }
}
