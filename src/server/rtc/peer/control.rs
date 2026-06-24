//! Control-channel message handling.
//!
//! The control data channel carries three categories of message:
//! - **Synchronous replies** (`heartbeat`, `set_view_context`, `room_chat`) —
//!   handled inline inside the str0m event loop.
//! - **RPC requests** (`http_request`, `chat`, `plan_*`, `ask_user_response`,
//!   `inference`, `list_models`) — returned as a pending `ControlRequest` so
//!   the main loop can run them off-loop and deliver the response via the
//!   `ctrl_resp` mpsc channel.

use std::collections::HashMap;
use std::sync::Arc;

use str0m::Rtc;

use crate::server::ServerState;

use super::ControlRequest;

/// Handle a message on the control data channel.
/// Returns an optional async request to process outside the str0m loop.
pub(super) fn handle_control_message(
    rtc: &mut Rtc,
    channel_id: str0m::channel::ChannelId,
    text: &str,
    state: &Arc<ServerState>,
    _session_channels: &mut HashMap<String, str0m::channel::ChannelId>,
    _channel_sessions: &mut HashMap<str0m::channel::ChannelId, String>,
    view_ctx: &mut crate::server::rtc::page_state::ViewContext,
    force_page_state: &mut bool,
    user_ctx: &crate::server::rtc::UserContext,
) -> Option<ControlRequest> {
    let msg: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Control message parse error: {e}");
            return None;
        }
    };

    let msg_type = msg
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let request_id = msg
        .get("request_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    match msg_type.as_str() {
        "heartbeat" => {
            if let Some(mut ch) = rtc.channel(channel_id) {
                let resp = serde_json::json!({ "type": "heartbeat", "ts": chrono::Utc::now().timestamp_millis() });
                let _ = ch.write(false, resp.to_string().as_bytes());
            }
            None
        }

        "http_request" | "chat" | "clear" | "compact" | "plan_approve" | "plan_reject"
        | "plan_edit" | "ask_user_response" | "inference" | "list_models" => {
            // These need async processing — return as pending request
            Some(ControlRequest {
                request_id,
                channel_id,
                msg_type,
                body: msg,
            })
        }

        "set_view_context" => {
            view_ctx.session_id = msg
                .get("session_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            view_ctx.project_root = msg
                .get("project_root")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            view_ctx.view = msg
                .get("view")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            // Remember the focused session so agent_chat can deliver into the
            // chat the user actually has open.
            if let Some(sid) = &view_ctx.session_id {
                *state.current_view.lock().unwrap() = Some((
                    sid.clone(),
                    view_ctx.project_root.clone().unwrap_or_default(),
                ));
            }
            *force_page_state = true;
            tracing::debug!(
                "View context updated: view={:?} session={:?} project={:?}",
                view_ctx.view,
                view_ctx.session_id,
                view_ctx.project_root
            );
            None
        }

        "room_chat" => {
            let text = msg.get("text").and_then(|v| v.as_str()).unwrap_or("");
            if text.is_empty() || text.len() > 2000 {
                return None;
            }
            let text = text.to_string();
            // Prefer server-side user_name (trusted), fall back to client-provided sender_name
            let sender_name: String = user_ctx
                .user_name
                .as_deref()
                .or_else(|| msg.get("sender_name").and_then(|v| v.as_str()))
                .unwrap_or(&user_ctx.user_id)
                .chars()
                .take(64)
                .collect();
            tracing::info!(
                "[room_chat] inbound on control channel from user_id={} text_len={}",
                user_ctx.user_id,
                text.len()
            );
            let _ = state.events_tx.send(crate::server::ServerEvent::RoomChat {
                sender_id: user_ctx.user_id.clone(),
                sender_name,
                avatar_url: user_ctx.avatar_url.clone(),
                text,
            });
            None
        }

        _ => {
            tracing::debug!("Unknown control message type: {msg_type}");
            if let Some(rid) = request_id {
                if let Some(mut ch) = rtc.channel(channel_id) {
                    let resp = serde_json::json!({
                        "request_id": rid,
                        "error": format!("Unknown message type: {msg_type}")
                    });
                    let _ = ch.write(false, resp.to_string().as_bytes());
                }
            }
            None
        }
    }
}

