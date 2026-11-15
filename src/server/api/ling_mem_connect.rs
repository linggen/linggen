//! A machine on the LAN asking ling-mem to share memory — shown here so the
//! Allow / Deny click can happen on whatever Linggen page is open.
//!
//! Memory owns the whole flow (its `/api/connect/*`): the pending request,
//! the token, the denied list. These routes call it in-process for the three
//! calls the modal makes, and only for someone sitting at this Mac:
//!
//! - the socket peer is loopback (a paired phone or laptop holds a token and
//!   passes the LAN gate, so a token is never enough);
//! - through the WebRTC tunnel (which re-issues a peer's calls from
//!   loopback), only a channel opened on this machine that names no device
//!   (`TUNNEL_LOCAL`, no `ACTOR_DEVICE_HEADER`) — this machine's own page;
//! - the Host names loopback and a browser Origin, if any, is a loopback page
//!   or the app shell — no extension, no other site.
//!
//! Anyone else gets 403, so the modal never renders for them: the page decides
//! nothing, the server does.

use axum::{
    extract::{ConnectInfo, Path},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::net::SocketAddr;

use crate::server::api::pair::{ACTOR_DEVICE_HEADER, TUNNEL_HEADER, TUNNEL_LOCAL};
use crate::server::loopback_guard;

/// `GET /api/ling-mem/connect/pending`
pub async fn pending(ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    if !is_at_this_mac(peer, &headers) {
        return refused();
    }
    forward(Method::GET, "pending").await
}

/// `POST /api/ling-mem/connect/{id}/{verdict}` — verdict is `allow` or `deny`.
pub async fn decide(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path((id, verdict)): Path<(String, String)>,
) -> Response {
    if !is_at_this_mac(peer, &headers) {
        return refused();
    }
    if !is_request_id(&id) || !matches!(verdict.as_str(), "allow" | "deny") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "bad request" })),
        )
            .into_response();
    }
    forward(Method::POST, &format!("{id}/{verdict}")).await
}

/// Someone at this Mac: loopback socket, loopback Host, no foreign Origin,
/// and — through the tunnel — a channel opened on this machine
/// (`TUNNEL_LOCAL`) that has not named a device. The local web UI speaks only
/// over its channel, so this is how its Allow click arrives; a phone's or a
/// relay peer's channel never carries that mark.
pub(super) fn is_at_this_mac(peer: SocketAddr, headers: &HeaderMap) -> bool {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    peer.ip().is_loopback()
        && header(TUNNEL_HEADER).is_none_or(|v| v == TUNNEL_LOCAL)
        && header(ACTOR_DEVICE_HEADER).is_none()
        && header("host").is_some_and(loopback_guard::host_is_loopback)
        && header("origin").is_none_or(is_local_page)
}

fn is_local_page(origin: &str) -> bool {
    loopback_guard::origin_is_loopback_page(origin) || origin == "tauri://localhost"
}

/// ling-mem's request ids are uuids; nothing else is forwarded into a path.
pub(super) fn is_request_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

pub(super) fn refused() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "ok": false, "error": "only at this Mac" })),
    )
        .into_response()
}

/// Memory's own connect route, called in-process as this machine. After an
/// Allow the device list is re-read at once, so the new machine's first
/// call is not refused by a cached copy.
async fn forward(method: Method, tail: &str) -> Response {
    let path = format!("/api/connect/{tail}");
    let (status, body) = crate::memory_host::local_call(method, &path, None).await;
    if status.is_success() && tail.ends_with("/allow") {
        crate::state_fs::devices::reload_devices();
    }
    (status, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    fn lo() -> SocketAddr {
        "127.0.0.1:50000".parse().unwrap()
    }

    #[test]
    fn the_mac_itself_may_decide() {
        assert!(is_at_this_mac(
            lo(),
            &headers(&[("host", "127.0.0.1:9527")])
        ));
        let page = headers(&[
            ("host", "localhost:9527"),
            ("origin", "http://localhost:9527"),
        ]);
        assert!(is_at_this_mac(lo(), &page));
        let shell = headers(&[("host", "127.0.0.1:9527"), ("origin", "tauri://localhost")]);
        assert!(is_at_this_mac(lo(), &shell));
        // This machine's own page, over its channel.
        let ui = headers(&[("host", "127.0.0.1:9527"), (TUNNEL_HEADER, TUNNEL_LOCAL)]);
        assert!(is_at_this_mac(lo(), &ui));
    }

    /// A paired phone or laptop passes the LAN gate with its token or cookie;
    /// it still may not approve anyone.
    #[test]
    fn a_paired_device_on_the_lan_may_not() {
        let lan: SocketAddr = "192.168.20.31:50000".parse().unwrap();
        let h = headers(&[
            ("host", "192.168.20.230:9527"),
            ("x-linggen-device", "cafe"),
            ("cookie", "linggen_device=cafe"),
        ]);
        assert!(!is_at_this_mac(lan, &h));
    }

    /// The WebRTC tunnel re-issues a phone's calls from loopback.
    #[test]
    fn a_tunnelled_phone_may_not() {
        let tunnelled = headers(&[("host", "127.0.0.1:9527"), (TUNNEL_HEADER, "1")]);
        assert!(!is_at_this_mac(lo(), &tunnelled));
        let relay_owner = headers(&[("host", "127.0.0.1:9527"), (TUNNEL_HEADER, "owner")]);
        assert!(!is_at_this_mac(lo(), &relay_owner));
        let local_but_a_phone = headers(&[
            ("host", "127.0.0.1:9527"),
            (TUNNEL_HEADER, TUNNEL_LOCAL),
            (ACTOR_DEVICE_HEADER, "dev"),
        ]);
        assert!(!is_at_this_mac(lo(), &local_but_a_phone));
        let acting = headers(&[("host", "127.0.0.1:9527"), (ACTOR_DEVICE_HEADER, "dev")]);
        assert!(!is_at_this_mac(lo(), &acting));
    }

    #[test]
    fn rebinding_and_cross_site_pages_may_not() {
        assert!(!is_at_this_mac(
            lo(),
            &headers(&[("host", "evil.example:9527")])
        ));
        assert!(!is_at_this_mac(lo(), &HeaderMap::new()));
        let csrf = headers(&[
            ("host", "127.0.0.1:9527"),
            ("origin", "https://evil.example"),
        ]);
        assert!(!is_at_this_mac(lo(), &csrf));
        let ext = headers(&[
            ("host", "127.0.0.1:9527"),
            ("origin", "chrome-extension://abc"),
        ]);
        assert!(!is_at_this_mac(lo(), &ext));
    }

    #[test]
    fn only_request_ids_reach_the_daemons_path() {
        assert!(is_request_id("3f2a9c1e-0b7d-4e6a-9f00-1234567890ab"));
        assert!(!is_request_id("../network"));
        assert!(!is_request_id(""));
    }
}
