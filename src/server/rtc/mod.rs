//! WebRTC transport — WHIP signaling + str0m data channels.
//!
//! This module handles:
//! - WHIP endpoint (`POST /api/rtc/whip`) for SDP offer/answer exchange
//! - Data channel management (control channel + per-session channels)
//! - Bridging data channel messages to/from the existing event system
//!
//! str0m is Sans-IO: we drive the event loop ourselves using a UDP socket
//! in a tokio task per peer connection.

pub(crate) mod contract;
pub(crate) mod page_state;
mod peer;
pub mod proxy_client;
pub mod proxy_room;
pub mod relay;
pub mod room_config;
pub mod token_store;

use crate::engine::permission::PermissionMode;

/// What a peer is to this machine — the one fact the rest of the label derives
/// from, so `user_type` can't say one thing here and another there.
///
/// This is provenance, not permission. What a paired device may do is its
/// standing (`server::person::standing_of`): the owner's device at admin, any
/// other at the level set for it on this Mac. This enum records how the peer
/// got here: on a grant this Mac minted, rather than on an account that owns
/// this instance — worth saying out loud rather than filing under "owner".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerKind {
    /// The account that owns this instance, or a local browser on it.
    Owner,
    /// A device this Mac paired — phone, tablet, watch, second Mac. It may be
    /// signed into a different account, or none at all.
    Paired,
    /// A proxy room consumer, borrowing this machine's models.
    Consumer,
    /// Mid-pairing: has the QR secret and nothing else. See [`UserContext::pairing_only`].
    Pairing,
}

impl PeerKind {
    /// The wire label. One place, so page state, the chat API and the
    /// `user_info` greeting can never disagree about what a peer is.
    pub fn as_str(self) -> &'static str {
        match self {
            PeerKind::Owner => "owner",
            PeerKind::Paired => "paired",
            PeerKind::Consumer => "consumer",
            PeerKind::Pairing => "pairing",
        }
    }
}

/// Every WebRTC peer connection carries a UserContext — both owner and consumer.
/// Owner = UserContext { user_id: "abc", permission: Admin, ... }
/// Consumer = UserContext { user_id: "xyz", permission: Read, ... }
#[derive(Debug, Clone)]
pub struct UserContext {
    /// User ID on linggen.dev (or "__local__" for owner without remote login).
    pub user_id: String,
    /// Display name (from linggen.dev profile or relay signaling).
    pub user_name: Option<String>,
    /// Avatar URL (from linggen.dev profile or relay signaling).
    pub avatar_url: Option<String>,
    /// What this peer is to us. Set at connection time — not derived from
    /// permission level, and the source of every `user_type` on the wire.
    pub kind: PeerKind,
    /// Permission ceiling — max session permission this user can use.
    pub permission: PermissionMode,
    /// Optional daily token budget (None = unlimited).
    pub token_budget_daily: Option<i64>,
    /// Room name (only for room consumers).
    pub room_name: Option<String>,
    /// Consumer transport type: "browser" or "linggen". None for owner.
    pub consumer_type: Option<String>,
    /// How the peer's channel was opened. Decides what the tunnel tells a
    /// handler about a peer that has not named a device ([`Self::tunnel_mark`]).
    pub reached_by: Reach,
    /// The relay said this peer's account owns this Mac's instance: it dialled
    /// with that account's credential (the owner's offer), or sent it beside
    /// its device grant (`owner` on a paired offer). What `identify` verifies
    /// a phone's claim by; nothing else sets it.
    pub account_vouched: bool,
}

/// How a peer's channel was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// WHIP from this machine: the local web UI, the app shell.
    ThisMachine,
    /// WHIP from the LAN, with a paired device's token.
    Lan,
    /// An offer through the linggen.dev relay.
    Relay,
}

