//! The user's other surfaces: pairing, WebRTC signaling, topics, skill
//! sync, the browser bridge, MCP, and the companion's voice and events.

use super::Routes;
use crate::server::{anthropic_gateway, api, bridge, mcp, rtc};
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
        // Anthropic Messages API over the user's models (`ling claude`)
        .route(
            "/anthropic/v1/messages",
            post(anthropic_gateway::messages_handler),
        )
        .route(
            "/anthropic/v1/messages/count_tokens",
            post(anthropic_gateway::count_tokens_handler),
        )
        .route(
            "/anthropic/v1/models",
            get(anthropic_gateway::models_handler),
        )
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
        // A LAN machine asking ling-mem to share memory: Allow / Deny here
        .route(
            "/api/ling-mem/connect/pending",
            get(api::ling_mem_connect::pending),
        )
        .route(
            "/api/ling-mem/connect/{id}/{verdict}",
            post(api::ling_mem_connect::decide),
        )
        // Primary or replica — the local ling-mem's own setting
        .route(
            "/api/ling-mem/primary",
            get(api::ling_mem_primary::get_primary).put(api::ling_mem_primary::put_primary),
        )
        // Where this machine's memory lives (client.json)
        .route(
            "/api/ling-mem/client",
            get(api::ling_mem_client::get_client).put(api::ling_mem_client::put_client),
        )
        .route(
            "/api/ling-mem/client/request-access",
            post(api::ling_mem_client::request_access),
        )
        .route(
            "/api/ling-mem/client/access/{id}",
            get(api::ling_mem_client::access_status),
        )
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
