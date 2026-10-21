//! `/api/connect/*` — a machine on the LAN asks to share this memory, and
//! someone at this machine clicks Allow or Deny. Nobody sees or types a token.
//!
//! Two open routes, for the machine asking (the gate lets them through, like
//! `/api/health`):
//! - `POST /api/connect/request {device_name, device_id}` → `{request_id,
//!   expires_in}`. A machine on the denied list is refused at once (403).
//! - `GET /api/connect/status/{request_id}` → pending | denied | expired |
//!   allowed; `allowed` carries the device token the first time only.
//!
//! The rest decide, and pass `local::require_local` in the handler — a paired
//! device's token is not enough to approve another, or itself:
//! - `GET /api/connect/pending`, `POST /api/connect/{id}/allow|deny`
//! - `GET /api/connect/devices`, `DELETE /api/connect/devices/{id}` (revoke)
//! - `GET /api/connect/denied`, `DELETE /api/connect/denied/{device_id}`
//!
//! Allow writes the token into `paired-devices.json` — the engine's file, the
//! one `gate.rs` reads — so `lan_gate` itself is unchanged.

mod broker;
mod devices;

pub use broker::Broker;

use axum::{
    extract::{ConnectInfo, Path, State},
    http::HeaderMap,
    response::Response,
    routing::{delete, get, post},
    Json, Router,
};
use broker::{expires_in, Pending, Status};
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;
use std::time::Instant;

use crate::http::envelope::{ok, ApiError};
use crate::http::local::require_local;
use crate::http::presence::{Presence, Seen};
use crate::http::state::SharedState;

/// Paths the gate leaves open: the asking machine has no token yet.
pub fn is_open_path(path: &str) -> bool {
    path == "/api/connect/request" || path.starts_with("/api/connect/status/")
}

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/api/connect/request", post(request))
        .route("/api/connect/status/{id}", get(status))
        .route("/api/connect/pending", get(pending))
        .route("/api/connect/{id}/allow", post(allow))
        .route("/api/connect/{id}/deny", post(deny))
        .route("/api/connect/devices", get(list_devices))
        .route("/api/connect/devices/{id}", delete(revoke))
        .route("/api/connect/denied", get(list_denied))
        .route("/api/connect/denied/{device_id}", delete(undeny))
}

#[derive(Deserialize)]
struct ConnectRequest {
    device_name: String,
    device_id: String,
}

fn io_error(e: std::io::Error) -> ApiError {
    ApiError::internal(anyhow::anyhow!(e))
}