impl UserContext {
    /// Build for owner (local or remote).
    pub fn owner(user_id: Option<String>) -> Self {
        let account = crate::account::load_account();
        Self {
            user_id: user_id
                .or_else(|| account.as_ref().and_then(|a| a.user_id.clone()))
                .unwrap_or_else(|| "__local__".to_string()),
            user_name: account
                .as_ref()
                .and_then(|a| a.user_name.clone())
                .or_else(|| Some(crate::account::instance_name())),
            avatar_url: account.as_ref().and_then(|a| a.avatar_url.clone()),
            kind: PeerKind::Owner,
            permission: PermissionMode::Admin,
            token_budget_daily: None,
            room_name: None,
            consumer_type: None,
            reached_by: Reach::Relay,
            account_vouched: false,
        }
    }

    /// Build for a device this Mac paired, arriving on the grant it minted.
    ///
    /// The ceiling is *the owner's* — taken from [`Self::owner`] rather than
    /// written out again. It is only a ceiling: what reaches this Mac through
    /// the peer is bounded by the device's standing
    /// (`server::person::standing_of`), and what a session may do by its
    /// `(path, mode)` grant. The ceiling never opens anything by itself.
    ///
    /// Identity is what differs. This peer's real one arrives later, as a
    /// device token on the channel's `identify`, so the user id stays a
    /// placeholder and the name and face are left blank rather than borrowing
    /// the account's — the phone may be held by someone else entirely.
    pub fn paired() -> Self {
        Self {
            user_id: crate::server::person::no_one().wire(),
            user_name: None,
            avatar_url: None,
            kind: PeerKind::Paired,
            ..Self::owner(None)
        }
    }

    /// Build for a phone that reached us on the strength of the on-screen QR
    /// and nothing else. See [`UserContext::pairing_only`].
    pub fn pairing() -> Self {
        Self {
            user_id: crate::server::person::pairing_wire().to_string(),
            user_name: None,
            avatar_url: None,
            kind: PeerKind::Pairing,
            permission: PermissionMode::Chat,
            token_budget_daily: None,
            room_name: None,
            consumer_type: None,
            reached_by: Reach::Relay,
            account_vouched: false,
        }
    }

    /// Build for a proxy room consumer.
    pub fn consumer(
        user_id: String,
        consumer_type: String,
        permission: PermissionMode,
        token_budget_daily: Option<i64>,
        room_name: Option<String>,
    ) -> Self {
        Self {
            user_id,
            user_name: None,
            avatar_url: None,
            kind: PeerKind::Consumer,
            permission,
            token_budget_daily,
            room_name,
            consumer_type: Some(consumer_type),
            reached_by: Reach::Relay,
            account_vouched: false,
        }
    }

    /// User type string for the chat API.
    #[cfg(test)]
    pub fn user_type(&self) -> &'static str {
        self.kind.as_str()
    }

    /// What this peer is once it has said who it is.
    ///
    /// The handshake tells us different amounts on different routes: the relay
    /// is told a device grant was used, a LAN peer arrives on network trust and
    /// says nothing. So a phone on the LAN starts out indistinguishable from
    /// the owner's own browser, and only `identify` — the device token it
    /// already holds — settles it. Same device, same label, either route.
    ///
    /// Owner is the only kind this promotes. A consumer stays a consumer
    /// whatever it holds, and a peer mid-pairing has not earned the name.
    pub fn effective_kind(&self, identified: bool) -> PeerKind {
        match self.kind {
            PeerKind::Owner if identified => PeerKind::Paired,
            k => k,
        }
    }

    /// [`Self::user_type`], corrected by what `identify` revealed.
    pub fn user_type_for(&self, identified: bool) -> &'static str {
        self.effective_kind(identified).as_str()
    }

    /// The `TUNNEL_HEADER` value for this peer's re-issued requests, while it
    /// has not named a device: the owner only when its channel was opened on
    /// this machine, or by the owner's own account through the relay. A LAN
    /// peer is a paired device until `identify` says which one, so it is
    /// neither.
    pub fn tunnel_mark(&self) -> &'static str {
        use crate::server::api::pair::{TUNNEL_LOCAL, TUNNEL_OWNER};
        match (self.reached_by, self.kind) {
            (Reach::ThisMachine, _) => TUNNEL_LOCAL,
            (Reach::Relay, PeerKind::Owner) => TUNNEL_OWNER,
            _ => "1",
        }
    }

    /// A proxy room consumer, borrowing this machine's models.
    pub fn is_consumer(&self) -> bool {
        self.kind == PeerKind::Consumer
    }

    /// This peer has scanned the QR but is not paired yet.
    ///
    /// It matters because a tunnelled request is re-issued to loopback, and the
    /// LAN gate trusts loopback unconditionally — so a channel is, by itself,
    /// full access to this machine. A phone mid-pairing has proved only that
    /// someone stood in front of this screen, which earns exactly one call:
    /// trading the scanned secret for a device token. Everything else is
    /// refused until it comes back holding that token.
    pub fn pairing_only(&self) -> bool {
        self.kind == PeerKind::Pairing
    }
}

