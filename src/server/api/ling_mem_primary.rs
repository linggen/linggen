//! Settings → Memory → Primary: this machine's ling-mem is the primary, or a
//! replica of another machine's. ling-mem owns the setting (`/api/sync/*`);
//! the engine only forwards to its LOCAL daemon, which it always talks to on
//! loopback — the primary's address never reaches a client.
//!
//! - `GET /api/ling-mem/primary` — role, upstream, online, last sync, queued.
//! - `PUT /api/ling-mem/primary {primary}` — someone at this Mac only.

use axum::{
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::server::api::ling_mem_connect::{is_at_this_mac, refused};
use crate::server::state::ServerState;

/// The local daemon: the configured address when it is on this machine,
/// else ling-mem's own default port.
async fn local_base(state: &ServerState) -> String {
    let url = state.manager.get_config_snapshot().await.agent.ling_mem_url;
    let host = url::Url::parse(&url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string));
    match host.as_deref() {
        Some("127.0.0.1" | "localhost" | "[::1]") => url.trim_end_matches('/').to_string(),
        _ => format!("http://127.0.0.1:{}", crate::config::DEFAULT_LING_MEM_PORT),
    }
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default()
}

async fn relay(req: reqwest::RequestBuilder) -> Response {
    match req.send().await {
        Ok(r) => {
            let status =
                StatusCode::from_u16(r.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            (status, Json(r.json::<Value>().await.unwrap_or(Value::Null))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "ok": false, "error": format!("ling-mem not running here: {e}") })),
        )
            .into_response(),
    }
}

/// `GET /api/ling-mem/primary`
pub async fn get_primary(State(state): State<Arc<ServerState>>) -> Response {
    let base = local_base(&state).await;
    relay(http().get(format!("{base}/api/sync/role"))).await
}

/// `PUT /api/ling-mem/primary`
pub async fn put_primary(
    State(state): State<Arc<ServerState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !is_at_this_mac(peer, &headers) {
        return refused();
    }
    let primary = body["primary"].as_str().unwrap_or("local").to_string();
    let base = local_base(&state).await;
    relay(
        http()
            .put(format!("{base}/api/sync/primary"))
            .json(&json!({ "primary": primary })),
    )
    .await
}