/// Process a pending control channel request asynchronously.
/// Runs in a spawned task — returns the result to be sent on the data channel
/// via the ctrl_resp channel (avoiding blocking str0m's event loop).
pub(super) async fn process_control_request_async(
    req: &ControlRequest,
    state: &Arc<ServerState>,
    client: &reqwest::Client,
    user_ctx: &crate::server::rtc::UserContext,
    tokens_used: &Arc<std::sync::atomic::AtomicI64>,
) -> serde_json::Value {
    let port = state.port;

    // Check token budget before chat calls
    if let Some(budget) = user_ctx.token_budget_daily {
        if tokens_used.load(std::sync::atomic::Ordering::Relaxed) >= budget
            && matches!(
                req.msg_type.as_str(),
                "chat" | "plan_approve" | "plan_reject" | "plan_edit"
            )
        {
            return serde_json::json!({
                "error": "Token budget exhausted for today. Please try again tomorrow or ask the proxy owner to increase the budget."
            });
        }
    }

    match req.msg_type.as_str() {
        "http_request" => {
            let method = req
                .body
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("GET");
            let url_path_raw = req.body.get("url").and_then(|v| v.as_str()).unwrap_or("/");
            // SECURITY: percent-decode then validate to prevent SSRF bypass via %2e%2e etc.
            let url_path = urlencoding::decode(url_path_raw)
                .unwrap_or(std::borrow::Cow::Borrowed(url_path_raw));
            let path_ok = url_path.starts_with("/api/")
                || url_path.starts_with("/assets/")
                || url_path.starts_with("/apps/")
                || url_path == "/index.html"
                || url_path == "/logo.svg";
            if !path_ok
                || url_path.contains('@')
                || url_path.contains("://")
                || url_path.contains("..")
            {
                return serde_json::json!({ "error": "Invalid URL path" });
            }
            let url = format!("http://127.0.0.1:{port}{url_path}");
            // Per-request chatter → debug (fires on every status poll, session save,
            // etc. and drowns out lifecycle events at info level).
            tracing::debug!("RTC http_request: {method} {url_path}");
            let mut body_val = req
                .body
                .get("body")
                .unwrap_or(&serde_json::Value::Null)
                .clone();
            // Inject user_id into POST /api/sessions for session ownership tracking
            if url_path == "/api/sessions" && method == "POST" {
                body_val["user_id"] = serde_json::Value::String(user_ctx.user_id.clone());
            }
            let resp = match method {
                "POST" => client.post(&url).json(&body_val).send().await,
                "PUT" => client.put(&url).json(&body_val).send().await,
                "PATCH" => client.patch(&url).json(&body_val).send().await,
                "DELETE" => client.delete(&url).json(&body_val).send().await,
                _ => client.get(&url).send().await,
            };
            match resp {
                Ok(r) => {
                    let status = r.status().as_u16();
                    let body = r.text().await.unwrap_or_default();
                    serde_json::json!({ "data": { "status": status, "body": body } })
                }
                Err(e) => serde_json::json!({ "error": format!("{e}") }),
            }
        }

        "chat" | "clear" | "compact" | "plan_approve" | "plan_reject" | "plan_edit"
        | "ask_user_response" => {
            let endpoint = match req.msg_type.as_str() {
                "chat" => "/api/chat",
                "clear" => "/api/chat/clear",
                "compact" => "/api/chat/compact",
                "plan_approve" => "/api/plan/approve",
                "plan_reject" => "/api/plan/reject",
                "plan_edit" => "/api/plan/edit",
                "ask_user_response" => "/api/ask-user-response",
                _ => unreachable!(),
            };
            // Inject user_type and user_id into the request body
            let mut body = req.body.clone();
            body["user_type"] = serde_json::Value::String(user_ctx.user_type().to_string());
            body["user_id"] = serde_json::Value::String(user_ctx.user_id.clone());
            let url = format!("http://127.0.0.1:{port}{endpoint}");
            match client.post(&url).json(&body).send().await {
                Ok(r) => {
                    let body: serde_json::Value = r.json().await.unwrap_or(serde_json::Value::Null);
                    serde_json::json!({ "data": body })
                }
                Err(e) => serde_json::json!({ "error": format!("{e}") }),
            }
        }

        _ => serde_json::json!({ "error": "unknown type" }),
    }
}
