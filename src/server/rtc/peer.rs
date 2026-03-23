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
    create_peer_inner(offer_sdp, state, false).await
}

/// Create a peer for a remote offer (via signaling relay).
/// Binds to 0.0.0.0 so STUN can discover the public address.
pub async fn create_remote_peer(offer_sdp: String, state: Arc<ServerState>) -> Result<String> {
    create_peer_inner(offer_sdp, state, true).await
}

async fn create_peer_inner(offer_sdp: String, state: Arc<ServerState>, remote: bool) -> Result<String> {
    // Parse the SDP offer (raw SDP text from WHIP POST body)
    let offer = SdpOffer::from_sdp_string(&offer_sdp)
        .context("Failed to parse SDP offer")?;

    // Bind a UDP socket for this peer connection.
    // Local: 127.0.0.1 (fast, no STUN needed)
    // Remote: 0.0.0.0 (allows STUN to discover public IP)
    let bind_addr = if remote { "0.0.0.0:0" } else { "127.0.0.1:0" };
    let socket = UdpSocket::bind(bind_addr)
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

    // Add local candidate (the UDP socket address).
    // For remote peers bound to 0.0.0.0, resolve the actual local IP.
    let candidate_addr = if remote {
        let local_ip = get_local_ip().unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
        std::net::SocketAddr::new(local_ip, local_addr.port())
    } else {
        local_addr
    };
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
        if let Err(e) = run_peer(rtc, socket, candidate_addr, state, events_rx).await {
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
) -> Result<()> {
    let mut buf = vec![0u8; 2000];
    let mut control_channel_id = None;
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
    // Queue of pending data channel writes — drained one per poll_output cycle
    // to avoid overflowing str0m's SCTP send buffer with large chunked responses.
    let mut pending_dc_writes: std::collections::VecDeque<(str0m::channel::ChannelId, String)> = std::collections::VecDeque::new();
    let mut dc_write_paused = false; // true when last write was rejected (buffer full)

    loop {
        // Drain one pending write — but only if not paused (waiting for buffer to flush)
        if !dc_write_paused {
            if let Some((cid, msg)) = pending_dc_writes.pop_front() {
                let msg_len = msg.len();
                let written = rtc.channel(cid)
                    .map(|mut ch| ch.write(false, msg.as_bytes()))
                    .unwrap_or(Ok(false));
                match written {
                    Ok(true) => {
                        tracing::debug!("DC write ok: {}bytes, {} remaining", msg_len, pending_dc_writes.len());
                    }
                    other => {
                        tracing::debug!("DC write blocked: {}bytes, result={:?}, pausing", msg_len, other);
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
                    Event::IceConnectionStateChange(state) => {
                        tracing::info!("WebRTC ICE state: {state:?}");
                        // Only exit on terminal states. Disconnected is transient.
                        if matches!(state, IceConnectionState::Disconnected) {
                            // Check if peer is truly dead
                            if !rtc.is_alive() {
                                tracing::info!("WebRTC peer is no longer alive, exiting");
                                return Ok(());
                            }
                        }
                    }

                    Event::ChannelOpen(id, label) => {
                        tracing::info!("Data channel opened: {label} (id: {id:?})");
                        if label == "control" {
                            control_channel_id = Some(id);
                        } else if let Some(session_id) = label.strip_prefix("sess-") {
                            session_channels.insert(session_id.to_string(), id);
                            channel_sessions.insert(id, session_id.to_string());
                            // Flush any buffered events for this session
                            if let Some((_created, buffered)) = pending_events.remove(session_id) {
                                tracing::info!("Flushing {} buffered events for session {session_id}", buffered.len());
                                for json in buffered {
                                    if let Some(mut ch) = rtc.channel(id) {
                                        let _ = ch.write(false, json.as_bytes());
                                    }
                                }
                            }
                        }
                    }

                    Event::ChannelData(data) => {
                        let text = String::from_utf8_lossy(&data.data).to_string();
                        tracing::debug!("Data channel message on {:?}: {}bytes", data.id, text.len());
                        if Some(data.id) == control_channel_id {
                            if let Some(req) = handle_control_message(
                                &mut rtc,
                                data.id,
                                &text,
                                &state,
                                &mut session_channels,
                                &mut channel_sessions,
                            ) {
                                // Spawn async processing to avoid blocking str0m's event loop.
                                let tx = ctrl_resp_tx.clone();
                                let st = state.clone();
                                let client = http_client.clone();
                                let rid = req.request_id.clone();
                                let cid = req.channel_id;
                                tokio::spawn(async move {
                                    let result = process_control_request_async(&req, &st, &client).await;
                                    let _ = tx.send((rid, cid, result)).await;
                                });
                            }
                        } else if let Some(session_id) = channel_sessions.get(&data.id).cloned() {
                            // Spawn session message handling to avoid blocking str0m.
                            let st = state.clone();
                            let client = http_client.clone();
                            tokio::spawn(async move {
                                handle_session_message(&text, &session_id, &st, &client).await;
                            });
                        }
                    }

                    Event::ChannelClose(id) => {
                        if let Some(session_id) = channel_sessions.remove(&id) {
                            session_channels.remove(&session_id);
                            tracing::info!("Session channel closed: {session_id}");
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

        // Calculate how long to wait
        let now = Instant::now();
        let wait = if timeout > now {
            timeout - now
        } else {
            Duration::ZERO
        };

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
                if let Ok(event) = result {
                    forward_event_to_channels(
                        &mut rtc,
                        &event,
                        &session_channels,
                        control_channel_id,
                        &mut pending_events,
                        &state,
                    );
                }
            }

            // Receive async control request responses and send on data channel
            Some((rid, cid, result)) = ctrl_resp_rx.recv() => {
                if let Some(rid) = rid {
                    enqueue_response(&mut pending_dc_writes, cid, &rid, result);
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
    _state: &Arc<ServerState>,
    _session_channels: &mut HashMap<String, str0m::channel::ChannelId>,
    _channel_sessions: &mut HashMap<str0m::channel::ChannelId, String>,
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
        | "ask_user_response" => {
            // These need async processing — return as pending request
            Some(ControlRequest {
                request_id,
                channel_id,
                msg_type,
                body: msg,
            })
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
) -> serde_json::Value {
    let port = state.port;

    match req.msg_type.as_str() {
        "http_request" => {
            let method = req.body.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
            let url_path = req.body.get("url").and_then(|v| v.as_str()).unwrap_or("/");
            // SECURITY: validate the path to prevent SSRF attacks.
            // Allow /api/*, /assets/*, /apps/* (skills), /index.html, /logo.svg
            // for tunnel-loaded remote UI and lazy-loaded chunks.
            let path_ok = url_path.starts_with("/api/")
                || url_path.starts_with("/assets/")
                || url_path.starts_with("/apps/")
                || url_path == "/index.html"
                || url_path == "/logo.svg";
            if !path_ok || url_path.contains('@') || url_path.contains("://") || url_path.contains("..") {
                return serde_json::json!({ "error": "Invalid URL path" });
            }
            let url = format!("http://127.0.0.1:{port}{url_path}");
            tracing::info!("RTC http_request: {method} {url_path}");
            let resp = if method == "POST" {
                client.post(&url).json(&req.body.get("body").unwrap_or(&serde_json::Value::Null)).send().await
            } else {
                client.get(&url).send().await
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
            let url = format!("http://127.0.0.1:{port}{endpoint}");
            match client.post(&url).json(&req.body).send().await {
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

/// Handle a message on a session data channel (chat, plan actions, etc.).
///
/// Proxies to the local HTTP API to reuse existing handler logic.
/// This avoids duplicating complex chat/plan/ask-user code.
async fn handle_session_message(
    text: &str,
    session_id: &str,
    state: &Arc<ServerState>,
    client: &reqwest::Client,
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
        "plan_edit" => ("/api/plan/edit", msg.clone()),
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
    tracing::info!("RTC session proxy: {msg_type} → {endpoint} body={body}");
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

/// Send a control channel response, chunking if it exceeds data channel message size limits.
/// WebRTC data channels negotiate max-message-size (typically 256 KB). Large responses
/// (e.g., file contents for tunnel loading) are split into chunks that the client reassembles.
const MAX_DC_MESSAGE: usize = 16_000; // ~16 KB — small enough for SCTP congestion window

fn enqueue_response(
    queue: &mut std::collections::VecDeque<(str0m::channel::ChannelId, String)>,
    cid: str0m::channel::ChannelId,
    request_id: &str,
    result: serde_json::Value,
) {
    let body = result.get("data")
        .and_then(|d| d.get("body"))
        .and_then(|b| b.as_str());

    if let Some(body_str) = body {
        tracing::debug!("Response for {request_id}: body_len={}", body_str.len());
        if body_str.len() > MAX_DC_MESSAGE {
            let status = result.get("data")
                .and_then(|d| d.get("status"))
                .and_then(|s| s.as_u64())
                .unwrap_or(200);
            let chunks = split_utf8_safe(body_str, MAX_DC_MESSAGE);
            let total = chunks.len();

            for (i, chunk) in chunks.iter().enumerate() {
                let msg = serde_json::json!({
                    "request_id": request_id,
                    "chunk": { "index": i, "total": total, "data": chunk }
                });
                queue.push_back((cid, msg.to_string()));
            }
            let final_msg = serde_json::json!({
                "request_id": request_id,
                "data": { "status": status, "body": "" }
            });
            queue.push_back((cid, final_msg.to_string()));
            return;
        }
    }

    let mut resp = result;
    resp["request_id"] = serde_json::Value::String(request_id.to_string());
    queue.push_back((cid, resp.to_string()));
}

/// Split a string into chunks of at most `max_bytes`, respecting UTF-8 character boundaries.
fn split_utf8_safe(s: &str, max_bytes: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < s.len() {
        let mut end = std::cmp::min(start + max_bytes, s.len());
        while end > start && !s.is_char_boundary(end) {
            end -= 1;
        }
        chunks.push(&s[start..end]);
        start = end;
    }
    chunks
}

/// Get the local (non-loopback) IP address for WebRTC host candidates.
/// Connects a UDP socket to a public address to determine the local IP
/// (no actual packets are sent).
fn get_local_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

/// Forward a server event to the appropriate data channel.
/// Max buffered events per session before the channel opens.
const MAX_PENDING_EVENTS: usize = 2000;

fn forward_event_to_channels(
    rtc: &mut Rtc,
    event: &crate::server::ServerEvent,
    session_channels: &HashMap<String, str0m::channel::ChannelId>,
    control_channel_id: Option<str0m::channel::ChannelId>,
    pending_events: &mut HashMap<String, (Instant, Vec<String>)>,
    state: &Arc<ServerState>,
) {
    // Prune expired pending event buffers (older than 60s)
    let now = Instant::now();
    pending_events.retain(|_, (created, _)| now.duration_since(*created) < Duration::from_secs(60));
    // Map the server event to a UI message (same as SSE handler)
    let seq = state.event_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let ui_msg = match crate::server::map_server_event_to_ui_message(event.clone(), seq) {
        Some(msg) => msg,
        None => return,
    };

    let json = match serde_json::to_string(&ui_msg) {
        Ok(j) => j,
        Err(_) => return,
    };

    // Route to session channel if available, buffer if channel not yet open
    match ui_msg.session_id.as_deref() {
        Some("global") | None => {
            // Global events and events without session_id go to control channel
            if let Some(cid) = control_channel_id {
                if let Some(mut ch) = rtc.channel(cid) {
                    let _ = ch.write(false, json.as_bytes());
                }
            }
        }
        Some(sid) => {
            if let Some(&cid) = session_channels.get(sid) {
                // Channel is open — send directly
                if let Some(mut ch) = rtc.channel(cid) {
                    let _ = ch.write(false, json.as_bytes());
                }
            } else {
                // Channel not yet open — buffer the event
                let (_, buf) = pending_events.entry(sid.to_string()).or_insert_with(|| (Instant::now(), Vec::new()));
                if buf.len() < MAX_PENDING_EVENTS {
                    buf.push(json);
                }
            }
        }
    }
}
