//! Outbound WebRTC client for proxy room consumers.
//!
//! When a linggen instance joins a room as a "linggen" consumer, it establishes
//! a WebRTC data channel connection to the room owner's linggen server.
//! The data channel ("inference") is used to send model inference requests
//! and receive streaming token responses.
//!
//! This module handles the client-side WebRTC: create offer, signal via relay,
//! accept answer, run the data channel event loop.

use anyhow::{Context, Result};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::{debug, info};

use str0m::change::SdpAnswer;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};

/// A connected proxy client — holds the send/receive channels for inference.
pub struct ProxyConnection {
    /// Send inference requests (JSON) to the owner's linggen.
    pub request_tx: mpsc::Sender<String>,
    /// Receive inference responses (JSON) from the owner's linggen.
    pub response_rx: mpsc::Receiver<String>,
}

/// Establish a WebRTC connection to a room owner's linggen via the relay.
///
/// Steps:
/// 1. Create str0m Rtc in client mode (not ICE-lite)
/// 2. Add "inference" data channel, generate SDP offer
/// 3. POST offer to relay signaling (as room consumer)
/// 4. Poll for SDP answer from owner
/// 5. Accept answer, run the peer connection event loop
///
/// Returns a ProxyConnection for sending/receiving inference messages.
pub async fn connect_to_room(
    relay_url: &str,
    instance_id: &str,
    api_token: &str,
    events_tx: Option<tokio::sync::broadcast::Sender<crate::server::ServerEvent>>,
) -> Result<ProxyConnection> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    // Bind UDP socket
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let local_addr = socket.local_addr()?;
    info!("Proxy client UDP socket bound to {local_addr}");

    // Create str0m Rtc in full ICE mode (client role, not ICE-lite)
    let mut rtc = Rtc::new(Instant::now());

    // Add inference data channel and create offer
    let mut changes = rtc.sdp_api();
    let _channel_id = changes.add_channel("inference".to_string());
    let (offer, pending) = changes.apply()
        .context("Failed to create SDP offer (no changes?)")?;
    let offer_sdp = offer.to_sdp_string();

    // Add local candidate
    let local_ip = super::peer::get_local_ip()
        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    rtc.add_local_candidate(Candidate::host(
        (local_ip, local_addr.port()).into(),
        Protocol::Udp,
    )?);

    // POST offer to relay signaling (as room consumer).
    // Send raw SDP text — the relay reads it with request.text() and wraps
    // it in a JSON envelope with consumer metadata from the DB.
    let offer_url = format!("{relay_url}/api/signaling/{instance_id}/offer");

    let resp = client
        .post(&offer_url)
        .bearer_auth(api_token)
        .header("Content-Type", "application/sdp")
        .body(offer_sdp)
        .send()
        .await
        .context("Failed to post offer to relay")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Relay rejected offer: {status} {body}");
    }

    let offer_resp: serde_json::Value = resp.json().await?;
    let nonce = offer_resp["nonce"].as_str()
        .context("No nonce in offer response")?
        .to_string();
    info!("Offer posted, nonce: {nonce}");

    // Poll for answer (up to 30 seconds)
    let answer_url = format!("{relay_url}/api/signaling/{instance_id}/answer?nonce={nonce}");
    let mut answer_sdp = String::new();
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let resp = client
            .get(&answer_url)
            .bearer_auth(api_token)
            .send()
            .await?;
        if resp.status() == 204 { continue; }
        if resp.status().is_success() {
            let data: serde_json::Value = resp.json().await?;
            if let Some(sdp) = data["sdp"].as_str() {
                answer_sdp = sdp.to_string();
                break;
            }
        }
    }
    if answer_sdp.is_empty() {
        anyhow::bail!("Timed out waiting for SDP answer from owner");
    }

    // Accept the answer
    let answer = SdpAnswer::from_sdp_string(&answer_sdp)
        .context("Failed to parse SDP answer")?;
    rtc.sdp_api().accept_answer(pending, answer)
        .context("Failed to accept SDP answer")?;
    info!("SDP answer accepted, starting peer connection");

    // Create channels for communicating with the peer loop
    let (request_tx, mut request_rx) = mpsc::channel::<String>(32);
    let (response_tx, response_rx) = mpsc::channel::<String>(256);
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    let mut events_rx = events_tx.as_ref().map(|tx| tx.subscribe());
    let events_broadcast_tx = events_tx.clone();

    // Spawn the peer connection event loop
    tokio::spawn(async move {
        run_proxy_client_loop(&mut rtc, &socket, &mut request_rx, &response_tx, Some(ready_tx), &mut events_rx, events_broadcast_tx).await;
        info!("Proxy client peer connection closed");
    });

    // Wait for the inference data channel to open before returning,
    // so callers can immediately send requests without a race.
    tokio::time::timeout(Duration::from_secs(10), ready_rx)
        .await
        .map_err(|_| anyhow::anyhow!("Timed out waiting for inference channel to open"))?
        .map_err(|_| anyhow::anyhow!("Proxy client loop exited before channel opened"))?;

    Ok(ProxyConnection {
        request_tx,
        response_rx,
    })
}