/// A name for the console: printable, trimmed, at most 64 characters.
fn clean_name(raw: &str) -> Option<String> {
    let name: String = raw.chars().filter(|c| !c.is_control()).take(64).collect();
    let name = name.trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// A stable id the asking machine chose: 8–128 of `[A-Za-z0-9._:-]`.
fn valid_device_id(id: &str) -> bool {
    (8..=128).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
}

async fn request(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<ConnectRequest>,
) -> Result<Response, ApiError> {
    let now = Instant::now();
    if !state.connect.admit(peer.ip(), now) {
        return Err(ApiError::too_many("too many requests; wait a minute"));
    }
    let name = clean_name(&body.device_name)
        .ok_or_else(|| ApiError::bad_request("device_name is required"))?;
    if !valid_device_id(&body.device_id) {
        return Err(ApiError::bad_request(
            "device_id must be 8-128 of [A-Za-z0-9._:-]",
        ));
    }
    if devices::is_denied(&state.data_dir, &body.device_id) {
        return Err(ApiError::forbidden("this machine was denied"));
    }
    let p = state.connect.open(name, body.device_id, peer.ip(), now);
    tracing::info!("connect request from {} ({})", p.device_name, p.ip);
    Ok(ok(
        json!({ "request_id": p.id, "expires_in": expires_in(&p, now) }),
    ))
}

async fn status(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    Ok(ok(status_json(state.connect.status(&id, Instant::now()))))
}

fn status_json(status: Status) -> serde_json::Value {
    match status {
        Status::Pending => json!({ "state": "pending" }),
        Status::Denied => json!({ "state": "denied" }),
        Status::Expired => json!({ "state": "expired" }),
        Status::Allowed(Some(token)) => json!({ "state": "allowed", "device_token": token }),
        Status::Allowed(None) => json!({ "state": "allowed" }),
    }
}

fn pending_json(state: &SharedState, p: &Pending, now: Instant) -> serde_json::Value {
    json!({
        "id": p.id,
        "device_name": p.device_name,
        "device_id": p.device_id,
        "ip": p.ip,
        "expires_in": expires_in(p, now),
        "known": devices::is_known(&state.data_dir, &p.device_id),
    })
}

async fn pending(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    require_local(peer.ip(), &headers, state.port)?;
    let now = Instant::now();
    let p = state.connect.pending(now);
    Ok(ok(
        json!({ "pending": p.map(|p| pending_json(&state, &p, now)) }),
    ))
}

/// The live request with this id, taken off the table to be decided.
fn take(state: &SharedState, id: &str) -> Result<Pending, ApiError> {
    state
        .connect
        .take(id, Instant::now())
        .ok_or_else(|| ApiError::not_found("no such request, or it expired"))
}

async fn allow(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    require_local(peer.ip(), &headers, state.port)?;
    let p = take(&state, &id)?;
    let token =
        devices::allow(&state.data_dir, &p.device_id, &p.device_name, p.ip).map_err(io_error)?;
    state.connect.allow(p.id, token, Instant::now());
    tracing::info!("allowed {} ({})", p.device_name, p.ip);
    Ok(ok(json!({ "allowed": p.device_name })))
}

async fn deny(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    require_local(peer.ip(), &headers, state.port)?;
    let p = take(&state, &id)?;
    devices::deny(&state.data_dir, &p.device_id, &p.device_name, p.ip).map_err(io_error)?;
    state.connect.deny(p.id, Instant::now());
    tracing::info!("denied {} ({})", p.device_name, p.ip);
    Ok(ok(json!({ "denied": p.device_name })))
}

async fn list_devices(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    require_local(peer.ip(), &headers, state.port)?;
    let rows = devices::list_allowed(&state.data_dir).map_err(io_error)?;
    Ok(ok(devices_json(&state.presence, &rows, Instant::now())))
}

/// Each machine this daemon allowed, with what the gate saw of it. No secret
/// is in `Allowed`, so none can leak here.
fn devices_json(presence: &Presence, rows: &[devices::Allowed], now: Instant) -> serde_json::Value {
    let list: Vec<_> = rows
        .iter()
        .map(|a| device_json(a, presence.get(&a.id), now))
        .collect();
    let connected = list.iter().filter(|d| d["online"] == true).count();
    json!({
        "devices": list,
        "summary": { "connected": connected, "total": rows.len() },
    })
}

fn device_json(a: &devices::Allowed, seen: Option<Seen>, now: Instant) -> serde_json::Value {
    let wall = chrono::Utc::now().timestamp();
    json!({
        "id": a.id,
        "name": a.name,
        "ip": seen.map_or(a.ip, |s| s.ip),
        "allowed_at": a.allowed_at,
        "kind": if seen.is_some_and(|s| s.is_replica()) { "replica" } else { "client" },
        "last_seen": seen.map(|s| wall - s.age_secs(now)),
        "online": seen.is_some_and(|s| s.online(now)),
    })
}

async fn revoke(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    require_local(peer.ip(), &headers, state.port)?;
    if !devices::revoke(&state.data_dir, &id).map_err(io_error)? {
        return Err(ApiError::not_found("no such device"));
    }
    Ok(ok(json!({ "revoked": id })))
}

async fn list_denied(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    require_local(peer.ip(), &headers, state.port)?;
    let rows = devices::list_denied(&state.data_dir).map_err(io_error)?;
    Ok(ok(json!({ "denied": rows })))
}

async fn undeny(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Result<Response, ApiError> {
    require_local(peer.ip(), &headers, state.port)?;
    if !devices::undeny(&state.data_dir, &device_id).map_err(io_error)? {
        return Err(ApiError::not_found("not on the denied list"));
    }
    Ok(ok(json!({ "undenied": device_id })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_asking_routes_are_open() {
        assert!(is_open_path("/api/connect/request"));
        assert!(is_open_path("/api/connect/status/abc"));
        for path in [
            "/api/connect/pending",
            "/api/connect/abc/allow",
            "/api/connect/abc/deny",
            "/api/connect/devices",
            "/api/connect/denied/x",
        ] {
            assert!(!is_open_path(path), "{path}");
        }
    }

    #[test]
    fn names_and_ids_are_checked() {
        assert_eq!(clean_name("  DS242\n").as_deref(), Some("DS242"));
        assert_eq!(clean_name(" \u{7} "), None);
        assert_eq!(clean_name(&"x".repeat(100)).unwrap().len(), 64);
        assert!(valid_device_id("3f2a9c1e-0b7d-4e6a-9f00-1234567890ab"));
        assert!(!valid_device_id("short"));
        assert!(!valid_device_id("has space in it"));
        assert!(!valid_device_id("../../etc/passwd"));
    }

    #[test]
    fn the_status_carries_the_token_only_when_given_one() {
        let v = status_json(Status::Allowed(Some("cafe".into())));
        assert_eq!(v["device_token"], "cafe");
        assert!(status_json(Status::Allowed(None))
            .get("device_token")
            .is_none());
        assert_eq!(status_json(Status::Expired)["state"], "expired");
    }

    fn local_headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("host", "127.0.0.1:9528".parse().unwrap());
        h
    }

    fn peer(ip: &str) -> ConnectInfo<SocketAddr> {
        ConnectInfo(SocketAddr::new(ip.parse().unwrap(), 50000))
    }

    async fn state(dir: &std::path::Path) -> SharedState {
        crate::http::state::scratch_state(dir, "0.0.0.0:9528".parse().unwrap()).await
    }

    fn ask(state: &SharedState) -> Pending {
        state.connect.open(
            "DS242".into(),
            "3f2a9c1e-0b7d-4e6a".into(),
            "192.168.20.31".parse().unwrap(),
            Instant::now(),
        )
    }

    fn forbidden(r: Result<Response, ApiError>) -> bool {
        matches!(r, Err(e) if matches!(e.code, crate::http::envelope::ErrorCode::Forbidden))
    }

    /// A paired device passes the gate with its token; it still may not
    /// approve anyone, see who is asking, or list devices.
    #[tokio::test]
    async fn a_paired_device_cannot_approve_or_look() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path()).await;
        let p = ask(&st);
        let mut h = local_headers();
        h.insert("x-linggen-device", "cafe".parse().unwrap());
        let lan = "192.168.20.31";
        assert!(forbidden(
            allow(State(st.clone()), peer(lan), h.clone(), Path(p.id.clone())).await
        ));
        assert!(forbidden(
            deny(State(st.clone()), peer(lan), h.clone(), Path(p.id.clone())).await
        ));
        assert!(forbidden(
            pending(State(st.clone()), peer(lan), h.clone()).await
        ));
        assert!(forbidden(
            list_devices(State(st.clone()), peer(lan), h.clone()).await
        ));
        assert!(forbidden(
            list_denied(State(st.clone()), peer(lan), h.clone()).await
        ));
        assert!(forbidden(
            revoke(State(st.clone()), peer(lan), h.clone(), Path("x".into())).await
        ));
        assert!(forbidden(
            undeny(State(st.clone()), peer(lan), h, Path("x".into())).await
        ));
        assert!(
            st.connect.pending(Instant::now()).is_some(),
            "nothing was decided"
        );
    }

    /// Loopback socket, rebound Host: refused before anything is decided.
    #[tokio::test]
    async fn a_rebound_page_on_this_machine_cannot_approve() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path()).await;
        let p = ask(&st);
        let mut h = HeaderMap::new();
        h.insert("host", "evil.example:9528".parse().unwrap());
        assert!(forbidden(
            allow(State(st.clone()), peer("127.0.0.1"), h, Path(p.id.clone())).await
        ));
        let mut h = local_headers();
        h.insert("origin", "https://evil.example".parse().unwrap());
        assert!(forbidden(
            allow(State(st.clone()), peer("127.0.0.1"), h, Path(p.id)).await
        ));
        assert!(st.connect.pending(Instant::now()).is_some());
    }

    #[tokio::test]
    async fn allow_from_this_machine_pairs_and_hands_the_token_out_once() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path()).await;
        let p = ask(&st);
        allow(
            State(st.clone()),
            peer("127.0.0.1"),
            local_headers(),
            Path(p.id.clone()),
        )
        .await
        .unwrap();
        let Status::Allowed(Some(token)) = st.connect.status(&p.id, Instant::now()) else {
            panic!("token expected");
        };
        assert_eq!(
            st.connect.status(&p.id, Instant::now()),
            Status::Allowed(None)
        );
        let lan = "192.168.20.31".parse().unwrap();
        assert!(crate::http::gate::admits_for_test(
            lan,
            "/mcp",
            &token,
            dir.path()
        ));
    }

    #[test]
    fn devices_carry_presence_and_a_summary() {
        let row = |id: &str| devices::Allowed {
            id: id.into(),
            device_id: format!("dev-{id}"),
            name: "luffy-ubuntu".into(),
            ip: "192.168.20.242".parse().unwrap(),
            allowed_at: 1,
        };
        let presence = Presence::default();
        let now = Instant::now();
        presence.record(
            "a",
            "192.168.20.50".parse().unwrap(),
            "/api/sync/changes",
            now,
        );
        let v = devices_json(&presence, &[row("a"), row("b")], now);
        assert_eq!(v["summary"]["connected"], 1);
        assert_eq!(v["summary"]["total"], 2);
        let a = &v["devices"][0];
        assert_eq!(a["online"], true);
        assert_eq!(a["kind"], "replica");
        assert_eq!(a["ip"], "192.168.20.50");
        assert!(a["last_seen"].is_i64());
        assert!(a.get("secret").is_none());
        let b = &v["devices"][1];
        assert_eq!(b["online"], false);
        assert_eq!(b["kind"], "client");
        assert!(b["last_seen"].is_null());
        assert_eq!(b["ip"], "192.168.20.242");
    }

    #[tokio::test]
    async fn a_denied_machine_is_refused_without_a_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path()).await;
        let p = ask(&st);
        deny(
            State(st.clone()),
            peer("127.0.0.1"),
            local_headers(),
            Path(p.id),
        )
        .await
        .unwrap();
        let body = ConnectRequest {
            device_name: "DS242".into(),
            device_id: "3f2a9c1e-0b7d-4e6a".into(),
        };
        let r = request(State(st.clone()), peer("192.168.20.31"), Json(body)).await;
        assert!(forbidden(r));
        assert!(st.connect.pending(Instant::now()).is_none());
    }
}
