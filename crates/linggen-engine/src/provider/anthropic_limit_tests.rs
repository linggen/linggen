//! The output limit an Anthropic request carries, read off the wire: a fake
//! host answers the Models API (`GET /v1/models/{id}`) and the Messages API,
//! and keeps what it was sent.
use crate::config::ModelConfig;
use crate::provider::models::{EffortAsk, ModelManager};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Host {
    /// Model id → (status, body) the Models API answers; none: 404.
    models: Arc<Mutex<HashMap<String, (u16, serde_json::Value)>>>,
    lookups: Arc<Mutex<Vec<String>>>,
    sent: Arc<Mutex<Vec<serde_json::Value>>>,
    /// Refuse the first Messages request as over the window.
    overflow_first: Arc<Mutex<Option<String>>>,
}

impl Host {
    fn describes(&self, id: &str, status: u16, body: serde_json::Value) {
        self.models.lock().unwrap().insert(id.into(), (status, body));
    }

    fn max_tokens_sent(&self) -> Vec<u64> {
        self.sent.lock().unwrap().iter().map(|r| r["max_tokens"].as_u64().unwrap()).collect()
    }
}

async fn model(State(host): State<Host>, Path(id): Path<String>) -> Response {
    host.lookups.lock().unwrap().push(id.clone());
    match host.models.lock().unwrap().get(&id) {
        Some((status, body)) => (StatusCode::from_u16(*status).unwrap(), Json(body.clone())).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"type": "error"}))).into_response(),
    }
}

async fn messages(State(host): State<Host>, Json(body): Json<serde_json::Value>) -> Response {
    host.sent.lock().unwrap().push(body);
    if let Some(refusal) = host.overflow_first.lock().unwrap().take() {
        return (StatusCode::BAD_REQUEST, refusal).into_response();
    }
    let done = "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    ([("content-type", "text/event-stream")], done).into_response()
}

