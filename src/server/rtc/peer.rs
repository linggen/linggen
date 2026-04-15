//! WebRTC peer connection lifecycle — one per connected client.
//!
//! Each peer runs in its own tokio task with a dedicated UDP socket.
//! The task drives str0m's Sans-IO event loop: poll_output → transmit/handle events,
//! then recv from socket → handle_input.
//!
//! Data channels:
//! - "control": session lifecycle, heartbeat, RPC (request/response)
//! - "sess-{id}": per-session chat events, bridged to events_tx

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

use str0m::change::SdpOffer;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcConfig};

use crate::server::ServerState;

/// Create a new WebRTC peer connection from a WHIP SDP offer.
///
/// Returns the SDP answer string to send back to the client.
/// Spawns a background task to run the peer connection event loop.
pub async fn create_peer(offer_sdp: String, state: Arc<ServerState>) -> Result<String> {
    // Local WHIP — owner with Admin permission
    let user_ctx = super::UserContext::owner(
        crate::cli::login::load_remote_config().and_then(|c| c.user_id)
    );
    create_peer_inner(offer_sdp, state, false, user_ctx).await
}

/// Create a peer for a remote offer (via signaling relay).
/// Binds to 0.0.0.0 so STUN can discover the public address.
pub async fn create_remote_peer(
    offer_sdp: String,
    state: Arc<ServerState>,
    user_ctx: super::UserContext,
) -> Result<String> {
    create_peer_inner(offer_sdp, state, true, user_ctx).await
}

async fn create_peer_inner(
    offer_sdp: String,
    state: Arc<ServerState>,
    remote: bool,
    user_ctx: super::UserContext,
) -> Result<String> {
    // Parse the SDP offer (raw SDP text from WHIP POST body)
    let offer = SdpOffer::from_sdp_string(&offer_sdp)
        .context("Failed to parse SDP offer")?;

    // Bind a UDP socket for this peer connection.
    // Always bind to 0.0.0.0 so WebRTC works when accessing via LAN IP.
    let socket = UdpSocket::bind("0.0.0.0:0")
        .await
        .context("Failed to bind UDP socket")?;
    let local_addr = socket.local_addr()?;
    tracing::info!("WebRTC peer UDP socket bound to {local_addr} (remote={remote})");

    // Create str0m Rtc instance with ICE-lite.
    // str0m is Sans-IO — it can't do STUN discovery, so full ICE won't work.
    // ICE-lite makes us passive; the browser drives connectivity checks.
    let mut rtc = RtcConfig::new()
        .set_ice_lite(true)
        .build(Instant::now());

    // Add local candidate with the real LAN IP (not 127.0.0.1)
    // so WebRTC works both via localhost and via LAN IP.
    let local_ip = get_local_ip().unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    let candidate_addr = std::net::SocketAddr::new(local_ip, local_addr.port());
    let candidate = Candidate::host(candidate_addr, "udp").context("Failed to create host candidate")?;
    rtc.add_local_candidate(candidate)
        .context("Failed to add local candidate")?;

    // Accept the offer and generate answer
    let answer = rtc
        .sdp_api()
        .accept_offer(offer)
        .context("Failed to accept SDP offer")?;

    let answer_sdp = answer.to_sdp_string();
    tracing::debug!("SDP answer:\n{answer_sdp}");

    // Spawn the peer event loop
    let events_rx = state.events_tx.subscribe();
    tokio::spawn(async move {
        if let Err(e) = run_peer(rtc, socket, candidate_addr, state, events_rx, user_ctx).await {
            tracing::warn!("WebRTC peer exited: {e:#}");
        }
    });

    Ok(answer_sdp)
}

