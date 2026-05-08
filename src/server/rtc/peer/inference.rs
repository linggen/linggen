//! Inference and `list_models` over the inference data channel.
//!
//! Used by "linggen server proxy" mode, where a consumer's linggen uses the
//! owner's linggen as a model provider. This module streams chunks back on
//! the control response channel rather than writing directly to the DC.

use std::sync::Arc;

use crate::server::ServerState;

use super::ControlRequest;

/// Process inference and list_models requests from proxy room consumers.
///
/// Protocol:
/// - list_models: returns { request_id, data: { models: [...] } }
/// - inference: streams { request_id, chunk: { type, ... } } then { request_id, done: true }
pub(super) async fn process_inference_request(
    req: &ControlRequest,
    state: &Arc<ServerState>,
    user_ctx: &crate::server::rtc::UserContext,
    tokens_used: &Arc<std::sync::atomic::AtomicI64>,
    tx: &tokio::sync::mpsc::Sender<(Option<String>, str0m::channel::ChannelId, serde_json::Value)>,
    cid: str0m::channel::ChannelId,
) {
    use futures_util::StreamExt;

    let rid = req.request_id.clone();

    // Only allow inference from linggen-type consumers (not browser consumers)
    if user_ctx.consumer_type.as_deref() != Some("linggen") && !user_ctx.permission.is_admin() {
        let _ = tx
            .send((
                rid.clone(),
                cid,
                serde_json::json!({
                    "error": "Inference endpoint is only available for linggen server consumers"
                }),
            ))
            .await;
        return;
    }

    match req.msg_type.as_str() {
        "list_models" => {
            let room_cfg = crate::server::rtc::room_config::load_room_config();
            let models = state.manager.models.read().await;
            let model_list: Vec<serde_json::Value> = models
                .list_models()
                .iter()
                .filter(|m| {
                    // Only expose local models the owner has explicitly shared.
                    // Proxy models (from rooms this owner joined) are never re-shared.
                    m.provider != "proxy" && room_cfg.shared_models.contains(&m.id)
                })
                .map(|m| {
                    serde_json::json!({
                        "id": m.id,
                        "model": m.model,
                        "provider": m.provider,
                        "supports_tools": m.supports_tools,
                    })
                })
                .collect();
            let _ = tx
                .send((
                    rid,
                    cid,
                    serde_json::json!({
                        "data": { "models": model_list }
                    }),
                ))
                .await;
        }

        "inference" => {
            // Check token budget (persistent store: room-level + per-consumer)
            {
                let room_cfg = crate::server::rtc::room_config::load_room_config();
                let mut store = state.token_usage.lock().await;
                if !store.check_budget(
                    &user_ctx.user_id,
                    room_cfg.token_budget_room_daily,
                    room_cfg
                        .token_budget_consumer_daily
                        .or(user_ctx.token_budget_daily),
                ) {
                    let _ = tx
                        .send((
                            rid.clone(),
                            cid,
                            serde_json::json!({
                                "error": "Token budget exhausted for today"
                            }),
                        ))
                        .await;
                    return;
                }
            }

            let model_id = req.body.get("model").and_then(|v| v.as_str()).unwrap_or("");
            if model_id.is_empty() {
                let _ = tx
                    .send((
                        rid.clone(),
                        cid,
                        serde_json::json!({ "error": "model required" }),
                    ))
                    .await;
                return;
            }

            // Verify the model is in the shared list
            let room_cfg = crate::server::rtc::room_config::load_room_config();
            if !room_cfg.shared_models.contains(&model_id.to_string()) {
                let _ = tx
                    .send((
                        rid.clone(),
                        cid,
                        serde_json::json!({
                            "error": format!("Model '{model_id}' is not shared in this room")
                        }),
                    ))
                    .await;
                return;
            }

            // Parse messages
            let messages: Vec<crate::message::ChatMessage> = match req.body.get("messages") {
                Some(m) => match serde_json::from_value(m.clone()) {
                    Ok(msgs) => msgs,
                    Err(e) => {
                        let _ = tx
                            .send((
                                rid.clone(),
                                cid,
                                serde_json::json!({
                                    "error": format!("Invalid messages: {e}")
                                }),
                            ))
                            .await;
                        return;
                    }
                },
                None => {
                    let _ = tx
                        .send((
                            rid.clone(),
                            cid,
                            serde_json::json!({ "error": "messages required" }),
                        ))
                        .await;
                    return;
                }
            };

            let tools: Option<Vec<serde_json::Value>> = req
                .body
                .get("tools")
                .and_then(|v| serde_json::from_value(v.clone()).ok());

            tracing::debug!(
                "Inference request: model={model_id}, messages={}, tools={}",
                messages.len(),
                tools.as_ref().map(|t| t.len()).unwrap_or(0)
            );

            let models = state.manager.models.read().await;

            let stream_result = if let Some(tools) = tools {
                if !tools.is_empty() {
                    models.chat_tool_stream(model_id, &messages, tools).await
                } else {
                    models.chat_text_stream(model_id, &messages).await
                }
            } else {
                models.chat_text_stream(model_id, &messages).await
            };

            let mut stream = match stream_result {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx
                        .send((
                            rid.clone(),
                            cid,
                            serde_json::json!({
                                "error": format!("Model error: {e}")
                            }),
                        ))
                        .await;
                    return;
                }
            };

            // Stream chunks back
            while let Some(item) = stream.next().await {
                let chunk_json = match item {
                    Ok(crate::agent_manager::models::StreamChunk::Token(text)) => {
                        serde_json::json!({ "chunk": { "type": "token", "text": text } })
                    }
                    Ok(crate::agent_manager::models::StreamChunk::Usage(usage)) => {
                        // Track token usage for budget enforcement (in-memory + persistent)
                        if let Some(total) = usage.total_tokens {
                            tokens_used
                                .fetch_add(total as i64, std::sync::atomic::Ordering::Relaxed);
                            state
                                .token_usage
                                .lock()
                                .await
                                .record_usage(&user_ctx.user_id, total as i64);
                        }
                        serde_json::json!({ "chunk": { "type": "usage",
                            "prompt_tokens": usage.prompt_tokens,
                            "completion_tokens": usage.completion_tokens,
                            "total_tokens": usage.total_tokens,
                        }})
                    }
                    Ok(crate::agent_manager::models::StreamChunk::ToolCall(tc)) => {
                        serde_json::json!({ "chunk": { "type": "tool_call",
                            "index": tc.index,
                            "id": tc.id,
                            "name": tc.name,
                            "arguments_delta": tc.arguments_delta,
                            "thought_signature": tc.thought_signature,
                        }})
                    }
                    Err(e) => {
                        let _ = tx
                            .send((
                                rid.clone(),
                                cid,
                                serde_json::json!({
                                    "error": format!("Stream error: {e}")
                                }),
                            ))
                            .await;
                        return;
                    }
                };
                if tx.send((rid.clone(), cid, chunk_json)).await.is_err() {
                    return; // Connection closed
                }
            }

            // Signal stream end
            let _ = tx
                .send((rid.clone(), cid, serde_json::json!({ "done": true })))
                .await;
        }

        _ => {
            let _ = tx
                .send((
                    rid,
                    cid,
                    serde_json::json!({ "error": "unknown inference type" }),
                ))
                .await;
        }
    }
}
