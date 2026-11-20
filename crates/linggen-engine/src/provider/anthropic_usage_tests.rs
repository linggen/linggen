//! What an Anthropic reply reports it used, read off the wire: a fake host
//! answers the Messages API with a scripted usage block.
use crate::config::ModelConfig;
use crate::provider::models::{ModelManager, StreamChunk, TokenUsage};
use axum::{
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use futures_util::StreamExt;

fn sse(events: &[(&str, serde_json::Value)]) -> String {
    events
        .iter()
        .map(|(name, data)| format!("event: {name}\ndata: {data}\n\n"))
        .collect()
}

async fn manager_answering(body: String) -> ModelManager {
    let app = Router::new()
        .route("/v1/models/{id}", get(|| async { StatusCode::NOT_FOUND }))
        .route(
            "/v1/messages",
            post(move || {
                let body = body.clone();
                async move { ([("content-type", "text/event-stream")], body).into_response() }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let config: ModelConfig = serde_json::from_value(serde_json::json!({
        "id": "m", "provider": "anthropic", "url": url, "model": "claude-sonnet-5-5",
        "api_key": "k", "supports_tools": true,
    }))
    .unwrap();
    ModelManager::new_with_credentials(vec![config], &crate::credentials::Credentials::default())
}

/// The usage one scripted reply reported, its two frames merged as the
/// engine merges them.
async fn reported(events: &[(&str, serde_json::Value)]) -> Option<TokenUsage> {
    let m = manager_answering(sse(events)).await;
    let messages = vec![crate::message::ChatMessage::new("user", "hi".to_string())];
    let mut stream = m
        .chat_text_stream("m", &messages, None, None)
        .await
        .unwrap();
    let mut usage: Option<TokenUsage> = None;
    while let Some(chunk) = stream.next().await {
        if let StreamChunk::Usage(u) = chunk.unwrap() {
            usage = Some(match usage.take() {
                Some(earlier) => earlier.merged(u),
                None => u,
            });
        }
    }
    usage
}

#[tokio::test]
async fn a_reply_reports_input_output_cache_read_and_cache_write() {
    let usage = reported(&[
        (
            "message_start",
            serde_json::json!({"type": "message_start", "message": {"usage": {
            "input_tokens": 1_200, "cache_creation_input_tokens": 3_000,
            "cache_read_input_tokens": 5_000, "output_tokens": 1}}}),
        ),
        (
            "message_delta",
            serde_json::json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"},
            "usage": {"output_tokens": 400}}),
        ),
        ("message_stop", serde_json::json!({"type": "message_stop"})),
    ])
    .await
    .expect("usage");
    // The prompt is everything sent: fresh + written + read.
    assert_eq!(usage.prompt_tokens, Some(9_200));
    assert_eq!(usage.cached_tokens, Some(5_000));
    assert_eq!(usage.cache_write_tokens, Some(3_000));
    assert_eq!(usage.completion_tokens, Some(400));
    let tokens = crate::provider::models::Tokens::of(&usage).unwrap();
    assert_eq!(
        (
            tokens.input,
            tokens.cache_write,
            tokens.cache_read,
            tokens.output
        ),
        (1_200, 3_000, 5_000, 400)
    );
}

/// A reply that reports no cache fields (an older gateway) writes none.
#[tokio::test]
async fn a_reply_without_cache_fields_has_no_cache_tokens() {
    let usage = reported(&[
        ("message_start", serde_json::json!({"type": "message_start", "message": {"usage": {"input_tokens": 70}}})),
        ("message_delta", serde_json::json!({"type": "message_delta", "delta": {}, "usage": {"output_tokens": 9}})),
    ]).await.expect("usage");
    let tokens = crate::provider::models::Tokens::of(&usage).unwrap();
    assert_eq!(
        (
            tokens.input,
            tokens.cache_write,
            tokens.cache_read,
            tokens.output
        ),
        (70, 0, 0, 9)
    );
}
