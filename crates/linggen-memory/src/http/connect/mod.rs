//! `/api/connect/*` — a machine on the LAN asks to share this memory, and
//! someone at this machine clicks Allow or Deny. Nobody sees or types a token.
//!
//! Two routes for the machine asking:
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
//! Allow writes the token into `paired-devices.json` — the engine's file.

mod broker;
pub(crate) mod devices;

pub use broker::Broker;
pub(crate) use broker::{expires_in, Pending, Status};

use axum::{
    extract::{ConnectInfo, Path, State},
    http::HeaderMap,
    response::Response,
    routing::{delete, get, post},
    Json, Router,
};
use std::net::SocketAddr;

use crate::http::envelope::{ok, ApiError};
use crate::http::local::require_local;
use crate::http::state::SharedState;
use crate::ops;
use crate::ops::connect::ConnectRequest;

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

async fn request(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<ConnectRequest>,
) -> Result<Response, ApiError> {
    Ok(ok(ops::connect::request(&state, peer.ip(), body).await?))
}

async fn status(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    Ok(ok(ops::connect::status(&state, &id).await?))
}

async fn pending(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    require_local(peer.ip(), &headers, state.port)?;
    Ok(ok(ops::connect::pending(&state).await?))
}

async fn allow(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    require_local(peer.ip(), &headers, state.port)?;
    Ok(ok(ops::connect::allow(&state, &id).await?))
}

async fn deny(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    require_local(peer.ip(), &headers, state.port)?;
    Ok(ok(ops::connect::deny(&state, &id).await?))
}

async fn list_devices(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    require_local(peer.ip(), &headers, state.port)?;
    Ok(ok(ops::connect::list_devices(&state).await?))
}

async fn revoke(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    require_local(peer.ip(), &headers, state.port)?;
    Ok(ok(ops::connect::revoke(&state, &id).await?))
}

async fn list_denied(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    require_local(peer.ip(), &headers, state.port)?;
    Ok(ok(ops::connect::list_denied(&state).await?))
}

async fn undeny(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Result<Response, ApiError> {
    require_local(peer.ip(), &headers, state.port)?;
    Ok(ok(ops::connect::undeny(&state, &device_id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn local_headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("host", "127.0.0.1:9527".parse().unwrap());
        h
    }

    fn peer(ip: &str) -> ConnectInfo<SocketAddr> {
        ConnectInfo(SocketAddr::new(ip.parse().unwrap(), 50000))
    }

    async fn state(dir: &std::path::Path) -> SharedState {
        crate::http::state::scratch_state(dir, "0.0.0.0:9527".parse().unwrap()).await
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
        h.insert("host", "evil.example:9527".parse().unwrap());
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
        let paired: Vec<serde_json::Value> = serde_json::from_slice(
            &std::fs::read(devices::devices_path(dir.path())).unwrap(),
        ).unwrap();
        assert_eq!(paired[0]["secret"], token);
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
