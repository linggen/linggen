//! An Anthropic Messages API in front of the user's own models, so a host
//! that speaks it (Claude Code via `ling claude`) runs on Linggen's
//! credentials instead of its own. Phase 1 serves the Anthropic models
//! configured with an API key: the request goes upstream as-is, with the
//! model id swapped for the upstream one and Linggen's key attached.
//! Subscription (`claude_oauth`) models are never served here — the point is
//! to spend Linggen's quota, not to relay the host's own login.

use super::ServerState;
use crate::config::ModelConfig;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;

/// Client headers that never go upstream: auth is always Linggen's own, and
/// hop-by-hop or body-framing headers belong to this connection, not the next.
/// `accept-encoding` stays local because the body is relayed undecoded while
/// its `content-encoding` is not. Everything else (User-Agent, x-app,
/// anthropic-*) is the client's own attribution and travels as sent.
const LOCAL_HEADERS: [&str; 14] = [
    "x-api-key",
    "authorization",
    "host",
    "content-length",
    "content-type",
    "accept-encoding",
    "connection",
    "keep-alive",
    "proxy-connection",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];
const DEFAULT_VERSION: &str = "2023-06-01";

pub(crate) async fn messages_handler(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    forward(&state, &headers, body, "/v1/messages").await
}

pub(crate) async fn count_tokens_handler(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    forward(&state, &headers, body, "/v1/messages/count_tokens").await
}

/// The servable models, in the Anthropic `/v1/models` shape.
pub(crate) async fn models_handler(State(state): State<Arc<ServerState>>) -> Response {
    let configs = servable_models(&state).await;
    let data: Vec<Value> = configs
        .iter()
        .map(|c| json!({ "type": "model", "id": c.id, "display_name": c.id, "created_at": "2026-01-01T00:00:00Z" }))
        .collect();
    let first = data.first().map(|m| m["id"].clone()).unwrap_or(Value::Null);
    let last = data.last().map(|m| m["id"].clone()).unwrap_or(Value::Null);
    Json(json!({ "data": data, "has_more": false, "first_id": first, "last_id": last }))
        .into_response()
}

async fn forward(state: &ServerState, headers: &HeaderMap, body: Bytes, path: &str) -> Response {
    let Ok(mut request) = serde_json::from_slice::<Value>(&body) else {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "body is not JSON",
        );
    };
    let asked = request["model"].as_str().unwrap_or_default().to_string();
    let servable = servable_models(state).await;
    let Some(cfg) = servable.iter().find(|c| c.id == asked || c.model == asked) else {
        let ids: Vec<&str> = servable.iter().map(|c| c.id.as_str()).collect();
        let msg = format!(
            "model '{asked}' is not served by Linggen. Available: {}",
            ids.join(", ")
        );
        return error(StatusCode::NOT_FOUND, "not_found_error", &msg);
    };
    // The body goes upstream byte for byte unless the id needs swapping.
    let body = if cfg.model == asked {
        body
    } else {
        request["model"] = Value::String(cfg.model.clone());
        Bytes::from(serde_json::to_vec(&request).unwrap_or_default())
    };

    let url = format!("{}{path}", upstream_base(&cfg.url));
    let mut rb = reqwest::Client::new()
        .post(url)
        .headers(forwarded_headers(headers))
        .header("content-type", "application/json")
        .header("x-api-key", cfg.api_key.clone().unwrap_or_default())
        .body(body);
    if !headers.contains_key("anthropic-version") {
        rb = rb.header("anthropic-version", DEFAULT_VERSION);
    }

    match rb.send().await {
        Ok(upstream) => relay(upstream),
        Err(e) => {
            tracing::warn!("[anthropic-gateway] {asked}: upstream unreachable: {e}");
            error(
                StatusCode::BAD_GATEWAY,
                "api_error",
                &format!("upstream unreachable: {e}"),
            )
        }
    }
}

/// The client's headers minus auth and per-connection ones.
fn forwarded_headers(headers: &HeaderMap) -> reqwest::header::HeaderMap {
    let mut out = reqwest::header::HeaderMap::new();
    for (name, value) in headers {
        if LOCAL_HEADERS.contains(&name.as_str()) {
            continue;
        }
        let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()),
            reqwest::header::HeaderValue::from_bytes(value.as_bytes()),
        ) else {
            continue;
        };
        out.append(name, value);
    }
    out
}

