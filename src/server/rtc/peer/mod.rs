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

mod control;
mod media_channel;
mod forward;
mod inference;
mod response;
mod session;
use control::{handle_control_message, process_control_request_async};
use forward::{forward_event_to_channels, EventFilter};
use inference::process_inference_request;
use response::{enqueue_push, enqueue_response, DcWrite, MAX_DC_WRITE_QUEUE};
use session::handle_session_message;

/// Create a new WebRTC peer connection from a WHIP SDP offer.
///
/// Returns the SDP answer string to send back to the client.
/// Spawns a background task to run the peer connection event loop.
pub async fn create_peer(offer_sdp: String, state: Arc<ServerState>) -> Result<String> {
    // Local WHIP — owner with Admin permission
    let user_ctx =
        super::UserContext::owner(crate::account::load_account().and_then(|a| a.user_id));
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
    let offer = SdpOffer::from_sdp_string(&offer_sdp).context("Failed to parse SDP offer")?;

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
    let mut rtc = RtcConfig::new().set_ice_lite(true).build(Instant::now());

    // Advertise both the LAN IP and 127.0.0.1 as host candidates. The LAN
    // IP makes WebRTC work for browser-to-daemon over the local network;
    // 127.0.0.1 is required for in-process webviews (Tauri, etc.) where
    // the LAN-IP path passes ICE Checking but fails consent-freshness
    // ~15s later, dropping the data channel.
    let port = local_addr.port();
    let local_ip = get_local_ip().unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    let candidate_addr = std::net::SocketAddr::new(local_ip, port);
    let mut host_ips: Vec<std::net::IpAddr> = vec![local_ip];
    let loopback = std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
    if !host_ips.contains(&loopback) {
        host_ips.push(loopback);
    }
    for ip in host_ips {
        let candidate = Candidate::host(std::net::SocketAddr::new(ip, port), "udp")
            .context("Failed to create host candidate")?;
        rtc.add_local_candidate(candidate)
            .context("Failed to add local candidate")?;
    }

    // Accept the offer and generate answer
    let answer = rtc
        .sdp_api()
        .accept_offer(offer)
        .context("Failed to accept SDP offer")?;

    let answer_sdp = answer.to_sdp_string();
    // SDPs are 20+ lines of boilerplate that aren't useful at DEBUG. Log the
    // interesting bits (bundle/ICE ufrag/candidate) inline; keep the full
    // string at TRACE for when someone is actually debugging SDP negotiation.
    let sdp_summary = summarize_sdp(&answer_sdp);
    tracing::debug!("SDP answer: {sdp_summary}");
    tracing::trace!("SDP answer full:\n{answer_sdp}");

    // Spawn the peer event loop. Track peer lifetime in active_peer_count
    // so the idle-shutdown watcher knows when no clients remain.
    let events_rx = state.events_tx.subscribe();
    // Unique id for the Yinyue presenter registry (FCFS singleton lock).
    let peer_id = state.mint_peer_id();
    state
        .active_peer_count
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let counter = state.active_peer_count.clone();
    let cleanup_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) =
            run_peer(rtc, socket, candidate_addr, state, events_rx, user_ctx, peer_id).await
        {
            tracing::warn!("WebRTC peer exited: {e:#}");
        }
        counter.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        // Release the Yinyue presenter lock on disconnect (no-op if this peer
        // never subscribed); if it held the lock, the next subscriber is promoted.
        cleanup_state.yinyue_release(peer_id);
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
    peer_id: u64,
) -> Result<()> {
    let mut buf = vec![0u8; 65536];
    let mut control_channel_id = None;
    // Bulk bytes live on their own channel so a photo can't stall chat, and
    // so they never pass through the control channel's text envelope.
    let mut media_channel_id: Option<str0m::channel::ChannelId> = None;
    let mut media_transfer: Option<media_channel::MediaTransfer> = None;
    // Download chunks are produced off-loop and queued here; bounded so a slow
    // peer applies backpressure to the reader instead of growing memory.
    let (media_out_tx, mut media_out_rx) =
        tokio::sync::mpsc::channel::<response::DcWrite>(64);
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
    let (ctrl_resp_tx, mut ctrl_resp_rx) = tokio::sync::mpsc::channel::<(
        Option<String>,
        str0m::channel::ChannelId,
        serde_json::Value,
    )>(32);
    // Queue of pending data channel writes — drained as fast as SCTP buffer
    // allows. Each entry carries its own text/binary flag: control and session
    // frames are JSON, media downloads are raw bytes.
    let mut pending_dc_writes: std::collections::VecDeque<DcWrite> =
        std::collections::VecDeque::new();
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
            if let Some(write) = pending_dc_writes.pop_front() {
                let written = rtc
                    .channel(write.channel)
                    .map(|mut ch| ch.write(write.binary, &write.data))
                    .unwrap_or(Ok(false));
                match written {
                    Ok(true) => { /* accepted */ }
                    _ => {
                        pending_dc_writes.push_front(write);
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
                                tracing::info!(
                                    "WebRTC peer disconnected and no longer alive, exiting"
                                );
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
                        if label == "media" {
                            media_channel_id = Some(id);
                        } else if label == "control" {
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
                                pending_dc_writes.push_back(DcWrite::text(id, info_msg.to_string()));
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
                                    pending_dc_writes.push_back(DcWrite::text(id, warning.to_string()));
                                }
                            }
                        } else if label == "inference" {
                            inference_channel_id = Some(id);
                        } else if let Some(session_id) = label.strip_prefix("sess-") {
                            // Verify session ownership for non-admin users.
                            // Check user_session_ids first (fast), then fall back to session store
                            // (handles race where channel opens before session_created event).
                            let mut allowed = user_ctx.permission.is_admin()
                                || user_session_ids.contains(session_id);
                            if !allowed {
                                // Check session store — session may have just been created
                                if let Ok(Some(meta)) =
                                    state.manager.global_sessions.get_session_meta(session_id)
                                {
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
                                if let Some((_created, buffered)) =
                                    pending_events.remove(session_id)
                                {
                                    tracing::debug!(
                                        "Flushing {} buffered events for session {session_id}",
                                        buffered.len()
                                    );
                                    for json in buffered {
                                        if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                                            pending_dc_writes.push_back(DcWrite::text(id, json));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    Event::ChannelData(data) => {
                        if Some(data.id) == media_channel_id {
                            let reply = if data.binary {
                                media_channel::handle_binary(&data.data, &mut media_transfer).await
                            } else {
                                let text = String::from_utf8_lossy(&data.data).to_string();
                                if media_channel::spawn_get(
                                    &text,
                                    state.port,
                                    data.id,
                                    media_out_tx.clone(),
                                ) {
                                    continue;
                                }
                                media_channel::handle_text(&text, &mut media_transfer).await
                            };
                            if let Some(msg) = reply {
                                if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                                    pending_dc_writes.push_back(DcWrite::text(data.id, msg));
                                }
                            }
                            continue;
                        }
                        let text = String::from_utf8_lossy(&data.data).to_string();
                        tracing::trace!(
                            "Data channel message on {:?}: {}bytes",
                            data.id,
                            text.len()
                        );
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
                                peer_id,
                            ) {
                                // Enforce consumer permissions: browser consumers can only chat
                                // and load static assets. All dynamic data (sessions, models,
                                // skills) is pushed via page_state — no HTTP needed.
                                if req.msg_type == "http_request" {
                                    let url =
                                        req.body.get("url").and_then(|v| v.as_str()).unwrap_or("");
                                    let method = req
                                        .body
                                        .get("method")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("GET");
                                    if !user_ctx.permission.can_access_endpoint(method, url) {
                                        if let Some(rid) = &req.request_id {
                                            let err = serde_json::json!({
                                                "request_id": rid,
                                                "data": { "status": 403, "body": "{\"error\":\"Not allowed\"}" }
                                            });
                                            if pending_dc_writes.len() < MAX_DC_WRITE_QUEUE {
                                                pending_dc_writes
                                                    .push_back(DcWrite::text(data.id, err.to_string()));
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
                                        process_inference_request(
                                            &req, &st, &ctx_clone, &tok, &tx, cid,
                                        )
                                        .await;
                                    });
                                } else {
                                    tokio::spawn(async move {
                                        let result = process_control_request_async(
                                            &req, &st, &client, &ctx_clone, &tok,
                                        )
                                        .await;
                                        let _ = tx.send((rid, cid, result)).await;
                                    });
                                }
                            }
                        } else if Some(data.id) == inference_channel_id {
                            // Inference channel: proxy client sends list_models / inference / room_chat
                            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&text) {
                                let msg_type = msg
                                    .get("type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let request_id = msg
                                    .get("request_id")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());
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
                                        process_inference_request(
                                            &req, &st, &ctx_clone, &tok, &tx, cid,
                                        )
                                        .await;
                                    });
                                } else if msg_type == "room_chat" {
                                    // Room chat from proxy consumer — broadcast to all local peers
                                    let chat_text = msg
                                        .get("text")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    if !chat_text.is_empty() && chat_text.len() <= 2000 {
                                        let sender_name = msg
                                            .get("sender_name")
                                            .and_then(|v| v.as_str())
                                            .or(user_ctx.user_name.as_deref())
                                            .unwrap_or(&user_ctx.user_id)
                                            .chars()
                                            .take(64)
                                            .collect();
                                        let avatar_url = msg
                                            .get("avatar_url")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string())
                                            .or_else(|| user_ctx.avatar_url.clone());
                                        tracing::info!(
                                            "[room_chat] inbound on inference channel from user_id={} text_len={}",
                                            user_ctx.user_id,
                                            chat_text.len()
                                        );
                                        let _ = state.events_tx.send(
                                            crate::server::ServerEvent::RoomChat {
                                                sender_id: user_ctx.user_id.clone(),
                                                sender_name,
                                                avatar_url,
                                                text: chat_text,
                                            },
                                        );
                                    }
                                } else {
                                    tracing::warn!(
                                        "Unknown inference channel message type: {msg_type}"
                                    );
                                }
                            }
                        } else if let Some(session_id) = channel_sessions.get(&data.id).cloned() {
                            // Check token budget before session messages
                            if let Some(budget) = user_ctx.token_budget_daily {
                                if tokens_used.load(std::sync::atomic::Ordering::Relaxed) >= budget
                                {
                                    tracing::info!(
                                        "Token budget exhausted, rejecting session message"
                                    );
                                    continue;
                                }
                            }
                            // Spawn session message handling to avoid blocking str0m.
                            let st = state.clone();
                            let client = http_client.clone();
                            let ctx_clone = user_ctx.clone();
                            tokio::spawn(async move {
                                handle_session_message(
                                    &text,
                                    &session_id,
                                    &st,
                                    &client,
                                    &ctx_clone,
                                )
                                .await;
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
                    let ps = super::page_state::build_page_state(&st, &ctx, flags, &user_ctx_clone)
                        .await;
                    if let Ok(data) = serde_json::to_value(&ps) {
                        let size = data.to_string().len();
                        tracing::info!(
                            "Pushing page_state (forced): {}bytes, models={}",
                            size,
                            data.get("models")
                                .and_then(|v| v.as_array())
                                .map(|a| a.len())
                                .unwrap_or(0)
                        );
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
            // Media download chunks, produced off-loop by spawn_get. Guarded:
            // while the SCTP buffer is full this arm is always ready and would
            // starve the socket read below — no ACKs, no drain, no progress.
            // Leaving the chunks in the bounded channel backpressures the
            // producer instead, which is what we want anyway.
            Some(write) = media_out_rx.recv(),
                if !dc_write_paused && pending_dc_writes.len() < MAX_DC_WRITE_QUEUE => {
                pending_dc_writes.push_back(write);
            }

            result = socket.recv_from(&mut buf) => {
                match result {
                    Ok((n, source)) => {
                        let contents: &[u8] = &buf[..n];
                        // Pick the local candidate destination that matches the
                        // packet's source. A packet from 127.0.0.1 must have been
                        // sent to 127.0.0.1; one from a LAN address went to the
                        // LAN candidate. Without this, str0m can't pair STUN
                        // checks and consent-freshness fails ~15s after Connected.
                        let destination = if source.ip().is_loopback() {
                            std::net::SocketAddr::new(
                                std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                                local_candidate_addr.port(),
                            )
                        } else {
                            local_candidate_addr
                        };
                        let receive = Receive {
                            proto: Protocol::Udp,
                            source,
                            destination,
                            contents: contents.try_into()?,
                        };
                        rtc.handle_input(Input::Receive(Instant::now(), receive))?;
                        // An inbound packet may carry SCTP acks, which is what
                        // frees send-buffer space — retry any paused write.
                        dc_write_paused = false;
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
                            view: view_ctx.view.as_deref(),
                            pinned_session_id: if view_ctx.view.as_deref() == Some("embed") {
                                view_ctx.session_id.as_deref()
                            } else {
                                None
                            },
                            peer_id,
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
                        // Unsolicited push (e.g. page_state) — chunk if large so
                        // an oversized frame can't reset the data channel.
                        enqueue_push(&mut pending_dc_writes, cid, result);
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
pub(super) struct ControlRequest {
    pub(super) request_id: Option<String>,
    pub(super) channel_id: str0m::channel::ChannelId,
    pub(super) msg_type: String,
    pub(super) body: serde_json::Value,
}


/// Get the local (non-loopback) IP address for WebRTC host candidates.
/// Connects a UDP socket to a public address to determine the local IP
/// (no actual packets are sent).
pub(crate) fn get_local_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

/// One-line summary of an SDP: pulls out the fields a human actually reads
/// (ICE ufrag, setup role, host candidate). Full SDP is 20+ lines of
/// boilerplate; at DEBUG level that just spams. Use TRACE for the full body.
fn summarize_sdp(sdp: &str) -> String {
    let mut ufrag = "?";
    let mut setup = "?";
    let mut candidate = String::new();
    for line in sdp.lines() {
        if let Some(v) = line.strip_prefix("a=ice-ufrag:") {
            ufrag = v;
        } else if let Some(v) = line.strip_prefix("a=setup:") {
            setup = v;
        } else if let Some(v) = line.strip_prefix("a=candidate:") {
            // Keep the first candidate only (usually host candidate).
            if candidate.is_empty() {
                candidate = v.to_string();
            }
        }
    }
    let bytes = sdp.len();
    let lines = sdp.lines().count();
    if candidate.is_empty() {
        format!("{bytes}B/{lines}L ufrag={ufrag} setup={setup}")
    } else {
        format!("{bytes}B/{lines}L ufrag={ufrag} setup={setup} candidate={candidate}")
    }
}