use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::server::ServerState;

/// Return the WHIP auth token (local UI fetches this before connecting).
pub async fn whip_token_handler(State(state): State<Arc<ServerState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "token": state.whip_token }))
}

/// WHIP endpoint: accept SDP offer, return SDP answer.
///
/// The client sends a complete SDP offer (with ICE candidates bundled).
/// We create an Rtc instance, bind a UDP socket, accept the offer,
/// and return the SDP answer. The peer connection runs in a background task.
///
/// Requires `Authorization: Bearer <whip_token>` header.
pub async fn whip_handler(
    State(state): State<Arc<ServerState>>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Verify WHIP auth token
    let auth_ok = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t == state.whip_token)
        .unwrap_or(false);
    if !auth_ok {
        return (StatusCode::UNAUTHORIZED, "Invalid or missing WHIP token").into_response();
    }

    let offer_str = match std::str::from_utf8(&body) {
        Ok(s) => s.to_string(),
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid UTF-8 in SDP offer").into_response();
        }
    };

    let (reach, device) = whip_reach(peer, &headers);
    match peer::create_peer(offer_str, state, reach, device).await {
        Ok(answer_sdp) => (
            StatusCode::CREATED,
            [(header::CONTENT_TYPE, "application/sdp")],
            answer_sdp,
        )
            .into_response(),
        Err(e) => {
            tracing::error!("WHIP error: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("WHIP error: {e}"),
            )
                .into_response()
        }
    }
}

