//! WebRTC transport — WHIP signaling + str0m data channels.
//!
//! This module handles:
//! - WHIP endpoint (`POST /api/rtc/whip`) for SDP offer/answer exchange
//! - Data channel management (control channel + per-session channels)
//! - Bridging data channel messages to/from the existing event system
//!
//! str0m is Sans-IO: we drive the event loop ourselves using a UDP socket
//! in a tokio task per peer connection.

pub(crate) mod page_state;
mod peer;
pub mod proxy_client;
pub mod proxy_room;
pub mod relay;
pub mod room_config;
pub mod token_store;

/// User-level permission ceiling — the maximum level this user can operate at.
/// Session permission (read/edit/admin) is per-session and changeable by the user,
/// but always capped by UserPermission.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum UserPermission {
    Chat,  // Conversation only — no tools
    Read,  // Browse + search (WebSearch, WebFetch)
    Edit,  // Full coding tools (Read, Write, Edit, Bash, etc.)
    Admin, // Full access — settings, config, everything
}

impl UserPermission {
    pub fn is_admin(&self) -> bool {
        matches!(self, UserPermission::Admin)
    }

    /// Check if this permission level allows accessing an HTTP endpoint.
    /// Admin can access everything. Others are restricted to static assets + session creation.
    pub fn can_access_endpoint(&self, method: &str, url: &str) -> bool {
        if self.is_admin() {
            return true;
        }
        // Static assets (tunnel loading) and skill app files
        if (url == "/index.html" || url.starts_with("/assets/") || url.starts_with("/apps/"))
            && method == "GET"
        {
            return true;
        }
        // Session creation + deletion (consumers can manage their own sessions)
        if url == "/api/sessions" && method == "POST" {
            return true;
        }
        if url == "/api/sessions/all" && method == "DELETE" {
            return true;
        }
        // Workspace state — chat history on page load / refresh
        if method == "GET"
            && (url.starts_with("/api/workspace/state")
                || url.starts_with("/api/skill-sessions/state"))
        {
            return true;
        }
        false
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            UserPermission::Admin => "admin",
            UserPermission::Edit => "edit",
            UserPermission::Read => "read",
            UserPermission::Chat => "chat",
        }
    }
}

/// What a peer is to this machine — the one fact the rest of the label derives
/// from, so `user_type` can't say one thing here and another there.
///
/// This is provenance, not permission. A paired device carries the owner's
/// ceiling, and what any peer may actually do is the `(path, mode)` grant on
/// its session — an ungranted path is `chat`, for owner and phone alike. So
/// this enum decides no access today. It records how the peer got here: on a
/// grant this Mac minted, rather than on an account that owns this instance —
/// worth saying out loud rather than filing under "owner", and the seam if a
/// device ever needs a ceiling of its own.
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
    pub permission: UserPermission,
    /// Optional daily token budget (None = unlimited).
    pub token_budget_daily: Option<i64>,
    /// Room name (only for room consumers).
    pub room_name: Option<String>,
    /// Consumer transport type: "browser" or "linggen". None for owner.
    pub consumer_type: Option<String>,
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
            permission: UserPermission::Admin,
            token_budget_daily: None,
            room_name: None,
            consumer_type: None,
        }
    }

    /// Build for a device this Mac paired, arriving on the grant it minted.
    ///
    /// The ceiling is *the owner's* — taken from [`Self::owner`] rather than
    /// written out again, so it stays the owner's if that ever moves. This is
    /// only a ceiling: what a session may actually do is the `(path, mode)`
    /// grant, and a path nobody granted is `chat` — no tools. The ceiling caps
    /// that lookup; it never opens anything by itself.
    ///
    /// Identity is what differs. This peer's real one arrives later, as a
    /// device token on the channel's `identify`, so the user id stays a
    /// placeholder and the name and face are left blank rather than borrowing
    /// the account's — the phone may be held by someone else entirely.
    pub fn paired() -> Self {
        Self {
            user_id: "__paired__".to_string(),
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
            user_id: "__pairing__".to_string(),
            user_name: None,
            avatar_url: None,
            kind: PeerKind::Pairing,
            permission: UserPermission::Chat,
            token_budget_daily: None,
            room_name: None,
            consumer_type: None,
        }
    }

    /// Build for a proxy room consumer.
    pub fn consumer(
        user_id: String,
        consumer_type: String,
        permission: UserPermission,
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
        }
    }

    /// User type string for the chat API.
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

    match peer::create_peer(offer_str, state).await {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn an_unidentified_peer_is_still_the_owner() {
        // The web UI and the app shell never identify — they have no device
        // token and are not devices. They must keep owner.
        assert_eq!(UserContext::owner(None).user_type_for(false), "owner");
    }

    #[test]
    fn identifying_never_promotes_the_other_kinds() {
        // A consumer with a device token is still bounded by its room, and a
        // peer mid-pairing has not finished earning anything.
        let room = UserContext::consumer(
            "u1".into(),
            "linggen".into(),
            UserPermission::Read,
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