/// Stream the upstream response back unchanged: status, the headers the
/// host's retry logic reads, and the body event by event.
fn relay(upstream: reqwest::Response) -> Response {
    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for (name, value) in upstream.headers() {
        if is_relayed_header(name.as_str()) {
            builder = builder.header(name.as_str(), value.as_bytes());
        }
    }
    builder
        .body(Body::from_stream(upstream.bytes_stream()))
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

fn is_relayed_header(name: &str) -> bool {
    matches!(
        name,
        "content-type" | "retry-after" | "x-should-retry" | "request-id"
    ) || name.starts_with("anthropic-ratelimit-")
}

/// Anthropic models that carry their own API key.
async fn servable_models(state: &ServerState) -> Vec<ModelConfig> {
    let models = state.manager.models.read().await.clone();
    models
        .list_models()
        .into_iter()
        .filter(|c| is_servable(c))
        .cloned()
        .collect()
}

fn is_servable(cfg: &ModelConfig) -> bool {
    cfg.provider == "anthropic"
        && cfg.auth_mode.as_deref() != Some("claude_oauth")
        && cfg.api_key.as_deref().is_some_and(|k| !k.is_empty())
}

fn upstream_base(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    if trimmed.is_empty() {
        "https://api.anthropic.com".to_string()
    } else {
        trimmed.to_string()
    }
}

/// An error in the Anthropic shape, so the host shows it as a model error.
fn error(status: StatusCode, kind: &str, message: &str) -> Response {
    let body = json!({ "type": "error", "error": { "type": kind, "message": message } });
    (status, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(provider: &str, auth: Option<&str>, key: Option<&str>) -> ModelConfig {
        let mut cfg: ModelConfig = serde_json::from_value(json!({
            "id": "m", "provider": provider, "url": "", "model": "m"
        }))
        .unwrap();
        cfg.auth_mode = auth.map(str::to_string);
        cfg.api_key = key.map(str::to_string);
        cfg
    }

    #[test]
    fn serves_only_keyed_anthropic_models() {
        assert!(is_servable(&model("anthropic", None, Some("sk"))));
        assert!(!is_servable(&model(
            "anthropic",
            Some("claude_oauth"),
            Some("sk")
        )));
        assert!(!is_servable(&model("anthropic", None, None)));
        assert!(!is_servable(&model("openai", None, Some("sk"))));
    }

    #[test]
    fn relays_retry_headers_only() {
        assert!(is_relayed_header("retry-after"));
        assert!(is_relayed_header("anthropic-ratelimit-unified-status"));
        assert!(!is_relayed_header("set-cookie"));
    }

    #[test]
    fn client_attribution_travels_but_auth_and_hop_headers_do_not() {
        let mut headers = HeaderMap::new();
        for (name, value) in [
            ("user-agent", "claude-cli/2.1.0 (external, cli)"),
            ("x-app", "cli"),
            ("anthropic-beta", "oauth-2025"),
            ("anthropic-version", "2023-06-01"),
            ("x-stainless-lang", "js"),
            ("x-api-key", "client-key"),
            ("authorization", "Bearer linggen"),
            ("host", "127.0.0.1:9527"),
            ("content-length", "42"),
            ("connection", "keep-alive"),
            ("accept-encoding", "gzip"),
        ] {
            headers.insert(name, value.parse().unwrap());
        }
        let out = forwarded_headers(&headers);
        assert_eq!(out["user-agent"], "claude-cli/2.1.0 (external, cli)");
        assert_eq!(out["x-app"], "cli");
        assert_eq!(out["anthropic-beta"], "oauth-2025");
        assert_eq!(out["x-stainless-lang"], "js");
        for gone in [
            "x-api-key",
            "authorization",
            "host",
            "content-length",
            "connection",
            "accept-encoding",
        ] {
            assert!(!out.contains_key(gone), "{gone} must not be forwarded");
        }
    }

    #[test]
    fn empty_url_goes_to_anthropic() {
        assert_eq!(upstream_base(""), "https://api.anthropic.com");
        assert_eq!(upstream_base("https://x.example/"), "https://x.example");
    }
}