/// How a WHIP peer reached us. This machine only when the request is what
/// "someone at this Mac" means everywhere (`is_at_this_mac`: loopback
/// socket, loopback Host, a loopback page or the app shell as Origin); any
/// other caller is a LAN peer, and the device whose token it presented is
/// the peer from the start, as `identify` would make it.
fn whip_reach(
    peer: std::net::SocketAddr,
    headers: &HeaderMap,
) -> (Reach, Option<crate::server::api::pair::Actor>) {
    if crate::server::api::ling_mem_connect::is_at_this_mac(peer, headers) {
        return (Reach::ThisMachine, None);
    }
    let device = crate::memory_host::caller::token_from_headers(headers)
        .and_then(|t| crate::server::api::pair::actor_for_token(&t));
    (Reach::Lan, device)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::routes::classes::tunnel_allows;

    /// Only this machine's own page or shell opens a `local` channel; a
    /// loopback request from another origin (an extension, a rebound name)
    /// is not "at this Mac", and a LAN peer is a device, never the owner.
    #[test]
    fn a_whip_peer_is_this_machine_only_when_someone_is_at_it() {
        let h = |pairs: &[(&'static str, &'static str)]| {
            let mut m = HeaderMap::new();
            for (k, v) in pairs {
                m.insert(*k, v.parse().unwrap());
            }
            m
        };
        let lo: std::net::SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let page = h(&[
            ("host", "127.0.0.1:9527"),
            ("origin", "http://127.0.0.1:9527"),
        ]);
        assert_eq!(whip_reach(lo, &page).0, Reach::ThisMachine);
        let shell = h(&[("host", "127.0.0.1:9527"), ("origin", "tauri://localhost")]);
        assert_eq!(whip_reach(lo, &shell).0, Reach::ThisMachine);
        let ext = h(&[
            ("host", "127.0.0.1:9527"),
            ("origin", "chrome-extension://abc"),
        ]);
        assert!(matches!(whip_reach(lo, &ext), (Reach::Lan, None)));
        let lan: std::net::SocketAddr = "192.168.1.5:5000".parse().unwrap();
        let unknown = h(&[("host", "192.168.1.2:9527"), ("x-linggen-device", "nobody")]);
        assert!(matches!(whip_reach(lan, &unknown), (Reach::Lan, None)));
    }

    /// A phone reaches us two ways and must answer to one name. Over the relay
    /// the grant says so at handshake; on the LAN nothing does, and `identify`
    /// is the only moment the Mac can tell the phone from its own browser.
    #[test]
    fn a_device_is_paired_on_either_route() {
        assert_eq!(UserContext::paired().user_type(), "paired");
        assert_eq!(
            UserContext::owner(None).user_type_for(true),
            "paired",
            "a LAN peer holding a device token is that device, not the owner"
        );
    }

    /// What a handler learns of a peer that has not named a device: the
    /// owner only for this machine's own page and the owner's relay login.
    #[test]
    fn the_tunnel_mark_says_owner_only_when_it_knows() {
        use crate::server::api::pair::{TUNNEL_LOCAL, TUNNEL_OWNER};
        let reached = |ctx: UserContext, reach| UserContext {
            reached_by: reach,
            ..ctx
        };
        assert_eq!(
            reached(UserContext::owner(None), Reach::ThisMachine).tunnel_mark(),
            TUNNEL_LOCAL
        );
        assert_eq!(UserContext::owner(None).tunnel_mark(), TUNNEL_OWNER);
        assert_eq!(
            reached(UserContext::owner(None), Reach::Lan).tunnel_mark(),
            "1"
        );
        assert_eq!(UserContext::paired().tunnel_mark(), "1");
        assert_eq!(UserContext::pairing().tunnel_mark(), "1");
    }

    #[test]
    fn an_unidentified_peer_is_still_the_owner() {
        // The web UI and the app shell never identify — they have no device
        // token and are not devices. They must keep owner.
        assert_eq!(UserContext::owner(None).user_type_for(false), "owner");
    }

    #[test]
    fn tunnel_endpoint_access_matches_the_baseline() {
        use PermissionMode::{Admin, Chat, Edit, Read};
        // Raw tunnel URL: exact matches retain the query, as do the prefixes.
        // The room gate and the named-device route seam are independent.
        let all = [true; 4];
        let admin = [false, false, false, true];
        let read = [false, true, true, true];
        let edit = [false, false, true, true];
        for (method, url, room, device) in [
            ("GET", "/index.html", all, all),
            ("GET", "/index.html?v=1", admin, all),
            ("GET", "/logo.svg", admin, all),
            ("GET", "/assets/app.js?v=1", all, all),
            ("HEAD", "/assets/app.js", admin, admin),
            ("GET", "/shared/chat-bridge.js", all, all),
            ("POST", "/shared/chat-bridge.js", admin, admin),
            ("GET", "/apps/cfo/scripts/cfo.html", all, read),
            ("POST", "/apps/cfo/capability/test", admin, admin),
            ("POST", "/api/sessions", all, all),
            ("POST", "/api/sessions?x=1", admin, all),
            ("DELETE", "/api/sessions/all", all, admin),
            ("GET", "/api/workspace/state?x=1", all, all),
            ("GET", "/api/skill-sessions/state?x=1", all, all),
            ("GET", "/api/user/language", all, all),
            ("GET", "/api/user/language?x=1", admin, all),
            ("POST", "/api/user/language", admin, admin),
            ("POST", "/api/chat", admin, all),
            ("GET", "/api/topic/latest", admin, read),
            ("POST", "/api/media/backup", admin, edit),
            ("POST", "/api/memory/search", admin, all),
            ("GET", "/api/config", admin, admin),
            ("POST", "/api/engine/restart", admin, admin),
        ] {
            for (i, level) in [Chat, Read, Edit, Admin].into_iter().enumerate() {
                assert_eq!(
                    tunnel_allows(level, method, url),
                    room[i],
                    "room gate: {method} {url} at {level}"
                );
                let standing = crate::server::person::for_test("u-device", level);
                assert_eq!(
                    crate::server::person::send_judged(
                        method,
                        url,
                        &mut serde_json::Value::Null,
                        &standing,
                        "paired",
                    )
                    .is_ok(),
                    device[i],
                    "device seam: {method} {url} at {level}"
                );
            }
        }
    }

    #[test]
    fn a_room_member_can_load_the_shared_page_helpers() {
        // A skill app opened through a room imports /shared/*.js; without it
        // the page is blank for everyone but the owner.
        let read = PermissionMode::Read;
        assert!(tunnel_allows(read, "GET", "/shared/chat-bridge.js"));
        assert!(tunnel_allows(read, "GET", "/apps/cfo/scripts/cfo.html"));
        assert!(!tunnel_allows(read, "POST", "/shared/chat-bridge.js"));
        assert!(!tunnel_allows(read, "GET", "/api/config"));
    }

    #[test]
    fn only_admin_peers_can_restart_the_engine() {
        for perm in [PermissionMode::Chat, PermissionMode::Read, PermissionMode::Edit] {
            assert!(!tunnel_allows(perm, "POST", "/api/restart/current"));
            assert!(!tunnel_allows(perm, "POST", "/api/engine/restart"));
        }
        assert!(tunnel_allows(PermissionMode::Admin, "POST", "/api/restart/current"));
    }

    #[test]
    fn any_peer_may_read_the_language_but_not_the_config() {
        for perm in [PermissionMode::Read, PermissionMode::Chat] {
            assert!(tunnel_allows(perm, "GET", "/api/user/language"));
            assert!(!tunnel_allows(perm, "POST", "/api/user/language"));
            assert!(!tunnel_allows(perm, "GET", "/api/config"));
        }
    }

    #[test]
    fn identifying_never_promotes_the_other_kinds() {
        // A consumer with a device token is still bounded by its room, and a
        // peer mid-pairing has not finished earning anything.
        let room = UserContext::consumer(
            "u1".into(),
            "linggen".into(),
            PermissionMode::Read,
            None,
            None,
        );
        assert_eq!(room.user_type_for(true), "consumer");
        assert_eq!(UserContext::pairing().user_type_for(true), "pairing");
    }

    #[test]
    fn paired_takes_the_owners_ceiling_whatever_it_is() {
        // Not "paired is admin" — paired is *the owner's*, so if the owner's
        // ceiling ever moves this follows it instead of drifting. The ceiling
        // is only a cap; what a session may do is the (path, mode) grant, and
        // an ungranted path is chat either way.
        assert_eq!(
            UserContext::paired().permission,
            UserContext::owner(None).permission
        );
        assert!(!UserContext::paired().is_consumer());
        assert!(!UserContext::paired().pairing_only());
    }

    /// A paired device wears no one else's face. The Mac's account name and
    /// avatar belong to the Mac's owner; the phone may be held by someone else,
    /// and says who on `identify`.
    #[test]
    fn paired_borrows_no_identity_from_the_account() {
        let p = UserContext::paired();
        assert_eq!(p.user_id, "__paired__");
        assert!(p.user_name.is_none());
        assert!(p.avatar_url.is_none());
    }
}
