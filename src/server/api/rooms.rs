//! Room config + proxy room connect/disconnect/status + token usage +
//! the linggen.dev rooms-API proxy.

use crate::server::ServerState;
use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::IntoResponse,
};
use std::sync::Arc;

/// GET /api/room-config — get local room config (shared models, allowed tools).
pub(crate) async fn get_room_config() -> impl IntoResponse {
    let config = crate::server::rtc::room_config::load_room_config();
    Json(serde_json::json!({
        "shared_models": config.shared_models,
        "allowed_tools": config.allowed_tools,
        "allowed_skills": config.allowed_skills,
        "room_enabled": config.room_enabled,
        "auto_connect": config.auto_connect,
    }))
}

/// POST /api/room-config — update local room config.
pub(crate) async fn update_room_config(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let mut config = crate::server::rtc::room_config::load_room_config();
    let was_enabled = config.room_enabled;

    if let Some(v) = body.get("shared_models") {
        config.shared_models = serde_json::from_value(v.clone()).unwrap_or_default();
    }
    if let Some(v) = body.get("allowed_tools") {
        config.allowed_tools = serde_json::from_value(v.clone()).unwrap_or_default();
    }
    if let Some(v) = body.get("allowed_skills") {
        config.allowed_skills = serde_json::from_value(v.clone()).unwrap_or_default();
    }
    if let Some(v) = body.get("room_enabled") {
        config.room_enabled = v.as_bool().unwrap_or(true);
    }
    if let Some(v) = body.get("auto_connect") {
        config.auto_connect = v.as_bool().unwrap_or(true);
    }
    if let Some(v) = body.get("token_budget_room_daily") {
        config.token_budget_room_daily = v.as_i64();
    }
    if let Some(v) = body.get("token_budget_consumer_daily") {
        config.token_budget_consumer_daily = v.as_i64();
    }
    if let Err(e) = crate::server::rtc::room_config::save_room_config(&config) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{e}") })),
        )
            .into_response();
    }

    // Sync room status to linggen.dev DB
    let room_enabled_changed = was_enabled != config.room_enabled;
    if room_enabled_changed {
        let status = if config.room_enabled {
            "available"
        } else {
            "disabled"
        };
        if let Some((token, _)) = crate::account::resolve_token() {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default();
            let _ = client
                .patch(format!("{}/api/rooms", crate::account::site_url()))
                .bearer_auth(&token)
                .json(&serde_json::json!({ "status": status }))
                .send()
                .await;
        }
    }

    // If room was just disabled, kick all consumer peers and disconnect proxy rooms.
    if was_enabled && !config.room_enabled {
        let _ = state
            .events_tx
            .send(crate::server::ServerEvent::RoomDisabled);
        crate::server::rtc::proxy_room::disconnect_all_proxy_rooms(state).await;
    }

    Json(serde_json::json!({ "ok": true })).into_response()
}

/// POST /api/proxy/connect — connect to a proxy room as a linggen consumer.
/// Body: { "instance_id": "...", "owner_name": "Tom" }
pub(crate) async fn connect_proxy_room_api(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let instance_id = match body.get("instance_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "instance_id required" })),
            )
                .into_response()
        }
    };
    let owner_name = body
        .get("owner_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let room_name = body
        .get("room_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    match crate::server::rtc::proxy_room::connect_proxy_room(
        state,
        &instance_id,
        owner_name,
        room_name,
    )
    .await
    {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

/// POST /api/proxy/disconnect — disconnect from proxy room(s).
/// Body: { "instance_id": "..." } for per-room, or empty/omitted for all.
pub(crate) async fn disconnect_proxy_room_api(
    State(state): State<Arc<ServerState>>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let instance_id = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            v.get("instance_id")
                .and_then(|id| id.as_str())
                .map(String::from)
        });

    match instance_id {
        Some(id) => {
            crate::server::rtc::proxy_room::disconnect_proxy_room_by_instance(state, &id).await
        }
        None => crate::server::rtc::proxy_room::disconnect_all_proxy_rooms(state).await,
    }
    Json(serde_json::json!({ "ok": true }))
}

/// GET /api/proxy/status — list active proxy room connections.
pub(crate) async fn proxy_status_api(
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    let connections = state.proxy_connections.list().await;
    Json(serde_json::json!({ "connections": connections }))
}

/// GET /api/token-usage — get current token usage from persistent store.
pub(crate) async fn token_usage_api(
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    let store = state.token_usage.lock().await;
    let room_cfg = crate::server::rtc::room_config::load_room_config();
    Json(serde_json::json!({
        "room_total": store.get_usage("").1,
        "room_budget": room_cfg.token_budget_room_daily,
        "consumer_budget": room_cfg.token_budget_consumer_daily,
    }))
}

/// Proxy linggen.dev room APIs — forwards GET/POST/PATCH/DELETE to /api/rooms/*.
/// Uses the API token from remote.toml for auth. Public endpoints
/// (`GET /api/rooms/public`, `GET /api/rooms/preview/...`) are reachable
/// without login by falling back to the default relay URL.
pub(crate) async fn proxy_rooms(
    method: axum::http::Method,
    uri: axum::http::Uri,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let room_path = uri
        .path()
        .strip_prefix("/api/rooms")
        .unwrap_or_default()
        .trim_start_matches('/');

    let is_public_endpoint = method == axum::http::Method::GET
        && (room_path == "public" || room_path.starts_with("preview/"));

    let account = crate::account::resolve_token();
    if account.is_none() && !is_public_endpoint {
        return (StatusCode::UNAUTHORIZED, "Not logged in to linggen.dev").into_response();
    }

    let relay_url = crate::account::site_url();

    // linggen.dev routes match `/api/rooms` exactly for room create/update/delete.
    // Normalize both local `/api/rooms` and `/api/rooms/` to the same upstream URL.
    let url = if room_path.is_empty() {
        format!("{}/api/rooms", relay_url)
    } else {
        format!("{}/api/rooms/{}", relay_url, room_path)
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default();

    let mut req = match method {
        axum::http::Method::GET => client.get(&url),
        axum::http::Method::POST => client.post(&url),
        axum::http::Method::PATCH => client.patch(&url),
        axum::http::Method::DELETE => client.delete(&url),
        _ => return (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
    };

    if let Some((token, _)) = &account {
        req = req.bearer_auth(token);
    }
    if !body.is_empty() {
        // Auto-inject instance_id for room creation/update if not already present.
        if let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(&body) {
            if json.get("instance_id").is_none() || json["instance_id"].is_null() {
                if let Ok(id) = crate::account::instance_id() {
                    json["instance_id"] = serde_json::Value::String(id);
                }
            }
            req = req
                .header("Content-Type", "application/json")
                .json(&json);
        } else {
            req = req
                .header("Content-Type", "application/json")
                .body(body.to_vec());
        }
    }

    match req.send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16())
                .unwrap_or(StatusCode::BAD_GATEWAY);
            let body_text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                tracing::warn!(
                    "rooms proxy {} {} → {} body={}",
                    method,
                    url,
                    status.as_u16(),
                    body_text.chars().take(300).collect::<String>()
                );
            }
            match serde_json::from_str::<serde_json::Value>(&body_text) {
                Ok(body) => (status, Json(body)).into_response(),
                Err(_) => (status, body_text).into_response(),
            }
        }
        Err(e) => {
            tracing::warn!("rooms proxy {} {} reqwest error: {}", method, url, e);
            (StatusCode::BAD_GATEWAY, format!("Relay error: {}", e)).into_response()
        }
    }
}