/// Run the str0m event loop for a single peer connection.
///
/// This bridges:
/// - Inbound data channel messages → server actions (chat, plan, etc.)
/// - Server events (events_tx) → outbound data channel messages
async fn run_peer(
    mut rtc: Rtc,
    socket: UdpSocket,
    local_candidate_addr: std::net::SocketAddr,
    state: Arc<ServerState>,
    mut events_rx: tokio::sync::broadcast::Receiver<crate::server::ServerEvent>,
    user_ctx: super::UserContext,
) -> Result<()> {
    let mut buf = vec![0u8; 65536];
    let mut control_channel_id = None;
    let mut inference_channel_id: Option<str0m::channel::ChannelId> = None;
    // Per-peer token counter — synced with persistent store for consumers.
    let tokens_used = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));
    // Load existing usage from persistent store for this consumer
    if user_ctx.is_consumer {
        let store = state.token_usage.lock().await;
        let (consumer_used, _) = store.get_usage(&user_ctx.user_id);
        tokens_used.store(consumer_used, std::sync::atomic::Ordering::Relaxed);
    }
    let mut session_channels: HashMap<String, str0m::channel::ChannelId> = HashMap::new();
    let mut channel_sessions: HashMap<str0m::channel::ChannelId, String> = HashMap::new();
    // Buffer events for sessions whose data channels aren't open yet.
    // When a session channel opens, flush the buffer.
    // Entries expire after 60s to prevent unbounded growth from dead sessions.
    let mut pending_events: HashMap<String, (Instant, Vec<String>)> = HashMap::new();
    // Reuse a single HTTP client for all proxy requests (connection pooling).
    let http_client = reqwest::Client::new();
    // Channel for async control request responses — avoids blocking str0m's event loop.
    let (ctrl_resp_tx, mut ctrl_resp_rx) = tokio::sync::mpsc::channel::<(Option<String>, str0m::channel::ChannelId, serde_json::Value)>(32);
    // Queue of pending data channel writes — drained as fast as SCTP buffer allows.
    // Messages are either text (JSON) or binary (gzip-compressed file data).
    let mut pending_dc_writes: std::collections::VecDeque<(str0m::channel::ChannelId, String)> = std::collections::VecDeque::new();
    let mut dc_write_paused = false;
    // Track when ICE entered Disconnected state for timeout-based cleanup.
    let mut disconnected_since: Option<Instant> = None;

    // Track which session IDs belong to this user (for event filtering).
    // Populated from session store on connect, updated on SessionCreated events.
    let mut user_session_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Ok(sessions) = state.manager.global_sessions.list_sessions() {
        for s in sessions {
            if s.user_id.as_deref() == Some(&user_ctx.user_id) {
                user_session_ids.insert(s.id);
            }
        }
    }

    // -- Page state push (replaces HTTP polling storm) --
    let mut view_ctx = super::page_state::ViewContext::default();
    let mut dirty_flags: u64 = 0;
    let mut force_page_state = false;
    let mut last_page_state_at = Instant::now();
    let mut page_state_interval = tokio::time::interval(Duration::from_secs(2));
    page_state_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        // Drain ONE pending write per cycle — writing multiple crashes str0m's SCTP.
        if !dc_write_paused {
            if let Some((cid, msg)) = pending_dc_writes.pop_front() {
                let written = rtc.channel(cid)
                    .map(|mut ch| ch.write(false, msg.as_bytes()))
                    .unwrap_or(Ok(false));
                match written {
                    Ok(true) => { /* accepted */ }
                    _ => {
                        pending_dc_writes.push_front((cid, msg));
                        dc_write_paused = true;
                    }
                }
            }
        }

        // Poll str0m for output
        let timeout = match rtc.poll_output()? {
            Output::Timeout(t) => t,

            Output::Transmit(t) => {
                socket.send_to(&t.contents, t.destination).await?;
                dc_write_paused = false; // buffer drained, can try writing again
                continue;
            }

            Output::Event(event) => {
                match event {
                    Event::IceConnectionStateChange(ice_state) => {
                        tracing::info!("WebRTC ICE state: {ice_state:?}");
                        if matches!(ice_state, IceConnectionState::Disconnected) {
                            if !rtc.is_alive() {
                                tracing::info!("WebRTC peer disconnected and no longer alive, exiting");
                                return Ok(());
                            }
                            // Start a disconnect timer — if still disconnected after 30s, exit.
                            disconnected_since = Some(Instant::now());
                        } else {
                            disconnected_since = None;
                        }
                    }

                    Event::ChannelOpen(id, label) => {
                        tracing::info!("Data channel opened: {label} (id: {id:?})");
                        if label == "control" {
                            control_channel_id = Some(id);
                            // Send connection metadata: user info + room info.
                            let data = if user_ctx.is_consumer {
                                let room_cfg = super::room_config::load_room_config();
                                serde_json::json!({
                                    "user": {
                                        "user_id": user_ctx.user_id,
                                        "user_type": "consumer",
                                        "user_name": user_ctx.user_name,
                                        "avatar_url": user_ctx.avatar_url,
                                    },
                                    "room": {
                                        "permission": user_ctx.permission.as_str(),
                                        "room_name": user_ctx.room_name,
                                        "token_budget_daily": user_ctx.token_budget_daily,
                                        "allowed_tools": room_cfg.allowed_tools,
                                        "allowed_skills": room_cfg.allowed_skills,
                                    },
                                })
                            } else {
                                serde_json::json!({
                                    "user": {
                                        "user_id": user_ctx.user_id,
                                        "user_type": "owner",
                                        "user_name": user_ctx.user_name,
                                        "avatar_url": user_ctx.avatar_url,
                                    },
                                })
                            };
                            let info_msg = serde_json::json!({
                                "kind": "user_info",
                                "data": data,
                            });
                            if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                                pending_dc_writes.push_back((id, info_msg.to_string()));
                            }
                            // Privacy warning for consumers
                            if user_ctx.is_consumer {
                                let warning = serde_json::json!({
                                    "kind": "notification",
                                    "data": {
                                        "type": "privacy_warning",
                                        "message": "You are chatting via a proxy room. The proxy owner can see your messages.",
                                        "persistent": true
                                    }
                                });
                                if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                                    pending_dc_writes.push_back((id, warning.to_string()));
                                }
                            }
                        } else if label == "inference" {
                            inference_channel_id = Some(id);
                        } else if let Some(session_id) = label.strip_prefix("sess-") {
                            // Verify session ownership for non-admin users.
                            // Check user_session_ids first (fast), then fall back to session store
                            // (handles race where channel opens before session_created event).
                            let mut allowed = user_ctx.permission.is_admin() || user_session_ids.contains(session_id);
                            if !allowed {
                                // Check session store — session may have just been created
                                if let Ok(Some(meta)) = state.manager.global_sessions.get_session_meta(session_id) {
                                    if meta.user_id.as_deref() == Some(&user_ctx.user_id) {
                                        user_session_ids.insert(session_id.to_string());
                                        allowed = true;
                                    }
                                }
                            }
                            if !allowed {
                                tracing::warn!("Rejected session channel for {session_id} — not owned by user {}", user_ctx.user_id);
                            } else {
                                session_channels.insert(session_id.to_string(), id);
                                channel_sessions.insert(id, session_id.to_string());
                                // Flush buffered events through the write queue (not directly —
                                // direct writes without poll_output() cause SCTP corruption).
                                if let Some((_created, buffered)) = pending_events.remove(session_id) {
                                    tracing::info!("Flushing {} buffered events for session {session_id}", buffered.len());
                                    for json in buffered {
                                        if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                                            pending_dc_writes.push_back((id, json));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    Event::ChannelData(data) => {
                        let text = String::from_utf8_lossy(&data.data).to_string();
                        tracing::trace!("Data channel message on {:?}: {}bytes", data.id, text.len());
                        if Some(data.id) == control_channel_id {
                            if let Some(req) = handle_control_message(
                                &mut rtc,
                                data.id,
                                &text,
                                &state,
                                &mut session_channels,
                                &mut channel_sessions,
                                &mut view_ctx,
                                &mut force_page_state,
                                &user_ctx,
                            ) {
                                // Enforce consumer permissions: browser consumers can only chat
                                // and load static assets. All dynamic data (sessions, models,
                                // skills) is pushed via page_state — no HTTP needed.
                                if req.msg_type == "http_request" {
                                    let url = req.body.get("url").and_then(|v| v.as_str()).unwrap_or("");
                                    let method = req.body.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
                                    if !user_ctx.permission.can_access_endpoint(method, url) {
                                        if let Some(rid) = &req.request_id {
                                            let err = serde_json::json!({
                                                "request_id": rid,
                                                "data": { "status": 403, "body": "{\"error\":\"Not allowed\"}" }
                                            });
                                            if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                                                pending_dc_writes.push_back((data.id, err.to_string()));
                                            }
                                        }
                                        continue;
                                    }
                                }

                                // Spawn async processing to avoid blocking str0m's event loop.
                                let tx = ctrl_resp_tx.clone();
                                let st = state.clone();
                                let client = http_client.clone();
                                let rid = req.request_id.clone();
                                let cid = req.channel_id;
                                let ctx_clone = user_ctx.clone();
                                let tok = tokens_used.clone();

                                if req.msg_type == "inference" || req.msg_type == "list_models" {
                                    // Inference/model-list: may stream multiple responses
                                    tokio::spawn(async move {
                                        process_inference_request(&req, &st, &ctx_clone, &tok, &tx, cid).await;
                                    });
                                } else {
                                    tokio::spawn(async move {
                                        let result = process_control_request_async(&req, &st, &client, &ctx_clone, &tok).await;
                                        let _ = tx.send((rid, cid, result)).await;
                                    });
                                }
                            }
                        } else if Some(data.id) == inference_channel_id {
                            // Inference channel: proxy client sends list_models / inference / room_chat
                            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&text) {
                                let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let request_id = msg.get("request_id").and_then(|v| v.as_str()).map(|s| s.to_string());
                                if msg_type == "inference" || msg_type == "list_models" {
                                    let req = ControlRequest {
                                        request_id,
                                        channel_id: data.id,
                                        msg_type,
                                        body: msg,
                                    };
                                    let tx = ctrl_resp_tx.clone();
                                    let st = state.clone();
                                    let ctx_clone = user_ctx.clone();
                                    let tok = tokens_used.clone();
                                    let cid = data.id;
                                    tokio::spawn(async move {
                                        process_inference_request(&req, &st, &ctx_clone, &tok, &tx, cid).await;
                                    });
                                } else if msg_type == "room_chat" {
                                    // Room chat from proxy consumer — broadcast to all local peers
                                    let chat_text = msg.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    if !chat_text.is_empty() && chat_text.len() <= 2000 {
                                        let sender_name = msg.get("sender_name").and_then(|v| v.as_str())
                                            .or(user_ctx.user_name.as_deref())
                                            .unwrap_or(&user_ctx.user_id)
                                            .chars().take(64).collect();
                                        let avatar_url = msg.get("avatar_url").and_then(|v| v.as_str()).map(|s| s.to_string())
                                            .or_else(|| user_ctx.avatar_url.clone());
                                        let _ = state.events_tx.send(crate::server::ServerEvent::RoomChat {
                                            sender_id: user_ctx.user_id.clone(),
                                            sender_name,
                                            avatar_url,
                                            text: chat_text,
                                        });
                                    }
                                } else {
                                    tracing::warn!("Unknown inference channel message type: {msg_type}");
                                }
                            }
                        } else if let Some(session_id) = channel_sessions.get(&data.id).cloned() {
                            // Check token budget before session messages
                            if let Some(budget) = user_ctx.token_budget_daily {
                                if tokens_used.load(std::sync::atomic::Ordering::Relaxed) >= budget {
                                    tracing::info!("Token budget exhausted, rejecting session message");
                                    continue;
                                }
                            }
                            // Spawn session message handling to avoid blocking str0m.
                            let st = state.clone();
                            let client = http_client.clone();
                            let ctx_clone = user_ctx.clone();
                            tokio::spawn(async move {
                                handle_session_message(&text, &session_id, &st, &client, &ctx_clone).await;
                            });
                        }
                    }

                    Event::ChannelClose(id) => {
                        if let Some(session_id) = channel_sessions.remove(&id) {
                            session_channels.remove(&session_id);
                            tracing::info!("Session channel closed: {session_id}");
                        }
                        if Some(id) == inference_channel_id {
                            inference_channel_id = None;
                            tracing::info!("Inference channel closed");
                        }
                        if Some(id) == control_channel_id {
                            control_channel_id = None;
                            tracing::info!("Control channel closed");
                        }
                    }

                    _ => {}
                }
                continue;
            }
        };

        // Exit if disconnected for more than 30 seconds (str0m has no Failed/Closed states).
        if let Some(since) = disconnected_since {
            if Instant::now().duration_since(since) > Duration::from_secs(30) {
                tracing::info!("WebRTC peer disconnected for 30s — exiting");
                return Ok(());
            }
        }

        // Calculate how long to wait
        let now = Instant::now();
        let wait = if timeout > now {
            timeout - now
        } else {
            Duration::ZERO
        };

        // Immediate page state push on view context change (don't wait for 2s tick)
        if force_page_state && control_channel_id.is_some() {
            let now_inst = Instant::now();
            if now_inst.duration_since(last_page_state_at) >= Duration::from_millis(200) {
                let flags = dirty_flags | super::page_state::DIRTY_ALL;
                dirty_flags = 0;
                force_page_state = false;
                last_page_state_at = now_inst;
                let cid = control_channel_id.unwrap();
                let st = state.clone();
                let tx = ctrl_resp_tx.clone();
                let user_ctx_clone = user_ctx.clone();
                let ctx = view_ctx.clone();
                tokio::spawn(async move {
                    let ps = super::page_state::build_page_state(&st, &ctx, flags, &user_ctx_clone).await;
                    if let Ok(data) = serde_json::to_value(&ps) {
                        let msg = serde_json::json!({ "kind": "page_state", "data": data });
                        let _ = tx.send((None, cid, msg)).await;
                    }
                });
            }
        }

        // Keep spinning without blocking if: timeout elapsed OR we have writes ready to send.
        // But do NOT spin when paused — we need to enter select! to receive UDP (SCTP ACKs).
        if wait.is_zero() || (!dc_write_paused && !pending_dc_writes.is_empty()) {
            rtc.handle_input(Input::Timeout(Instant::now()))?;
            continue;
        }

        // Wait for either: UDP packet, server event, or timeout
        tokio::select! {
            result = socket.recv_from(&mut buf) => {
                match result {
                    Ok((n, source)) => {
                        let contents: &[u8] = &buf[..n];
                        let receive = Receive {
                            proto: Protocol::Udp,
                            source,
                            destination: local_candidate_addr,
                            contents: contents.try_into()?,
                        };
                        rtc.handle_input(Input::Receive(Instant::now(), receive))?;
                    }
                    Err(e) => {
                        tracing::warn!("UDP recv error: {e}");
                    }
                }
            }

            // Forward server events to the appropriate session data channel
            result = events_rx.recv() => {
                match result {
                    Ok(crate::server::ServerEvent::RoomDisabled) if user_ctx.is_consumer => {
                        tracing::info!("Room disabled by owner — disconnecting consumer peer");
                        return Ok(());
                    }
                    Ok(event) => {
                        let mut filter = EventFilter {
                            session_ids: &mut user_session_ids,
                            user_id: &user_ctx.user_id,
                            is_admin: user_ctx.permission.is_admin(),
                        };
                        forward_event_to_channels(
                            &event, &session_channels, control_channel_id,
                            inference_channel_id,
                            &mut pending_events, &mut pending_dc_writes,
                            &state, &mut dirty_flags, &mut filter,
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("WebRTC event relay lagged — dropped {n} events");
                    }
                    Err(_) => {}
                }
            }

            // Receive async control request responses and send on data channel
            Some((rid, cid, result)) = ctrl_resp_rx.recv() => {
                match rid {
                    Some(rid) => enqueue_response(&mut pending_dc_writes, cid, &rid, result),
                    None => {
                        // Unsolicited push (e.g. page_state) — write directly
                        if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                            pending_dc_writes.push_back((cid, result.to_string()));
                        }
                    }
                }
            }

            // Page state heartbeat — push aggregated state every 2s when dirty
            _ = page_state_interval.tick() => {
                let should_send = (dirty_flags != 0 || force_page_state)
                    && control_channel_id.is_some();
                if should_send {
                    let now_inst = Instant::now();
                    // Debounce: skip if last push was < 200ms ago (rapid context changes)
                    if now_inst.duration_since(last_page_state_at) >= Duration::from_millis(200) {
                        let flags = dirty_flags;
                        dirty_flags = 0;
                        force_page_state = false;
                        last_page_state_at = now_inst;
                        let cid = control_channel_id.unwrap();
                        let st = state.clone();
                        let tx = ctrl_resp_tx.clone();
                        let user_ctx_clone = user_ctx.clone();
                        let ctx = view_ctx.clone();
                        tokio::spawn(async move {
                            let ps = super::page_state::build_page_state(&st, &ctx, flags, &user_ctx_clone).await;
                            if let Ok(data) = serde_json::to_value(&ps) {
                                let msg = serde_json::json!({ "kind": "page_state", "data": data });
                                let _ = tx.send((None, cid, msg)).await;
                            }
                        });
                    }
                }
            }

            _ = tokio::time::sleep(wait) => {
                rtc.handle_input(Input::Timeout(Instant::now()))?;
            }
        }
    }
}

