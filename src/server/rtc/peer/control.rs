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
    actor: &super::PeerActor,
    peer_id: u64,
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

        // A peer says who is holding it. The device token is the same secret it
        // already presents on LAN calls; resolving it here means identity is
        // bound once per connection and works the same over relay, where no
        // token rides the handshake at all.
        "identify" => {
            let token = msg
                .get("device_token")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let account = msg
                .get("account")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            let resolved = crate::server::api::pair::set_device_account(token, account);
            if resolved.is_none() && !token.is_empty() {
                tracing::warn!("[rtc] identify with an unknown device token — staying anonymous");
            }
            let was_identified = actor.lock().unwrap().is_some();
            let now_identified = resolved.is_some();
            // This is the moment the Mac learns a device is on the other end,
            // on the LAN as over the relay — so it is where perception learns
            // it too. Idempotent, and silent for peers that are not devices.
            if let Some(a) = &resolved {
                crate::perception::devices::arrived(&a.device, peer_id);
            }
            *actor.lock().unwrap() = resolved;

            // On the relay the Mac already knew this was a paired device; on
            // the LAN it could not, and greeted the phone as the owner. Now it
            // knows, so it says so — the peer is told rather than left holding
            // a label the rest of this connection contradicts.
            if now_identified != was_identified
                && user_ctx.effective_kind(now_identified)
                    != user_ctx.effective_kind(was_identified)
            {
                if let Some(mut ch) = rtc.channel(channel_id) {
                    let msg = super::user_info_msg(user_ctx, now_identified);
                    let _ = ch.write(false, msg.to_string().as_bytes());
                }
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

        // Yinyue presenter lock (FCFS singleton). A surface that renders her
        // subscribes on mount; the server grants the lock to the first
        // subscriber and tells the others (via `yinyue_present`) to stay blank.
        "yinyue_subscribe" => {
            state.yinyue_subscribe(peer_id);
            None
        }
        "yinyue_release" => {
            state.yinyue_release(peer_id);
            None
        }

        // Device topics: one surface publishes, every other surface of this
        // user receives (the daemon is the hub). Payload is opaque — topics
        // are a contract between the surfaces, not the engine.
        "topic_publish" => {
            let topic = msg.get("topic").and_then(|v| v.as_str()).unwrap_or("");
            let op = msg.get("op").and_then(|v| v.as_str()).unwrap_or("");
            if topic.is_empty() || op.is_empty() {
                return None;
            }
            tracing::info!("[topic] {topic}/{op} published by a device");
            // A phone publishing what it is — rather than that something
            // changed — asks for the value to be kept, because the surfaces
            // that want it (skill pages) have no peer to receive it live.
            if msg.get("retain").and_then(|v| v.as_bool()).unwrap_or(false) {
                let payload = msg
                    .get("payload")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                crate::server::api::topic::retain(topic, op, &payload);
            }
            let _ = state
                .events_tx
                .send(crate::server::ServerEvent::DeviceTopic {
                    topic: topic.to_string(),
                    op: op.to_string(),
                    payload: msg
                        .get("payload")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                    from_device: msg
                        .get("from_device")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                });
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

/// Wrap a loopback response for the control channel: a success body rides as
/// `data`, a failure comes back as `error` so the client's promise rejects.
///
/// Wrapping every body as `data` regardless of status is what made a refused
/// plan action look like a success — the UI resolved, showed nothing, and the
/// user was left clicking a button that did nothing.
fn wrap_endpoint_response(status: u16, body: serde_json::Value) -> serde_json::Value {
    if (200..300).contains(&status) {
        return serde_json::json!({ "data": body });
    }
    let reason = body
        .get("error")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("HTTP {status}"));
    serde_json::json!({ "error": reason })
}

/// Process a pending control channel request asynchronously.
/// Runs in a spawned task — returns the result to be sent on the data channel
/// via the ctrl_resp channel (avoiding blocking str0m's event loop).
pub(super) async fn process_control_request_async(
    req: &ControlRequest,
    state: &Arc<ServerState>,
    client: &reqwest::Client,
    user_ctx: &crate::server::rtc::UserContext,
    actor: &super::PeerActor,
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
            // Carry the peer's identity into the loopback request. The phone
            // speaks only WebRTC, so without this a handler cannot tell which
            // paired device is asking — which is what per-phone state, like the
            // delete queue, needs to be correct with more than one phone.
            let actor_device = actor.lock().unwrap().as_ref().map(|a| a.device.clone());
            let tag = |b: reqwest::RequestBuilder| match &actor_device {
                Some(d) => b.header(crate::server::api::pair::ACTOR_DEVICE_HEADER, d),
                None => b,
            };
            let resp = match method {
                "POST" => tag(client.post(&url).json(&body_val)).send().await,
                "PUT" => tag(client.put(&url).json(&body_val)).send().await,
                "PATCH" => tag(client.patch(&url).json(&body_val)).send().await,
                "DELETE" => tag(client.delete(&url).json(&body_val)).send().await,
                _ => tag(client.get(&url)).send().await,
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
            let identified = actor.lock().unwrap().is_some();
            body["user_type"] =
                serde_json::Value::String(user_ctx.user_type_for(identified).to_string());
            body["user_id"] = serde_json::Value::String(user_ctx.user_id.clone());
            let url = format!("http://127.0.0.1:{port}{endpoint}");
            match client.post(&url).json(&body).send().await {
                Ok(r) => {
                    let status = r.status().as_u16();
                    let body: serde_json::Value = r.json().await.unwrap_or(serde_json::Value::Null);
                    if !(200..300).contains(&status) {
                        tracing::warn!("RTC {} → {endpoint} failed: {status}", req.msg_type);
                    }
                    wrap_endpoint_response(status, body)
                }
                Err(e) => serde_json::json!({ "error": format!("{e}") }),
            }
        }

        _ => serde_json::json!({ "error": "unknown type" }),
    }
}

#[cfg(test)]
mod tests {
    use super::wrap_endpoint_response;
    use serde_json::json;

    #[test]
    fn success_body_rides_as_data() {
        let out = wrap_endpoint_response(200, json!({ "status": "rejected" }));
        assert_eq!(out, json!({ "data": { "status": "rejected" } }));
    }

    #[test]
    fn failure_surfaces_the_server_reason() {
        let out = wrap_endpoint_response(404, json!({ "error": "No pending plan" }));
        assert_eq!(out, json!({ "error": "No pending plan" }));
    }

    #[test]
    fn failure_without_a_reason_falls_back_to_the_status() {
        let out = wrap_endpoint_response(500, json!(null));
        assert_eq!(out, json!({ "error": "HTTP 500" }));
    }
}
