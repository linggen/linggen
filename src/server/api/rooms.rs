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
    if let Err(e) = apply_update(&mut config, &body) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response();
    }
    if let Err(e) = crate::server::rtc::room_config::save_room_config(&config) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("{e}") })),
        )
            .into_response();
    }
    sync_room_status(&state, was_enabled, config.room_enabled).await;
    Json(serde_json::json!({ "ok": true })).into_response()
}

/// The fields `body` names, onto `config` — all of them, or none when one is
/// malformed. Nothing defaults to "on".
fn apply_update(
    config: &mut crate::server::rtc::room_config::RoomConfig,
    body: &serde_json::Value,
) -> Result<(), String> {
    let Some(body) = body.as_object() else {
        return Err("the room config must be an object".into());
    };
    let mut next = config.clone();
    if let Some(v) = body.get("shared_models") {
        next.shared_models = names(v, "shared_models")?;
    }
    if let Some(v) = body.get("allowed_tools") {
        next.allowed_tools = names(v, "allowed_tools")?;
    }
    if let Some(v) = body.get("allowed_skills") {
        next.allowed_skills = names(v, "allowed_skills")?;
    }
    if let Some(v) = body.get("room_enabled") {
        next.room_enabled = flag(v, "room_enabled")?;
    }
    if let Some(v) = body.get("auto_connect") {
        next.auto_connect = flag(v, "auto_connect")?;
    }
    if let Some(v) = body.get("token_budget_room_daily") {
        next.token_budget_room_daily = budget(v, "token_budget_room_daily")?;
    }
    if let Some(v) = body.get("token_budget_consumer_daily") {
        next.token_budget_consumer_daily = budget(v, "token_budget_consumer_daily")?;
    }
    *config = next;
    Ok(())
}

fn names(v: &serde_json::Value, field: &str) -> Result<Vec<String>, String> {
    serde_json::from_value(v.clone()).map_err(|_| format!("{field} must be a list of names"))
}

fn flag(v: &serde_json::Value, field: &str) -> Result<bool, String> {
    v.as_bool()
        .ok_or_else(|| format!("{field} must be true or false"))
}

/// A whole number of tokens, or `null` for no budget.
fn budget(v: &serde_json::Value, field: &str) -> Result<Option<i64>, String> {
    if v.is_null() {
        return Ok(None);
    }
    v.as_i64()
        .map(Some)
        .ok_or_else(|| format!("{field} must be a whole number or null"))
}

/// Tell linggen.dev a switched room's status; a room just switched off
/// sends its consumers away.
async fn sync_room_status(state: &Arc<ServerState>, was_enabled: bool, room_enabled: bool) {
    // Sync room status to linggen.dev DB
    let room_enabled_changed = was_enabled != room_enabled;
    if room_enabled_changed {
        let status = if room_enabled {
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
    if was_enabled && !room_enabled {
        let _ = state
            .events_tx
            .send(crate::server::ServerEvent::RoomDisabled);
        crate::server::rtc::proxy_room::disconnect_all_proxy_rooms(state.clone()).await;
    }
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
pub(crate) async fn proxy_status_api(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let connections = state.proxy_connections.list().await;
    Json(serde_json::json!({ "connections": connections }))
}

/// GET /api/token-usage — get current token usage from persistent store.
pub(crate) async fn token_usage_api(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
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
            req = req.header("Content-Type", "application/json").json(&json);
        } else {
            req = req
                .header("Content-Type", "application/json")
                .body(body.to_vec());
        }
    }

    match req.send().await {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::rtc::room_config::RoomConfig;
    use serde_json::json;

    #[test]
    fn a_malformed_room_update_is_refused_and_changes_nothing() {
        for body in [
            json!({"shared_models": "gpt"}),
            json!({"allowed_tools": [1, 2]}),
            json!({"allowed_skills": {"a": 1}}),
            json!({"room_enabled": "yes"}),
            json!({"auto_connect": 1}),
            json!({"token_budget_room_daily": "lots"}),
            json!({"token_budget_consumer_daily": 1.5}),
            json!(["room_enabled", true]),
        ] {
            let mut config = RoomConfig::default();
            config.room_enabled = false;
            config.shared_models = vec!["m".into()];
            let before = serde_json::to_value(&config).unwrap();
            assert!(
                apply_update(&mut config, &body).is_err(),
                "{body} was taken"
            );
            assert_eq!(serde_json::to_value(&config).unwrap(), before, "{body}");
        }
    }

    #[test]
    fn a_well_formed_room_update_is_taken() {
        let mut config = RoomConfig::default();
        let body = json!({
            "shared_models": ["a"], "allowed_tools": ["Read"], "allowed_skills": [],
            "room_enabled": false, "auto_connect": false,
            "token_budget_room_daily": 100, "token_budget_consumer_daily": null,
        });
        apply_update(&mut config, &body).unwrap();
        assert_eq!(config.shared_models, vec!["a".to_string()]);
        assert_eq!(config.allowed_tools, vec!["Read".to_string()]);
        assert!(!config.room_enabled && !config.auto_connect);
        assert_eq!(config.token_budget_room_daily, Some(100));
        assert_eq!(config.token_budget_consumer_daily, None);
    }
}