/// Pending control channel requests that need async processing.
/// Since str0m's event loop is synchronous, we queue async work and
/// process it outside the poll loop.
struct ControlRequest {
    request_id: Option<String>,
    channel_id: str0m::channel::ChannelId,
    msg_type: String,
    body: serde_json::Value,
}

/// Handle a message on the control data channel.
/// Returns an optional async request to process outside the str0m loop.
fn handle_control_message(
    rtc: &mut Rtc,
    channel_id: str0m::channel::ChannelId,
    text: &str,
    state: &Arc<ServerState>,
    _session_channels: &mut HashMap<String, str0m::channel::ChannelId>,
    _channel_sessions: &mut HashMap<str0m::channel::ChannelId, String>,
    view_ctx: &mut super::page_state::ViewContext,
    force_page_state: &mut bool,
    user_ctx: &super::UserContext,
) -> Option<ControlRequest> {
    let msg: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Control message parse error: {e}");
            return None;
        }
    };

    let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let request_id = msg.get("request_id").and_then(|v| v.as_str()).map(|s| s.to_string());

    match msg_type.as_str() {
        "heartbeat" => {
            if let Some(mut ch) = rtc.channel(channel_id) {
                let resp = serde_json::json!({ "type": "heartbeat", "ts": chrono::Utc::now().timestamp_millis() });
                let _ = ch.write(false, resp.to_string().as_bytes());
            }
            None
        }

        "http_request" | "chat" | "clear" | "compact"
        | "plan_approve" | "plan_reject" | "plan_edit"
        | "ask_user_response"
        | "inference" | "list_models" => {
            // These need async processing — return as pending request
            Some(ControlRequest {
                request_id,
                channel_id,
                msg_type,
                body: msg,
            })
        }

        "set_view_context" => {
            view_ctx.session_id = msg.get("session_id").and_then(|v| v.as_str()).map(|s| s.to_string());
            view_ctx.project_root = msg.get("project_root").and_then(|v| v.as_str()).map(|s| s.to_string());
            view_ctx.is_compact = msg.get("is_compact").and_then(|v| v.as_bool()).unwrap_or(false);
            *force_page_state = true;
            tracing::debug!("View context updated: session={:?} project={:?} compact={}", view_ctx.session_id, view_ctx.project_root, view_ctx.is_compact);
            None
        }

        "room_chat" => {
            let text = msg.get("text").and_then(|v| v.as_str()).unwrap_or("");
            if text.is_empty() || text.len() > 2000 {
                return None;
            }
            let text = text.to_string();
            // Prefer server-side user_name (trusted), fall back to client-provided sender_name
            let sender_name: String = user_ctx.user_name.as_deref()
                .or_else(|| msg.get("sender_name").and_then(|v| v.as_str()))
                .unwrap_or(&user_ctx.user_id)
                .chars().take(64).collect();
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
async fn process_control_request_async(
    req: &ControlRequest,
    state: &Arc<ServerState>,
    client: &reqwest::Client,
    user_ctx: &super::UserContext,
    tokens_used: &std::sync::Arc<std::sync::atomic::AtomicI64>,
) -> serde_json::Value {
    let port = state.port;

    // Check token budget before chat calls
    if let Some(budget) = user_ctx.token_budget_daily {
        if tokens_used.load(std::sync::atomic::Ordering::Relaxed) >= budget && matches!(req.msg_type.as_str(), "chat" | "plan_approve" | "plan_reject" | "plan_edit") {
            return serde_json::json!({
                "error": "Token budget exhausted for today. Please try again tomorrow or ask the proxy owner to increase the budget."
            });
        }
    }

    match req.msg_type.as_str() {
        "http_request" => {
            let method = req.body.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
            let url_path_raw = req.body.get("url").and_then(|v| v.as_str()).unwrap_or("/");
            // SECURITY: percent-decode then validate to prevent SSRF bypass via %2e%2e etc.
            let url_path = urlencoding::decode(url_path_raw).unwrap_or(std::borrow::Cow::Borrowed(url_path_raw));
            let path_ok = url_path.starts_with("/api/")
                || url_path.starts_with("/assets/")
                || url_path.starts_with("/apps/")
                || url_path == "/index.html"
                || url_path == "/logo.svg";
            if !path_ok || url_path.contains('@') || url_path.contains("://") || url_path.contains("..") {
                return serde_json::json!({ "error": "Invalid URL path" });
            }
            let url = format!("http://127.0.0.1:{port}{url_path}");
            if method == "GET" {
                tracing::trace!("RTC http_request: {method} {url_path}");
            } else {
                tracing::info!("RTC http_request: {method} {url_path}");
            }
            let mut body_val = req.body.get("body").unwrap_or(&serde_json::Value::Null).clone();
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

        "chat" | "clear" | "compact"
        | "plan_approve" | "plan_reject" | "plan_edit"
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

/// Process inference and list_models requests from proxy room consumers.
/// These are used by the "linggen server proxy" mode where a consumer's linggen
/// uses the owner's linggen as a model provider.
///
/// Protocol:
/// - list_models: returns { request_id, data: { models: [...] } }
/// - inference: streams { request_id, chunk: { type, ... } } then { request_id, done: true }
async fn process_inference_request(
    req: &ControlRequest,
    state: &Arc<ServerState>,
    user_ctx: &super::UserContext,
    tokens_used: &std::sync::Arc<std::sync::atomic::AtomicI64>,
    tx: &tokio::sync::mpsc::Sender<(Option<String>, str0m::channel::ChannelId, serde_json::Value)>,
    cid: str0m::channel::ChannelId,
) {
    use futures_util::StreamExt;

    let rid = req.request_id.clone();

    // Only allow inference from linggen-type consumers (not browser consumers)
    if user_ctx.consumer_type.as_deref() != Some("linggen") && !user_ctx.permission.is_admin() {
        let _ = tx.send((rid.clone(), cid, serde_json::json!({
            "error": "Inference endpoint is only available for linggen server consumers"
        }))).await;
        return;
    }

    match req.msg_type.as_str() {
        "list_models" => {
            let room_cfg = super::room_config::load_room_config();
            let models = state.manager.models.read().await;
            let model_list: Vec<serde_json::Value> = models.list_models().iter()
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
                }).collect();
            let _ = tx.send((rid, cid, serde_json::json!({
                "data": { "models": model_list }
            }))).await;
        }

        "inference" => {
            // Check token budget (persistent store: room-level + per-consumer)
            {
                let room_cfg = super::room_config::load_room_config();
                let mut store = state.token_usage.lock().await;
                if !store.check_budget(
                    &user_ctx.user_id,
                    room_cfg.token_budget_room_daily,
                    room_cfg.token_budget_consumer_daily.or(user_ctx.token_budget_daily),
                ) {
                    let _ = tx.send((rid.clone(), cid, serde_json::json!({
                        "error": "Token budget exhausted for today"
                    }))).await;
                    return;
                }
            }

            let model_id = req.body.get("model").and_then(|v| v.as_str()).unwrap_or("");
            if model_id.is_empty() {
                let _ = tx.send((rid.clone(), cid, serde_json::json!({ "error": "model required" }))).await;
                return;
            }

            // Verify the model is in the shared list
            let room_cfg = super::room_config::load_room_config();
            if !room_cfg.shared_models.contains(&model_id.to_string()) {
                let _ = tx.send((rid.clone(), cid, serde_json::json!({
                    "error": format!("Model '{model_id}' is not shared in this room")
                }))).await;
                return;
            }

            // Parse messages
            let messages: Vec<crate::ollama::ChatMessage> = match req.body.get("messages") {
                Some(m) => match serde_json::from_value(m.clone()) {
                    Ok(msgs) => msgs,
                    Err(e) => {
                        let _ = tx.send((rid.clone(), cid, serde_json::json!({
                            "error": format!("Invalid messages: {e}")
                        }))).await;
                        return;
                    }
                },
                None => {
                    let _ = tx.send((rid.clone(), cid, serde_json::json!({ "error": "messages required" }))).await;
                    return;
                }
            };

            let tools: Option<Vec<serde_json::Value>> = req.body.get("tools")
                .and_then(|v| serde_json::from_value(v.clone()).ok());

            tracing::info!("Inference request: model={model_id}, messages={}, tools={}", messages.len(), tools.as_ref().map(|t| t.len()).unwrap_or(0));

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
                    let _ = tx.send((rid.clone(), cid, serde_json::json!({
                        "error": format!("Model error: {e}")
                    }))).await;
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
                            tokens_used.fetch_add(total as i64, std::sync::atomic::Ordering::Relaxed);
                            state.token_usage.lock().await.record_usage(&user_ctx.user_id, total as i64);
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
                        let _ = tx.send((rid.clone(), cid, serde_json::json!({
                            "error": format!("Stream error: {e}")
                        }))).await;
                        return;
                    }
                };
                if tx.send((rid.clone(), cid, chunk_json)).await.is_err() {
                    return; // Connection closed
                }
            }

            // Signal stream end
            let _ = tx.send((rid.clone(), cid, serde_json::json!({ "done": true }))).await;
        }

        _ => {
            let _ = tx.send((rid, cid, serde_json::json!({ "error": "unknown inference type" }))).await;
        }
    }
}