async fn manager_on(host: &Host, id: &str, max_output_tokens: Option<u32>) -> ModelManager {
    let app = Router::new()
        .route("/v1/models/{id}", get(model))
        .route("/v1/messages", post(messages))
        .with_state(host.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let config = ModelConfig {
        id: id.into(),
        provider: "anthropic".into(),
        url,
        model: id.into(),
        api_key: Some("k".into()),
        keep_alive: None,
        context_window: None,
        tags: vec![],
        supports_tools: Some(true),
        auth_mode: None,
        reasoning_effort: None,
        max_output_tokens,
        provided_by: None,
        is_builtin: false,
        family_binding: None,
            price: None,
    };
    ModelManager::new_with_credentials(vec![config], &crate::credentials::Credentials::default())
}

async fn ask_with_tools(m: &ModelManager, id: &str) -> anyhow::Result<()> {
    let tool = serde_json::json!({"type":"function","function":{"name":"Write","description":"w","parameters":{"type":"object"}}});
    let messages = vec![crate::message::ChatMessage::new("user", "hi".to_string())];
    let mut stream = m.chat_tool_stream(id, &messages, vec![tool], EffortAsk::default(), None).await?;
    while futures_util::StreamExt::next(&mut stream).await.is_some() {}
    Ok(())
}

async fn ask_text(m: &ModelManager, id: &str) -> anyhow::Result<()> {
    let messages = vec![crate::message::ChatMessage::new("user", "hi".to_string())];
    let mut stream = m.chat_text_stream(id, &messages, EffortAsk::default(), None).await?;
    while futures_util::StreamExt::next(&mut stream).await.is_some() {}
    Ok(())
}

fn described(cap: u64) -> serde_json::Value {
    serde_json::json!({"id": "m", "max_input_tokens": 1_000_000, "max_tokens": cap})
}

/// A model the Models API describes gets its working limit — the cap when
/// that is lower — and the lookup is made once.
#[tokio::test]
async fn a_described_model_is_sent_its_working_limit_and_asked_about_once() {
    let host = Host::default();
    host.describes("big-model", 200, described(128_000));
    let m = manager_on(&host, "big-model", None).await;

    ask_with_tools(&m, "big-model").await.unwrap();
    ask_with_tools(&m, "big-model").await.unwrap();

    assert_eq!(host.max_tokens_sent(), [64_000, 64_000]);
    assert_eq!(host.lookups.lock().unwrap().len(), 1, "learned once per model");

    let host = Host::default();
    host.describes("small-model", 200, described(32_000));
    let m = manager_on(&host, "small-model", None).await;
    ask_with_tools(&m, "small-model").await.unwrap();
    assert_eq!(host.max_tokens_sent(), [32_000], "never more than the model's cap");
}

/// A text-only request (a summary, a companion's one line) keeps the
/// conservative number.
#[tokio::test]
async fn a_text_only_request_keeps_the_conservative_limit() {
    let host = Host::default();
    host.describes("big-model", 200, described(128_000));
    let m = manager_on(&host, "big-model", None).await;

    ask_text(&m, "big-model").await.unwrap();

    assert_eq!(host.max_tokens_sent(), [8_192]);
}

/// A model the host cannot describe falls back to the number used before,
/// and is not asked about again; one whose lookup fails is asked again.
#[tokio::test]
async fn an_undescribed_model_falls_back_and_a_failed_lookup_is_retried() {
    let host = Host::default();
    let m = manager_on(&host, "gateway-model", None).await;
    ask_with_tools(&m, "gateway-model").await.unwrap();
    ask_with_tools(&m, "gateway-model").await.unwrap();
    assert_eq!(host.max_tokens_sent(), [8_192, 8_192]);
    assert_eq!(host.lookups.lock().unwrap().len(), 1, "a 404 is a final answer");

    let host = Host::default();
    host.describes("flaky", 503, serde_json::json!({}));
    let m = manager_on(&host, "flaky", None).await;
    ask_with_tools(&m, "flaky").await.unwrap();
    assert_eq!(host.max_tokens_sent(), [8_192], "no answer yet: the fallback");
    host.describes("flaky", 200, described(128_000));
    ask_with_tools(&m, "flaky").await.unwrap();
    assert_eq!(host.max_tokens_sent(), [8_192, 64_000], "asked again, and learned");
}

/// A config value overrides what the API says, but never goes over the cap.
#[tokio::test]
async fn a_configured_limit_overrides_and_stops_at_the_cap() {
    let host = Host::default();
    host.describes("big-model", 200, described(128_000));
    let m = manager_on(&host, "big-model", Some(20_000)).await;
    ask_with_tools(&m, "big-model").await.unwrap();
    ask_text(&m, "big-model").await.unwrap();
    assert_eq!(host.max_tokens_sent(), [20_000, 20_000]);

    let host = Host::default();
    host.describes("big-model", 200, described(128_000));
    let m = manager_on(&host, "big-model", Some(500_000)).await;
    ask_with_tools(&m, "big-model").await.unwrap();
    assert_eq!(host.max_tokens_sent(), [128_000]);

    let host = Host::default();
    let m = manager_on(&host, "gateway-model", Some(30_000)).await;
    ask_with_tools(&m, "gateway-model").await.unwrap();
    assert_eq!(host.max_tokens_sent(), [30_000], "unknown cap: the value as set");
}

/// A context filled nearly to the window leaves less than the limit: the
/// refusal names the input it counted, and the request goes again with the
/// room that is left.
#[tokio::test]
async fn a_request_over_the_window_goes_again_with_the_room_left() {
    let host = Host::default();
    host.describes("big-model", 200, described(128_000));
    *host.overflow_first.lock().unwrap() = Some(r#"{"type":"error","error":{"type":"invalid_request_error","message":"input length and `max_tokens` exceed context limit: 190000 + 64000 > 200000, decrease input length or `max_tokens` and try again"}}"#.into());
    let m = manager_on(&host, "big-model", None).await;

    ask_with_tools(&m, "big-model").await.unwrap();

    assert_eq!(host.max_tokens_sent(), [64_000, 10_000]);
}

/// A refusal that leaves no room is the caller's, as before.
#[tokio::test]
async fn a_request_with_no_room_left_keeps_its_refusal() {
    let host = Host::default();
    host.describes("big-model", 200, described(128_000));
    *host.overflow_first.lock().unwrap() = Some(r#"{"type":"error","error":{"type":"invalid_request_error","message":"input length and `max_tokens` exceed context limit: 199900 + 64000 > 200000, decrease input length"}}"#.into());
    let m = manager_on(&host, "big-model", None).await;

    let err = ask_with_tools(&m, "big-model").await.unwrap_err();

    assert_eq!(host.max_tokens_sent(), [64_000]);
    assert_eq!(
        crate::provider::error::kind_of(&err),
        crate::provider::error::ProviderErrorKind::ContextTooLong
    );
}
