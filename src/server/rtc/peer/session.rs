//! Session-channel message proxy.
//!
//! Session data channels carry the same JSON message types the local HTTP API
//! already handles (`chat`, `plan_approve`, etc.). Rather than duplicating that
//! logic, we translate each message to an HTTP request against `127.0.0.1:{port}`
//! and let the existing handler run.

use std::sync::Arc;

use crate::server::ServerState;

/// Handle a message on a session data channel (chat, plan actions, etc.).
///
/// Proxies to the local HTTP API to reuse existing handler logic.
/// This avoids duplicating complex chat/plan/ask-user code.
pub(super) async fn handle_session_message(
    text: &str,
    session_id: &str,
    state: &Arc<ServerState>,
    client: &reqwest::Client,
    user_ctx: &crate::server::rtc::UserContext,
    // Whether this peer has identified itself with a device token — a LAN
    // phone is only distinguishable from the owner's browser after that.
    identified: bool,
) {
    let msg: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Session message parse error for {session_id}: {e}");
            return;
        }
    };

    let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let port = state.port;

    // Map data channel message types to local API endpoints
    let (endpoint, body) = match msg_type {
        "chat" => {
            let mut body = msg.clone();
            body["session_id"] = serde_json::Value::String(session_id.to_string());
            // Inject user type and user_id
            body["user_type"] =
                serde_json::Value::String(user_ctx.user_type_for(identified).to_string());
            body["user_id"] = serde_json::Value::String(user_ctx.user_id.clone());
            ("/api/chat", body)
        }
        "ask_user_response" => ("/api/ask-user-response", msg.clone()),
        "plan_approve" => {
            let mut body = msg.clone();
            body["session_id"] = serde_json::Value::String(session_id.to_string());
            ("/api/plan/approve", body)
        }
        "plan_reject" => {
            let mut body = msg.clone();
            body["session_id"] = serde_json::Value::String(session_id.to_string());
            ("/api/plan/reject", body)
        }
        "plan_edit" => {
            let mut body = msg.clone();
            body["session_id"] = serde_json::Value::String(session_id.to_string());
            ("/api/plan/edit", body)
        }
        "clear" => {
            let mut body = msg.clone();
            body["session_id"] = serde_json::Value::String(session_id.to_string());
            ("/api/chat/clear", body)
        }
        _ => {
            tracing::debug!("Unknown session message type: {msg_type}");
            return;
        }
    };

    // Proxy to local HTTP API
    let url = format!("http://127.0.0.1:{port}{endpoint}");
    tracing::debug!("RTC session proxy: {msg_type} → {endpoint}");
    match client.post(&url).json(&body).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                let status = resp.status();
                let resp_body = resp.text().await.unwrap_or_default();
                tracing::warn!("RTC proxy to {endpoint} failed: {status} body={resp_body}");
            }
        }
        Err(e) => {
            tracing::warn!("RTC proxy to {endpoint} error: {e}");
        }
    }
}
