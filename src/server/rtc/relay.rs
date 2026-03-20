//! Remote relay client — heartbeat + offer polling for linggen.dev signaling.
//!
//! When `~/.linggen/remote.toml` exists, the server:
//! 1. Sends heartbeats every 30s to keep the instance "online" on the dashboard.
//! 2. Polls for incoming SDP offers from remote browser clients.
//! 3. Feeds offers into `create_peer()` and posts answers back.

use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn, debug};

use crate::cli::login::{load_remote_config, RemoteConfig};
use crate::server::ServerState;

/// Start relay background tasks if remote config exists.
/// Call this after the server is ready to accept peer connections.
pub fn spawn_relay_tasks(state: Arc<ServerState>) {
    let Some(config) = load_remote_config() else {
        debug!("No remote.toml found — relay tasks not started");
        return;
    };

    info!(
        "Remote access enabled: {} ({})",
        config.instance_name, config.instance_id
    );

    // Spawn heartbeat loop
    let hb_config = config.clone();
    tokio::spawn(async move {
        heartbeat_loop(&hb_config).await;
    });

    // Spawn offer polling loop
    let poll_config = config.clone();
    tokio::spawn(async move {
        offer_poll_loop(&poll_config, state).await;
    });
}

/// Send heartbeats every 30s to keep the instance online.
async fn heartbeat_loop(config: &RemoteConfig) {
    let client = reqwest::Client::new();
    let url = format!(
        "{}/api/instances/{}/heartbeat",
        config.relay_url, config.instance_id
    );

    loop {
        match client
            .post(&url)
            .bearer_auth(&config.api_token)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                debug!("Heartbeat sent");
            }
            Ok(resp) => {
                warn!("Heartbeat failed: {}", resp.status());
            }
            Err(e) => {
                warn!("Heartbeat error: {}", e);
            }
        }

        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

/// Poll for incoming SDP offers and create peer connections.
async fn offer_poll_loop(config: &RemoteConfig, state: Arc<ServerState>) {
    let client = reqwest::Client::new();
    let url = format!(
        "{}/api/signaling/{}/offer",
        config.relay_url, config.instance_id
    );

    loop {
        match client
            .get(&url)
            .bearer_auth(&config.api_token)
            .send()
            .await
        {
            Ok(resp) if resp.status() == 204 => {
                // No pending offer — wait and poll again
            }
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<serde_json::Value>().await {
                    Ok(data) => {
                        let nonce = data["nonce"].as_str().unwrap_or("").to_string();
                        let sdp = data["sdp"].as_str().unwrap_or("").to_string();

                        if !nonce.is_empty() && !sdp.is_empty() {
                            info!("Received remote offer (nonce: {nonce})");
                            handle_remote_offer(config, &state, &client, &nonce, &sdp).await;
                        }
                    }
                    Err(e) => {
                        warn!("Failed to parse offer response: {}", e);
                    }
                }
            }
            Ok(resp) => {
                warn!("Offer poll failed: {}", resp.status());
                // Back off on errors
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            Err(e) => {
                warn!("Offer poll error: {}", e);
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Process a remote SDP offer: create peer connection and post answer back.
async fn handle_remote_offer(
    config: &RemoteConfig,
    state: &Arc<ServerState>,
    client: &reqwest::Client,
    nonce: &str,
    offer_sdp: &str,
) {
    // Create peer connection for remote offer (binds to 0.0.0.0, full ICE)
    match super::peer::create_remote_peer(offer_sdp.to_string(), state.clone()).await {
        Ok(answer_sdp) => {
            info!("Created peer connection for remote offer, posting answer");

            let answer_url = format!(
                "{}/api/signaling/{}/answer",
                config.relay_url, config.instance_id
            );

            match client
                .post(&answer_url)
                .bearer_auth(&config.api_token)
                .json(&serde_json::json!({
                    "nonce": nonce,
                    "sdp": answer_sdp,
                }))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    info!("Answer posted for nonce {nonce}");
                }
                Ok(resp) => {
                    warn!("Failed to post answer: {}", resp.status());
                }
                Err(e) => {
                    warn!("Error posting answer: {}", e);
                }
            }
        }
        Err(e) => {
            warn!("Failed to create peer for remote offer: {}", e);
        }
    }
}