/// Run the str0m event loop for the proxy client connection.
async fn run_proxy_client_loop(
    rtc: &mut Rtc,
    socket: &UdpSocket,
    request_rx: &mut mpsc::Receiver<String>,
    response_tx: &mpsc::Sender<String>,
    ready_tx: Option<tokio::sync::oneshot::Sender<()>>,
    events_rx: &mut Option<tokio::sync::broadcast::Receiver<crate::server::ServerEvent>>,
    events_broadcast_tx: Option<tokio::sync::broadcast::Sender<crate::server::ServerEvent>>,
) {
    let mut buf = vec![0u8; 65536];
    let mut inference_channel: Option<str0m::channel::ChannelId> = None;
    let mut ready_tx = ready_tx;
    // Local user_id for filtering — only forward our own room_chat events
    let local_user_id = crate::cli::login::load_remote_config()
        .and_then(|c| c.user_id)
        .unwrap_or_else(|| "__local__".to_string());
    // Track sender_ids received from the owner to avoid echoing them back
    let mut remote_sender_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    loop {
        // Drain str0m outputs (STUN, DTLS, SCTP packets)
        let timeout = match rtc.poll_output().unwrap_or(Output::Timeout(Instant::now())) {
            Output::Transmit(transmit) => {
                let _ = socket.send_to(&transmit.contents, transmit.destination).await;
                continue;
            }
            Output::Timeout(t) => t,
            Output::Event(event) => {
                match event {
                    Event::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                        info!("Proxy client: ICE disconnected");
                        return;
                    }
                    Event::ChannelOpen(id, label) => {
                        info!("Proxy client: data channel opened: {label}");
                        if label == "inference" {
                            inference_channel = Some(id);
                            if let Some(tx) = ready_tx.take() {
                                let _ = tx.send(());
                            }
                        }
                    }
                    Event::ChannelData(data) => {
                        if let Ok(text) = std::str::from_utf8(&data.data) {
                            // Check if this is a room_chat from the owner — broadcast locally
                            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(text) {
                                if msg.get("type").and_then(|v| v.as_str()) == Some("room_chat") {
                                    if let Some(chat_data) = msg.get("data") {
                                        let sender_id = chat_data.get("sender_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        // Skip echoes — don't re-broadcast our own messages
                                        if sender_id != local_user_id {
                                            remote_sender_ids.insert(sender_id.clone());
                                            let sender_name = chat_data.get("sender_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                            let avatar_url = chat_data.get("avatar_url").and_then(|v| v.as_str()).map(|s| s.to_string());
                                            let chat_text = chat_data.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                            if !chat_text.is_empty() {
                                                if let Some(ref etx) = events_broadcast_tx {
                                                    let _ = etx.send(crate::server::ServerEvent::RoomChat {
                                                        sender_id, sender_name, avatar_url, text: chat_text,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                    continue;
                                }
                            }
                            let _ = response_tx.send(text.to_string()).await;
                        }
                    }
                    Event::ChannelClose(id) => {
                        if inference_channel == Some(id) {
                            info!("Proxy client: inference channel closed");
                            return;
                        }
                    }
                    _ => {}
                }
                continue;
            }
        };

        let duration = timeout.saturating_duration_since(Instant::now());
        let sleep = tokio::time::sleep(duration.min(Duration::from_millis(100)));

        tokio::select! {
            // Receive UDP packets
            result = socket.recv_from(&mut buf) => {
                if let Ok((n, addr)) = result {
                    let receive = Receive::new(Protocol::Udp, addr, socket.local_addr().unwrap(), &buf[..n])
                        .expect("valid receive");
                    rtc.handle_input(Input::Receive(Instant::now(), receive)).expect("handle input");
                }
            }

            // Send inference requests
            Some(msg) = request_rx.recv() => {
                if let Some(cid) = inference_channel {
                    if let Some(mut ch) = rtc.channel(cid) {
                        let written = ch.write(false, msg.as_bytes());
                        info!("Inference channel write: {:?} ({}bytes)", written, msg.len());
                    } else {
                        tracing::warn!("Inference channel {:?} not found in rtc", cid);
                    }
                } else {
                    tracing::warn!("Inference channel not open yet, dropping request");
                }
            }

            // Forward local room_chat events to the owner via inference channel
            result = async {
                match events_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Ok(crate::server::ServerEvent::RoomChat { sender_id, sender_name, text, .. }) = result {
                    // Only forward messages from the local user to the owner.
                    // Skip: owner's messages (echoed back), other remote senders.
                    if sender_id != local_user_id {
                        continue;
                    }
                    if let Some(cid) = inference_channel {
                        let msg = serde_json::json!({
                            "type": "room_chat",
                            "text": text,
                            "sender_name": sender_name,
                            "sender_id": sender_id,
                        });
                        if let Some(mut ch) = rtc.channel(cid) {
                            let _ = ch.write(false, msg.to_string().as_bytes());
                        }
                    }
                }
            }

            // Timeout — drive str0m
            _ = sleep => {
                rtc.handle_input(Input::Timeout(Instant::now())).expect("handle timeout");
            }
        }
    }
}
