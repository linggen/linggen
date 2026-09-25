//! The user's other surfaces: pairing, WebRTC signaling, topics, skill
//! sync, the browser bridge, MCP, and the companion's voice and events.

use super::Routes;
use crate::server::{api, bridge, mcp, rtc};
use axum::routing::{get, patch, post};

pub(super) fn routes() -> Routes {
    Routes::new()
        // Companion
        .route("/api/tts", post(api::tts::tts_handler))
        .route("/api/yinyue/chat", post(api::yinyue::chat_handler))
        .route("/api/yinyue/event", post(api::yinyue::event_handler))
        .route("/api/presence", post(api::yinyue::presence_handler))
        // Browser bridge and the MCP front door
        .route("/api/bridge/socket", get(bridge::socket_handler))
        .route("/api/bridge/call", post(bridge::call_handler))
        .route("/api/bridge/status", get(bridge::status_handler))
        .route("/mcp", post(mcp::post_handler).get(mcp::get_handler))
        // WebRTC signaling
        .route("/api/rtc/whip", post(rtc::whip_handler))
        .route("/api/rtc/token", get(rtc::whip_token_handler))
        // Pairing
        .route("/api/pair/request", post(api::pair::post_pair_request))
        .route("/api/pair/confirm", post(api::pair::post_pair_confirm))
        .route(
            "/api/pair/qr-confirm",
            post(api::pair::post_pair_qr_confirm),
        )
        .route(
            "/api/pair/window/keepalive",
            post(api::pair::post_pair_window_keepalive),
        )
        .route("/api/pair/info", get(api::pair::get_pair_info))
        .route("/api/pair/me", get(api::pair::get_pair_me))
        .route("/api/pair/qr", get(api::pair::get_pair_qr))
        .route(
            "/api/pair/devices/{id}",
            patch(api::pair::rename_pair_device).delete(api::pair::delete_pair_device),
        )
        .route("/pair", get(api::pair::get_pair_page))
        // Topics and the activity log
        .route("/api/topic/publish", post(api::topic::publish))
        .route("/api/topic/latest", get(api::topic::latest))
        .route(
            "/api/activity",
            post(api::activity::record).get(api::activity::recent),
        )
        // Skill-declared sync
        .route(
            "/api/skill-sync/{skill}/items",
            get(api::skill_sync::get_items),
        )
        .route(
            "/api/skill-sync/{skill}/file",
            get(api::skill_sync::get_file),
        )
        .route(
            "/api/skill-sync/{skill}/devices",
            get(api::skill_sync::get_devices),
        )
        .route(
            "/api/skill-sync/{skill}/have",
            post(api::skill_sync::post_have),
        )
}
