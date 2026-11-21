//! Control-channel message handling.
//!
//! The control data channel carries three categories of message:
//! - **Synchronous replies** (`heartbeat`, `set_view_context`, `room_chat`,
//!   `fact`) — handled inline inside the str0m event loop.
//! - **RPC requests** (`http_request`, `chat`, `plan_*`, `ask_user_response`,
//!   `inference`, `list_models`) — returned as a pending `ControlRequest` so
//!   the main loop can run them off-loop and deliver the response via the
//!   `ctrl_resp` mpsc channel.

use crate::util::LockExt;
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
            let account: Option<crate::server::api::pair::AccountRef> = msg
                .get("account")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            // The relay vouched for this channel's account: a claim of this
            // Mac's own account is verified by it (`set_device_account`).
            let vouched_for = user_ctx
                .account_vouched
                .then(|| crate::server::person::mac_account())
                .flatten();
            let contract = crate::server::rtc::contract::told(&msg);
            let identified = crate::server::api::pair::set_device_account(
                token,
                account.clone(),
                vouched_for.as_deref(),
                contract,
            );
            // A phone that wrote memory while signed out did so under its
            // device id; now that it is somebody — or now that it is verified
            // as the owner's — those rows become theirs. Here, because this
            // is the only place a sign-in or a verification is seen.
            if let Some(id) = identified.as_ref() {
                let gained = id.signed_in.clone().or_else(|| {
                    id.verified_now.as_ref().and(account.clone())
                });
                if let Some(gained) = gained {
                    let device = id.actor.device.clone();
                    tokio::spawn(async move {
                        crate::server::api::memory::restamp_device(&device, &gained).await;
                    });
                }
            }
            let resolved = identified.map(|i| i.actor);
            // A stage it holds is now its person's.
            if let Some(a) = &resolved {
                state.yinyue_identify(peer_id, person_for(user_ctx, Some(&a.device)));
            }
            if resolved.is_none() && !token.is_empty() {
                tracing::warn!("[rtc] identify with an unknown device token — staying anonymous");
            }
            let now_identified = resolved.is_some();
            // This is the moment the Mac learns a device is on the other end,
            // on the LAN as over the relay — so it is where perception learns
            // it too. Idempotent, and silent for peers that are not devices.
            if let Some(a) = &resolved {
                crate::perception::devices::arrived(&a.device, peer_id);
            }
            *actor.lock_ok() = resolved;

            // Every identify is answered with the greeting: what the peer is
            // now (on the LAN the Mac greeted a phone as the owner until it
            // knew), and the level its device stands at.
            let device = actor.lock_ok().as_ref().map(|a| a.device.clone());
            let standing = standing_for(user_ctx, device.as_deref());
            let unverified = device
                .as_deref()
                .is_some_and(crate::server::person::claims_the_owners_unverified);
            if let Some(mut ch) = rtc.channel(channel_id) {
                let msg = super::user_info_msg(user_ctx, now_identified, (&standing, unverified));
                let _ = ch.write(false, msg.to_string().as_bytes());
            }
            None
        }

        "http_request" | "chat" | "clear" | "compact" | "plan_approve" | "plan_reject"
        | "plan_edit" | "ask_user_response" | "inference" | "list_models" => {
            // These need async processing — return as pending request.
            // A clear names the app whose page asks: in the day's session
            // that app's part alone is cleared.
            let mut body = msg;
            if let (Some(skill), "clear") = (view_ctx.skill.as_deref(), msg_type.as_str()) {
                body["skill_name"] = serde_json::Value::String(skill.to_string());
            }
            Some(ControlRequest {
                request_id,
                channel_id,
                msg_type,
                body,
            })
        }

        // A paired device saying a kind of thing happened, at a time. The
        // device is this channel's bound identity, never the payload.
        "fact" => {
            // With the daily session on, another person's facts stamp
            // nothing of the builder's; below read, a peer's facts are not
            // taken at all.
            if !shares_builders(state, user_ctx, actor) || !reads(user_ctx, actor) {
                return None;
            }
            let device = actor.lock_ok().as_ref().map(|a| a.device.clone());
            crate::server::facts::heard(state, device, &msg);
            None
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
            view_ctx.skill = msg
                .get("skill")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            // Remember the focused session so agent_chat can deliver into the
            // chat the user actually has open — the owner's chat: a peer
            // below the owner's standing never moves where it lands.
            let device = actor.lock_ok().as_ref().map(|a| a.device.clone());
            let owners = standing_for(user_ctx, device.as_deref()).is_owner();
            if let Some(sid) = view_ctx.session_id.as_ref().filter(|_| owners) {
                *state.current_view.lock_ok() = Some((
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

        // Yinyue presenter lock — one device, one voice. A surface that renders
        // her subscribes on mount, saying whether it has a `stage` (it stands
        // her in a place); a stage outranks a pet corner, otherwise the first
        // subscriber holds. The others are told (via `yinyue_present`) to stay
        // blank.
        "yinyue_subscribe" => {
            let stage = msg.get("stage").and_then(|v| v.as_bool()).unwrap_or(false);
            // A stage on an app page names the page's chat session: what the
            // user says to her while it holds her goes to that chat — one its
            // person may enter. A stage is held per person: the surface is
            // its device's person's.
            let device = actor.lock_ok().as_ref().map(|a| a.device.clone());
            let standing = standing_for(user_ctx, device.as_deref());
            // With the daily session off every surface holds her for the
            // owner: one below the owner's standing holds none.
            if !state.manager.daily_session() && !standing.is_owner() {
                return None;
            }
            let person = standing.person().clone();
            let session = msg
                .get("session")
                .and_then(|v| v.as_str())
                .filter(|sid| crate::server::day_session::may_enter(sid, &person))
                .map(str::to_string);
            state.yinyue_subscribe(peer_id, stage, session, person);
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
            // With the daily session on, a topic is the builder's devices':
            // another person's neither replaces a kept reading nor reaches
            // the builder's surfaces. Below read, it is not taken.
            if !shares_builders(state, user_ctx, actor) || !reads(user_ctx, actor) {
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

/// The standing a peer's request is judged by, resolved from the device it
/// named (`person::standing_of`). A room consumer is its own account
/// (`person::consumer`). A peer with no device named is its
/// channel's user at admin when the tunnel knows it as the owner (this
/// machine's page, the owner's account on the relay), else no one at chat
/// (`person::unnamed`) — never the owner by default.
pub(super) fn standing_for(
    user_ctx: &crate::server::rtc::UserContext,
    device: Option<&str>,
) -> crate::server::person::Standing {
    use crate::server::api::pair::{TUNNEL_LOCAL, TUNNEL_OWNER};
    if user_ctx.is_consumer() {
        return crate::server::person::consumer(&user_ctx.user_id);
    }
    let Some(device) = device else {
        let owner = matches!(user_ctx.tunnel_mark(), TUNNEL_LOCAL | TUNNEL_OWNER);
        return crate::server::person::unnamed(owner);
    };
    crate::server::person::standing_of(device)
}

/// The person a peer's request is for ([`standing_for`]).
pub(super) fn person_for(user_ctx: &crate::server::rtc::UserContext, device: Option<&str>) -> crate::server::person::Person {
    standing_for(user_ctx, device).person().clone()
}

/// Whether this peer is given what is the builder's alone
/// (`day_session::shares_builders`): any peer with the daily session off;
/// with it on, a peer whose person is the builder.
fn shares_builders(
    state: &Arc<ServerState>,
    user_ctx: &crate::server::rtc::UserContext,
    actor: &super::PeerActor,
) -> bool {
    if !state.manager.daily_session() {
        return true;
    }
    let device = actor.lock_ok().as_ref().map(|a| a.device.clone());
    crate::server::day_session::is_builder(&person_for(user_ctx, device.as_deref()))
}

/// Whether this peer stands at read or above, as the frames that publish
/// (a fact, a topic) need; a room consumer by its room.
fn reads(user_ctx: &crate::server::rtc::UserContext, actor: &super::PeerActor) -> bool {
    if user_ctx.is_consumer() {
        return true;
    }
    let device = actor.lock_ok().as_ref().map(|a| a.device.clone());
    standing_for(user_ctx, device.as_deref()).level() >= crate::engine::permission::PermissionMode::Read
}

/// The `http_request` answer for a request refused at the seam
/// (`person::send_judged`): a 403, as the LAN gate answers it.
fn refused_reply(refused: &crate::server::person::Refused) -> serde_json::Value {
    let body = refused.body().to_string();
    serde_json::json!({ "data": reply_data(403, "application/json", body.as_bytes()) })
}

/// A media channel `get` of `url` by this peer: the URL to fetch and the
/// level to stamp on it, judged and sent as its person as the same GET on
/// the control channel is; `None` when refused. A peer mid-pairing gets
/// nothing; a room consumer is judged by its room.
pub(super) fn media_get(
    user_ctx: &crate::server::rtc::UserContext,
    device: Option<&str>,
    url: &str,
) -> Option<(String, crate::engine::permission::PermissionMode)> {
    use crate::server::routes::classes::tunnel_allows;

    if user_ctx.pairing_only() || !tunnel_allows(user_ctx.permission, "GET", url) {
        return None;
    }
    let mut none = serde_json::Value::Null;
    let user_type = user_ctx.user_type_for(device.is_some());
    send_for_peer(user_ctx, device, ("GET", url, &mut none), user_type).ok()
}

/// Returns the URL to send and the level to stamp on it.
pub(super) fn send_for_peer(
    user_ctx: &crate::server::rtc::UserContext,
    device: Option<&str>,
    (method, url_path, body): (&str, &str, &mut serde_json::Value),
    user_type: &str,
) -> Result<(String, crate::engine::permission::PermissionMode), crate::server::person::Refused> {
    let standing = standing_for(user_ctx, device);
    let level = standing.level();
    if user_ctx.is_consumer() {
        return crate::server::person::send_as_guest(method, url_path, body, &standing, user_type)
            .map(|url| (url, level));
    }
    crate::server::person::send_judged(method, url_path, body, &standing, user_type).map(|url| (url, level))
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

/// A loopback reply as the `data` of an `http_request` answer: its status,
/// content type and body. A text body rides as is; bytes (Yinyue's WAV, an
/// image) ride as base64 with `body_encoding: "base64"` — read as UTF-8 they
/// arrived mangled, which is what once kept her voice off the channel.
async fn proxied_reply(r: reqwest::Response) -> serde_json::Value {
    let status = r.status().as_u16();
    let content_type = r
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = r.bytes().await.unwrap_or_default();
    reply_data(status, &content_type, &bytes)
}

/// `{ status, content_type, body[, body_encoding] }` for one reply.
fn reply_data(status: u16, content_type: &str, bytes: &[u8]) -> serde_json::Value {
    if is_text(content_type) {
        let body = String::from_utf8_lossy(bytes);
        return serde_json::json!({ "status": status, "content_type": content_type, "body": body });
    }
    use base64::Engine;
    let body = base64::engine::general_purpose::STANDARD.encode(bytes);
    serde_json::json!({
        "status": status,
        "content_type": content_type,
        "body": body,
        "body_encoding": "base64",
    })
}

/// Whether a content type is text, safe to carry as a UTF-8 string. No type
/// at all is an engine handler answering a bare string — text too.
fn is_text(content_type: &str) -> bool {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    mime.is_empty()
        || mime.starts_with("text/")
        || mime.ends_with("json")
        || mime.ends_with("+xml")
        || mime.ends_with("/xml")
        || mime.ends_with("javascript")
        || mime == "image/svg+xml"
}

/// One loopback request to this engine's own HTTP API. GET carries no body;
/// every other method sends `body` as JSON. `actor_device` names the paired
/// device asking, for handlers that keep per-device state; `level` is the
/// standing it was judged at (`LEVEL_HEADER`), for handlers that judge
/// within a route.
async fn loopback(
    client: &reqwest::Client,
    (method, url, body): (&str, &str, &serde_json::Value),
    tunnel_mark: &str,
    actor_device: Option<&str>,
    level: crate::engine::permission::PermissionMode,
) -> reqwest::Result<reqwest::Response> {
    let req = match method {
        "POST" => client.post(url).json(body),
        "PUT" => client.put(url).json(body),
        "PATCH" => client.patch(url).json(body),
        "DELETE" => client.delete(url).json(body),
        _ => client.get(url),
    };
    let req = req
        .header(crate::server::api::pair::TUNNEL_HEADER, tunnel_mark)
        .header(crate::server::api::pair::LEVEL_HEADER, level.to_string());
    let req = match actor_device {
        Some(d) => req.header(crate::server::api::pair::ACTOR_DEVICE_HEADER, d),
        None => req,
    };
    req.send().await
}

/// How long a request waits for its peer to say which device it is, counted
/// from the peer's first request. A phone names itself as its channel opens,
/// and its first requests can overtake that message; unnamed, they would be
/// no one's and memory would refuse them.
const IDENTIFY_GRACE: std::time::Duration = std::time::Duration::from_millis(1500);

/// The device a peer's request is re-issued for. A peer the tunnel already
/// knows as the owner needs no name; any other is given until
/// [`IDENTIFY_GRACE`] to send one.
async fn named_device(
    user_ctx: &crate::server::rtc::UserContext,
    actor: &super::PeerActor,
    first_request: std::time::Instant,
) -> Option<String> {
    use crate::server::api::pair::{TUNNEL_LOCAL, TUNNEL_OWNER};
    let named = || actor.lock_ok().as_ref().map(|a| a.device.clone());
    if matches!(user_ctx.tunnel_mark(), TUNNEL_LOCAL | TUNNEL_OWNER) {
        return named();
    }
    let deadline = first_request + IDENTIFY_GRACE;
    loop {
        if let Some(device) = named() {
            return Some(device);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
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
    actor: &super::PeerActor,
    first_request: std::time::Instant,
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
                || url_path.starts_with("/shared/")
                || url_path == "/index.html"
                || url_path == "/logo.svg";
            if !path_ok
                || !crate::server::routes::classes::sent_unchanged(&url_path)
                || url_path.contains('@')
                || url_path.contains("://")
                || url_path.contains("..")
            {
                return serde_json::json!({ "error": "Invalid URL path" });
            }
            // Carry the peer's identity into the loopback request. The phone
            // speaks only WebRTC, so without this a handler cannot tell which
            // paired device is asking — which is what per-phone state, like the
            // delete queue, needs to be correct with more than one phone.
            let actor_device = named_device(user_ctx, actor, first_request).await;
            let mut body_val = req
                .body
                .get("body")
                .unwrap_or(&serde_json::Value::Null)
                .clone();
            // A request is judged by this peer's standing and, when it comes
            // into a session, sent as its person.
            let user_type = user_ctx.user_type_for(actor.lock_ok().is_some());
            let (url_path, level) = match send_for_peer(
                user_ctx,
                actor_device.as_deref(),
                (method, &url_path, &mut body_val),
                user_type,
            ) {
                Ok(sent) => sent,
                Err(refused) => return refused_reply(&refused),
            };
            let url = format!("http://127.0.0.1:{port}{url_path}");
            // Per-request chatter → debug (fires on every status poll, session save,
            // etc. and drowns out lifecycle events at info level).
            tracing::debug!("RTC http_request: {method} {url_path}");
            let mark = user_ctx.tunnel_mark();
            match loopback(
                client,
                (method, &url, &body_val),
                mark,
                actor_device.as_deref(),
                level,
            )
            .await
            {
                Ok(r) => serde_json::json!({ "data": proxied_reply(r).await }),
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
            // Judged by this peer's standing and sent as its person,
            // resolved from the device it named.
            let actor_device = named_device(user_ctx, actor, first_request).await;
            let mut body = req.body.clone();
            let user_type = user_ctx.user_type_for(actor.lock_ok().is_some());
            let sent = send_for_peer(user_ctx, actor_device.as_deref(), ("POST", endpoint, &mut body), user_type);
            let level = match sent {
                Ok((_, level)) => level,
                Err(refused) => return refused.body(),
            };
            let url = format!("http://127.0.0.1:{port}{endpoint}");
            match loopback(client, ("POST", &url, &body), user_ctx.tunnel_mark(), None, level).await {
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
    /// A peer the tunnel does not know as the owner, with no device named,
    /// is no one; this machine's own page is the owner.
    #[test]
    fn a_peer_with_no_device_named_is_no_one_unless_the_tunnel_knows_it() {
        assert!(super::person_for(&UserContext::paired(), None).is_no_one());
        let lan = UserContext {
            reached_by: crate::server::rtc::Reach::Lan,
            ..UserContext::owner(Some("u-owner".into()))
        };
        assert!(super::person_for(&lan, None).is_no_one());
        let here = UserContext {
            reached_by: crate::server::rtc::Reach::ThisMachine,
            ..UserContext::owner(Some("u-owner".into()))
        };
        assert!(super::person_for(&here, None).is_owner());
        assert!(super::person_for(&here, Some("nobody-paired")).is_no_one());
    }

    use super::{is_text, named_device, reply_data, wrap_endpoint_response, IDENTIFY_GRACE};
    use crate::server::api::pair::Actor;
    use crate::server::rtc::UserContext;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    fn phone() -> Actor {
        Actor {
            device: "phone".into(),
            account: None,
        }
    }

    /// A phone's first request can overtake the message that names it: the
    /// request waits, and is re-issued as that phone's.
    /// A media get is judged as the control channel's GET: a peer that named
    /// no device stands at chat and is refused a read route; this machine's
    /// page reaches it at admin, sent as its person; a path that parsing
    /// moves is refused to anyone; a peer mid-pairing gets nothing.
    #[test]
    fn a_media_get_is_judged_like_the_channels() {
        use crate::engine::permission::PermissionMode;
        assert_eq!(super::media_get(&UserContext::paired(), None, "/api/topic/latest?topic=t"), None);
        let here = UserContext {
            reached_by: crate::server::rtc::Reach::ThisMachine,
            ..UserContext::owner(Some("u-owner".into()))
        };
        let got = super::media_get(&here, None, "/api/sessions/day").unwrap();
        let owners = format!("/api/sessions/day?user_id={}", crate::server::person::owner().wire());
        assert_eq!(got, (owners, PermissionMode::Admin));
        assert_eq!(super::media_get(&here, None, "/apps/x/.\t./.\t./api/credentials"), None);
        assert_eq!(super::media_get(&UserContext::pairing(), None, "/shared/a.js"), None);
        let unknown = super::media_get(&UserContext::paired(), Some("not-paired"), "/api/skills");
        assert_eq!(unknown, None, "a device not paired is no one at chat");
    }

    /// A guest (room consumer) names a session by id only to act on their
    /// own: the builder's, another guest's, and a day of another person are
    /// refused wherever the request names them, whatever the method; their
    /// own go through.
    #[test]
    fn a_guest_acts_only_on_their_own_sessions() {
        let guest = UserContext::consumer(
            "u-guest-probe".into(),
            "linggen".into(),
            crate::engine::permission::PermissionMode::Chat,
            None,
            None,
        );
        crate::server::person::keep_session_for_test("sess-probe-guest-own", Some("u-guest-probe"));
        crate::server::person::keep_session_for_test("sess-probe-guest-other", Some("u-other-guest"));
        crate::server::person::keep_session_for_test("sess-probe-guest-builder", None);
        let sent = |method: &str, url: &str, mut body: serde_json::Value| {
            super::send_for_peer(&guest, None, (method, url, &mut body), "consumer").is_ok()
        };
        for sid in ["sess-probe-guest-other", "sess-probe-guest-builder", "sess-day-2026-11-16"] {
            assert!(!sent("DELETE", "/api/sessions/all", json!({ "session_id": sid })), "delete {sid}");
            assert!(!sent("GET", &format!("/api/workspace/state?session_id={sid}"), json!(null)), "workspace {sid}");
            assert!(!sent("GET", &format!("/api/skill-sessions/state?session_id={sid}"), json!(null)), "skill {sid}");
        }
        // A body that is not an object is filled into a route's body type by
        // position (`["sess-…"]`), naming a session where the seam never looks.
        let by_position = json!(["sess-probe-guest-builder"]);
        assert!(!sent("DELETE", "/api/sessions/all", by_position), "delete by position");
        assert!(sent("DELETE", "/api/sessions/all", json!({ "session_id": "sess-probe-guest-own" })));
        assert!(sent("GET", "/api/workspace/state?session_id=sess-probe-guest-own", json!(null)));
    }

    #[tokio::test]
    async fn a_request_waits_for_its_peer_to_name_itself() {
        let actor: super::super::PeerActor = Arc::new(Mutex::new(None));
        let late = actor.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            *late.lock().unwrap() = Some(phone());
        });
        let named = named_device(&UserContext::paired(), &actor, Instant::now()).await;
        assert_eq!(named.as_deref(), Some("phone"));
    }

    /// The wait is counted from the peer's first request, so a peer that
    /// never names itself is held up once, not on every request; and a peer
    /// the tunnel knows as the owner is not held up at all.
    #[tokio::test]
    async fn the_wait_ends_and_the_owner_never_waits() {
        let nobody: super::super::PeerActor = Arc::new(Mutex::new(None));
        let long_ago = Instant::now() - IDENTIFY_GRACE;
        let asked = Instant::now();
        assert!(named_device(&UserContext::paired(), &nobody, long_ago)
            .await
            .is_none());
        assert!(
            named_device(&UserContext::owner(None), &nobody, Instant::now())
                .await
                .is_none()
        );
        assert!(asked.elapsed() < Duration::from_millis(200));
    }

    #[test]
    fn text_replies_ride_as_text() {
        for ct in [
            "application/json",
            "text/plain; charset=utf-8",
            "application/javascript",
            "image/svg+xml",
            "application/problem+json",
            "",
        ] {
            assert!(is_text(ct), "{ct}");
        }
        let out = reply_data(200, "application/json", br#"{"ok":true}"#);
        assert_eq!(
            out,
            json!({ "status": 200, "content_type": "application/json", "body": "{\"ok\":true}" })
        );
    }

    #[test]
    fn byte_replies_ride_as_base64_intact() {
        for ct in ["audio/wav", "image/png", "application/octet-stream"] {
            assert!(!is_text(ct), "{ct}");
        }
        // Not valid UTF-8: a lossy text read would have mangled these.
        let wav = [0x52, 0x49, 0x46, 0x46, 0xff, 0xfe, 0x00, 0x80];
        let out = reply_data(200, "audio/wav", &wav);
        assert_eq!(out["body_encoding"], "base64");
        assert_eq!(out["content_type"], "audio/wav");
        use base64::Engine;
        let back = base64::engine::general_purpose::STANDARD
            .decode(out["body"].as_str().unwrap())
            .unwrap();
        assert_eq!(back, wav);
    }

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
