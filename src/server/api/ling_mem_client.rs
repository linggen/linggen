//! Settings → Memory → "ling-mem address": where this machine's memory lives,
//! read from and written to `~/.linggen/client.json` (`state_fs::client`) —
//! the same file `/linggen:config` writes and the plugin hooks read.
//!
//! - `GET  /api/ling-mem/client` — the address, whether a token is held, and
//!   the connection state as a fact: `ok` | `not_approved` | `no_answer`.
//! - `PUT  /api/ling-mem/client {ling_mem}` — change the address.
//! - `POST /api/ling-mem/client/request-access` — ask that machine to connect
//!   (ling-mem's `/api/connect/request`), as `/linggen:config` does.
//! - `GET  /api/ling-mem/client/access/{id}` — follow the request; on Allow
//!   the token is saved to client.json and never shown to the page.
//!
//! Writes and access requests are for someone at this Mac only — the same
//! rule as `ling_mem_connect`.

use axum::{
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::server::api::ling_mem_connect::{is_at_this_mac, refused};
use crate::server::state::ServerState;
use crate::state_fs::client;

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .build()
        .unwrap_or_default()
}

fn reply(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

fn bad(msg: &str) -> Response {
    reply(
        StatusCode::BAD_REQUEST,
        json!({ "ok": false, "error": msg }),
    )
}

/// Can this machine use that ling-mem? Asked of `/mcp`, the gated route real
/// calls take — `/api/health` answers even when this machine is not approved.
async fn probe(url: &str) -> &'static str {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
    let req = client::with_token(http().post(format!("{url}/mcp")).json(&body));
    match req.send().await {
        Ok(r) if r.status().is_success() => "ok",
        Ok(r) if matches!(r.status().as_u16(), 401 | 403) => "not_approved",
        Ok(_) => "error",
        Err(_) => "no_answer",
    }
}

fn is_loopback_url(url: &str) -> bool {
    match url::Url::parse(url)
        .ok()
        .and_then(|u| u.host().map(|h| h.to_owned()))
    {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

async fn view(url: &str) -> Value {
    json!({
        "ling_mem": url,
        "local": is_loopback_url(url),
        "has_token": client::token().is_some(),
        "state": probe(url).await,
    })
}

async fn current_url(state: &ServerState) -> String {
    state.manager.get_config_snapshot().await.agent.ling_mem_url
}

/// `GET /api/ling-mem/client`
pub async fn get_client(State(state): State<Arc<ServerState>>) -> Response {
    let url = current_url(&state).await;
    reply(
        StatusCode::OK,
        json!({ "ok": true, "data": view(&url).await }),
    )
}

#[derive(Deserialize)]
pub struct SetClient {
    ling_mem: String,
}

/// `host:port` is how a person says an address; accept it.
fn normalize(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_end_matches('/');
    let with_scheme = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("http://{raw}")
    };
    let u = url::Url::parse(&with_scheme).ok()?;
    let plain = matches!(u.scheme(), "http" | "https")
        && u.host().is_some()
        && u.path() == "/"
        && u.query().is_none();
    plain.then(|| with_scheme)
}

/// `PUT /api/ling-mem/client`
pub async fn put_client(
    State(state): State<Arc<ServerState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<SetClient>,
) -> Response {
    if !is_at_this_mac(peer, &headers) {
        return refused();
    }
    let Some(url) = normalize(&body.ling_mem) else {
        return bad("an http address like 192.168.1.5:9528");
    };
    let saved = client::update(|o| {
        o.insert("ling_mem".into(), Value::String(url.clone()));
    });
    if let Err(e) = saved {
        return reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "ok": false, "error": format!("could not write client.json: {e}") }),
        );
    }
    state.manager.set_ling_mem_url(url.clone()).await;
    reconnect_memory(&state).await;
    reply(
        StatusCode::OK,
        json!({ "ok": true, "data": view(&url).await }),
    )
}

/// Re-dial the built-in memory MCP server at the current address and token.
async fn reconnect_memory(state: &ServerState) {
    let cfg = state.manager.get_config_snapshot().await;
    let servers = crate::mcp_client::with_builtin(&cfg.mcp_servers, &cfg.agent.ling_mem_url);
    tokio::spawn(async move { crate::mcp_client::registry().connect_all(&servers).await });
}

fn host_name() -> String {
    let out = std::process::Command::new("hostname").arg("-s").output();
    out.ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Linggen".to_string())
}

/// `POST /api/ling-mem/client/request-access`
pub async fn request_access(
    State(state): State<Arc<ServerState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if !is_at_this_mac(peer, &headers) {
        return refused();
    }
    let url = current_url(&state).await;
    if is_loopback_url(&url) {
        return bad("this machine's own ling-mem needs no access");
    }
    let device_id = match client::device_id() {
        Ok(id) => id,
        Err(e) => return bad(&format!("could not write client.json: {e}")),
    };
    let body = json!({ "device_name": host_name(), "device_id": device_id });
    forward(
        http()
            .post(format!("{url}/api/connect/request"))
            .json(&body),
    )
    .await
}

/// `GET /api/ling-mem/client/access/{id}` — the token stays on this machine.
pub async fn access_status(
    State(state): State<Arc<ServerState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if !is_at_this_mac(peer, &headers) {
        return refused();
    }
    if !crate::server::api::ling_mem_connect::is_request_id(&id) {
        return bad("bad request id");
    }
    let url = current_url(&state).await;
    let Ok(r) = http()
        .get(format!("{url}/api/connect/status/{id}"))
        .send()
        .await
    else {
        return reply(
            StatusCode::OK,
            json!({ "ok": true, "data": { "state": "no_answer" } }),
        );
    };
    let body: Value = r.json().await.unwrap_or(Value::Null);
    let st = body["data"]["state"]
        .as_str()
        .unwrap_or("expired")
        .to_string();
    if let Some(token) = body["data"]["device_token"].as_str() {
        if let Err(e) = save_token(token) {
            return bad(&format!("approved, but could not save the token: {e}"));
        }
        reconnect_memory(&state).await;
    }
    reply(
        StatusCode::OK,
        json!({ "ok": true, "data": { "state": st } }),
    )
}

fn save_token(token: &str) -> std::io::Result<()> {
    client::update(|o| {
        o.insert("token".into(), Value::String(token.to_string()));
    })
}

async fn forward(req: reqwest::RequestBuilder) -> Response {
    match req.send().await {
        Ok(r) => {
            let status =
                StatusCode::from_u16(r.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            reply(status, r.json().await.unwrap_or(Value::Null))
        }
        Err(e) => reply(
            StatusCode::BAD_GATEWAY,
            json!({ "ok": false, "error": format!("no answer: {e}") }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_normalized_or_refused() {
        assert_eq!(
            normalize("192.168.1.5:9528").as_deref(),
            Some("http://192.168.1.5:9528")
        );
        assert_eq!(
            normalize("http://127.0.0.1:9528/").as_deref(),
            Some("http://127.0.0.1:9528")
        );
        assert!(normalize("http://x:9528/api").is_none());
        assert!(normalize("ftp://x:1").is_none());
        assert!(normalize("").is_none());
    }

    #[test]
    fn loopback_addresses_are_this_machine() {
        assert!(is_loopback_url("http://127.0.0.1:9528"));
        assert!(is_loopback_url("http://localhost:9528"));
        assert!(!is_loopback_url("http://192.168.20.230:9528"));
    }
}
