//! WebRTC transport — WHIP signaling + str0m data channels.
//!
//! This module handles:
//! - WHIP endpoint (`POST /api/rtc/whip`) for SDP offer/answer exchange
//! - Data channel management (control channel + per-session channels)
//! - Bridging data channel messages to/from the existing event system
//!
//! str0m is Sans-IO: we drive the event loop ourselves using a UDP socket
//! in a tokio task per peer connection.

mod peer;
pub(crate) mod page_state;
pub mod proxy_client;
pub mod proxy_room;
pub mod relay;
pub mod token_store;
pub mod room_config;

/// User-level permission ceiling — the maximum level this user can operate at.
/// Session permission (read/edit/admin) is per-session and changeable by the user,
/// but always capped by UserPermission.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum UserPermission {
    Chat,   // Conversation only — no tools
    Read,   // Browse + search (WebSearch, WebFetch)
    Edit,   // Full coding tools (Read, Write, Edit, Bash, etc.)
    Admin,  // Full access — settings, config, everything
}

impl UserPermission {
    pub fn is_admin(&self) -> bool { matches!(self, UserPermission::Admin) }

    /// Check if this permission level allows accessing an HTTP endpoint.
    /// Admin can access everything. Others are restricted to static assets + session creation.
    pub fn can_access_endpoint(&self, method: &str, url: &str) -> bool {
        if self.is_admin() { return true; }
        // Static assets (tunnel loading) and skill app files
        if (url == "/index.html" || url.starts_with("/assets/") || url.starts_with("/apps/")) && method == "GET" {
            return true;
        }
        // Session creation + deletion (consumers can manage their own sessions)
        if url == "/api/sessions" && method == "POST" { return true; }
        if url == "/api/sessions/all" && method == "DELETE" { return true; }
        // Workspace state — chat history on page load / refresh
        if method == "GET" && (url.starts_with("/api/workspace/state") || url.starts_with("/api/skill-sessions/state")) {
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
/// This is provenance, not permission. [`PeerKind::Paired`] and
/// [`PeerKind::Owner`] both hold Admin: a phone that scanned this Mac's QR is
/// the admin (2026-08-04), and the relay already lets it in on the strength of
/// the grant this Mac minted. The distinction is that the phone reached us on
/// its device grant rather than on an account that owns this instance — which
/// is worth saying out loud rather than filing under "owner".
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
            user_id: user_id.or_else(|| account.as_ref().and_then(|a| a.user_id.clone()))
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
    /// Admin, like the owner: pairing is physical-presence proof and a paired
    /// phone is the admin. What differs is the label, and that this peer's real
    /// identity arrives later as a device token on the channel's `identify` —
    /// so the user id stays a placeholder rather than borrowing the account's.
    pub fn paired() -> Self {
        Self {
            user_id: "__paired__".to_string(),
            user_name: None,
            avatar_url: None,
            kind: PeerKind::Paired,
            permission: UserPermission::Admin,
            token_budget_daily: None,
            room_name: None,
            consumer_type: None,
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
pub async fn whip_token_handler(
    State(state): State<Arc<ServerState>>,
) -> Json<serde_json::Value> {
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
            (StatusCode::INTERNAL_SERVER_ERROR, format!("WHIP error: {e}")).into_response()
        }
    }
}