/// Handle a message on a session data channel (chat, plan actions, etc.).
///
/// Proxies to the local HTTP API to reuse existing handler logic.
/// This avoids duplicating complex chat/plan/ask-user code.
async fn handle_session_message(
    text: &str,
    session_id: &str,
    state: &Arc<ServerState>,
    client: &reqwest::Client,
    user_ctx: &super::UserContext,
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
            body["user_type"] = serde_json::Value::String(
                user_ctx.user_type().to_string(),
            );
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
    tracing::info!("RTC session proxy: {msg_type} → {endpoint}");
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

/// Max base64 chunk size per JSON message (~48 KB raw → ~64 KB base64, well under 256 KB SCTP limit).
const MAX_CHUNK_RAW: usize = 48_000;
/// Max pending data channel writes before dropping new messages.
const MAX_DC_WRITE_QUEUE: usize = 4000;

/// Enqueue a response, using gzip + base64 for large bodies.
///
/// Protocol for large responses (all text, no binary DC):
/// 1. JSON: `{ request_id, gzip_start: { total_bytes, chunks, status } }`
/// 2. JSON: `{ request_id, gzip_chunk: "<base64 data>" }` × N
/// 3. JSON: `{ request_id, gzip_end: true }`
///
/// Client collects base64 chunks, decodes to binary, decompresses gzip.
fn enqueue_response(
    queue: &mut std::collections::VecDeque<(str0m::channel::ChannelId, String)>,
    cid: str0m::channel::ChannelId,
    request_id: &str,
    result: serde_json::Value,
) {
    if queue.len() >= MAX_DC_WRITE_QUEUE {
        tracing::warn!("DC write queue full ({MAX_DC_WRITE_QUEUE}), dropping response for {request_id}");
        return;
    }

    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;

    let body = result.get("data")
        .and_then(|d| d.get("body"))
        .and_then(|b| b.as_str());

    if let Some(body_str) = body {
        if body_str.len() > MAX_CHUNK_RAW {
            let status = result.get("data")
                .and_then(|d| d.get("status"))
                .and_then(|s| s.as_u64())
                .unwrap_or(200);

            // Gzip compress the body
            use flate2::write::GzEncoder;
            use flate2::Compression;
            use std::io::Write;
            let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
            encoder.write_all(body_str.as_bytes()).ok();
            let compressed = encoder.finish().unwrap_or_default();

            // Split compressed bytes into chunks, base64-encode each
            let raw_chunks: Vec<&[u8]> = compressed.chunks(MAX_CHUNK_RAW).collect();
            let num_chunks = raw_chunks.len();

            tracing::info!(
                "Response for {request_id}: {}KB → gzip {}KB → {num_chunks} base64 chunks",
                body_str.len() / 1024,
                compressed.len() / 1024,
            );

            // Check if there's enough room for the full gzip transfer (header + chunks + footer)
            let needed = 2 + num_chunks;
            if queue.len() + needed > MAX_DC_WRITE_QUEUE {
                tracing::warn!("DC write queue too full for gzip response ({needed} entries needed), dropping {request_id}");
                let err = serde_json::json!({ "request_id": request_id, "error": "Queue full" });
                queue.push_back((cid, err.to_string()));
                return;
            }

            // Header
            let header = serde_json::json!({
                "request_id": request_id,
                "gzip_start": { "total_bytes": compressed.len(), "chunks": num_chunks, "status": status }
            });
            queue.push_back((cid, header.to_string()));

            // Base64-encoded chunks
            for chunk in &raw_chunks {
                let encoded = b64.encode(chunk);
                let msg = serde_json::json!({
                    "request_id": request_id,
                    "gzip_chunk": encoded
                });
                queue.push_back((cid, msg.to_string()));
            }

            // Footer
            let footer = serde_json::json!({
                "request_id": request_id,
                "gzip_end": true
            });
            queue.push_back((cid, footer.to_string()));
            return;
        }
    }

    // Small response — single JSON message
    let mut resp = result;
    resp["request_id"] = serde_json::Value::String(request_id.to_string());
    queue.push_back((cid, resp.to_string()));
}

/// Get the local (non-loopback) IP address for WebRTC host candidates.
/// Connects a UDP socket to a public address to determine the local IP
/// (no actual packets are sent).
pub(crate) fn get_local_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

/// Forward a server event to the appropriate data channel.
/// Max buffered events per session before the channel opens.
const MAX_PENDING_EVENTS: usize = 2000;

/// Event filter context — passed to forward_event_to_channels for all peers.
/// Admin peers skip session filtering; non-admin peers only see their own sessions.
struct EventFilter<'a> {
    session_ids: &'a mut std::collections::HashSet<String>,
    user_id: &'a str,
    is_admin: bool,
}

fn forward_event_to_channels(
    event: &crate::server::ServerEvent,
    session_channels: &HashMap<String, str0m::channel::ChannelId>,
    control_channel_id: Option<str0m::channel::ChannelId>,
    inference_channel_id: Option<str0m::channel::ChannelId>,
    pending_events: &mut HashMap<String, (Instant, Vec<String>)>,
    pending_dc_writes: &mut std::collections::VecDeque<(str0m::channel::ChannelId, String)>,
    state: &Arc<ServerState>,
    dirty_flags: &mut u64,
    filter: &mut EventFilter<'_>,
) {
    // StateUpdated → set dirty flag for page state push, don't forward to DC
    if matches!(event, crate::server::ServerEvent::StateUpdated) {
        *dirty_flags |= super::page_state::DIRTY_ALL;
        return;
    }

    // Prune expired pending event buffers (older than 60s)
    let now = Instant::now();
    pending_events.retain(|_, (created, _)| now.duration_since(*created) < Duration::from_secs(60));
    // Map the server event to a UI message (same as SSE handler)
    let seq = state.event_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut ui_msg = match crate::server::map_server_event_to_ui_message(event.clone(), seq) {
        Some(msg) => msg,
        None => return,
    };

    // Session isolation: non-admin peers only see their own sessions.
    // Admin peers skip filtering entirely.
    if !filter.is_admin {
        match ui_msg.session_id.as_deref() {
            Some("global") | None => {
                // Room chat must reach all peers regardless of permission level.
                if ui_msg.kind != "room_chat" {
                    // Global events — drop for non-admin (they get page_state instead)
                    return;
                }
            }
            Some(sid) => {
                // For SessionCreated events, check if the new session belongs to us
                if ui_msg.kind == "session_created" {
                    if let Ok(Some(meta)) = state.manager.global_sessions.get_session_meta(sid) {
                        if meta.user_id.as_deref() == Some(filter.user_id) {
                            filter.session_ids.insert(sid.to_string());
                        }
                    }
                }
                if !filter.session_ids.contains(sid) {
                    return;
                }
            }
        }
    }

    let json = match serde_json::to_string(&ui_msg) {
        Ok(j) => j,
        Err(_) => return,
    };

    // Route to session channel if available, buffer if channel not yet open.
    // All writes go through the pending_dc_writes queue — writing directly to
    // str0m channels without a poll_output() in between causes SCTP corruption.
    //
    // Broadcast events (notifications, agent_status, activity, run) are also
    // sent to the control channel so the main page can react (e.g. session list
    // updates when a skill or mission creates a new session).
    let is_broadcast = matches!(ui_msg.kind.as_str(),
        "agent_status" | "activity" | "run" | "notification"
        | "ask_user" | "widget_resolved" | "room_chat"
    );
    match ui_msg.session_id.as_deref() {
        Some("global") | None => {
            if let Some(cid) = control_channel_id {
                if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                    pending_dc_writes.push_back((cid, json.clone()));
                }
            }
        }
        Some(sid) => {
            // Send to the session's dedicated channel
            if let Some(&cid) = session_channels.get(sid) {
                if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                    pending_dc_writes.push_back((cid, json.clone()));
                }
            } else if !is_broadcast {
                // Channel not yet open — buffer the event
                let (_, buf) = pending_events.entry(sid.to_string()).or_insert_with(|| (Instant::now(), Vec::new()));
                if buf.len() < MAX_PENDING_EVENTS {
                    buf.push(json.clone());
                }
            }
            // Also send broadcast events to control channel for main page
            if is_broadcast {
                if let Some(cid) = control_channel_id {
                    if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                        pending_dc_writes.push_back((cid, json.clone()));
                    }
                }
            }
        }
    }

    // Forward room_chat to inference channel — only on the OWNER side
    // (owner → consumer direction). Consumer → owner is handled by proxy_client loop.
    // We detect the owner side by checking: inference_channel_id is set (proxy peer)
    // AND control_channel_id is NOT set (proxy peers don't have a control channel).
    if ui_msg.kind == "room_chat" && inference_channel_id.is_some() && control_channel_id.is_none() {
        if let Some(cid) = inference_channel_id {
            let chat_msg = serde_json::json!({
                "type": "room_chat",
                "data": ui_msg.data,
            });
            if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                pending_dc_writes.push_back((cid, chat_msg.to_string()));
            }
        }
    }
}
