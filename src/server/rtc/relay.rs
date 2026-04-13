//! Remote relay client — heartbeat + offer polling for linggen.dev signaling.
//!
//! When `~/.linggen/remote.toml` exists, the server:
//! 1. Sends heartbeats every 5 minutes to keep the instance "online" on the dashboard.
//! 2. Polls for incoming SDP offers from remote browser clients.
//! 3. Feeds offers into `create_peer()` and posts answers back.

use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn, debug};

use crate::cli::login::{load_remote_config, RemoteConfig};
use crate::server::ServerState;

fn build_relay_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Start relay background tasks. Watches for remote config and auto-reloads on changes.
/// Call this after the server is ready to accept peer connections.
pub fn spawn_relay_tasks(state: Arc<ServerState>) {
    tokio::spawn(async move {
        relay_supervisor(state).await;
    });
}

/// Supervisor that starts/restarts relay tasks when remote.toml appears or changes.
async fn relay_supervisor(state: Arc<ServerState>) {
    let mut last_token: Option<String> = None;

    loop {
        let config = load_remote_config();

        match (&config, &last_token) {
            (Some(cfg), None) => {
                info!("Remote access enabled: {} ({})", cfg.instance_name, cfg.instance_id);
            }
            (Some(cfg), Some(old)) if cfg.api_token != *old => {
                info!("Remote credentials updated — restarting relay tasks");
            }
            (Some(_), Some(_)) => {
                // No change — sleep and check again
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }
            (None, _) => {
                last_token = None;
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }
        }

        let config = config.unwrap();
        last_token = Some(config.api_token.clone());

        // Run heartbeat and offer polling until auth fails or config changes
        let cfg1 = config.clone();
        let cfg2 = config.clone();
        let st = state.clone();

        let hb = tokio::spawn(async move { heartbeat_loop(&cfg1).await });
        let poll = tokio::spawn(async move { offer_poll_loop(&cfg2, st).await });

        // Wait for either task to exit (auth failure causes return)
        tokio::select! {
            _ = hb => {}
            _ = poll => {}
        }

        // One of the tasks exited (likely 401). Wait a bit, then re-check config.
        info!("Relay tasks stopped — will re-check credentials in 30s");
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

/// Send heartbeats every 5 minutes to keep the instance online.
async fn heartbeat_loop(config: &RemoteConfig) {
    let client = build_relay_client();
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
            Ok(resp) if resp.status() == 401 || resp.status() == 403 => {
                warn!("Heartbeat auth rejected ({}). Will retry after credential reload.", resp.status());
                return;
            }
            Ok(resp) => {
                warn!("Heartbeat failed: {}", resp.status());
            }
            Err(e) => {
                warn!("Heartbeat error: {}", e);
            }
        }

        tokio::time::sleep(Duration::from_secs(300)).await; // 5 minutes
    }
}

/// Poll for incoming SDP offers and create peer connections.
async fn offer_poll_loop(config: &RemoteConfig, state: Arc<ServerState>) {
    let client = build_relay_client();
    let url = format!(
        "{}/api/signaling/{}/offer",
        config.relay_url, config.instance_id
    );
    let mut error_backoff = Duration::from_secs(5);

    loop {
        match client
            .get(&url)
            .bearer_auth(&config.api_token)
            .send()
            .await
        {
            Ok(resp) if resp.status() == 204 => {
                error_backoff = Duration::from_secs(5); // reset on success
            }
            Ok(resp) if resp.status().is_success() => {
                error_backoff = Duration::from_secs(5); // reset on success
                match resp.json::<serde_json::Value>().await {
                    Ok(data) => {
                        let nonce = data["nonce"].as_str().unwrap_or("").to_string();
                        let sdp = data["sdp"].as_str().unwrap_or("").to_string();

                        if !nonce.is_empty() && !sdp.is_empty() {
                            // Build UserContext for this peer
                            let user_ctx = if let Some(ct) = data["consumer_type"].as_str() {
                                // Consumer — derive permission from room_config
                                let room_cfg = super::room_config::load_room_config();
                                let permission = super::room_config::tools_to_permission(&room_cfg.allowed_tools);
                                let user_id = data["consumer_user_id"].as_str().unwrap_or("").to_string();
                                info!("Received proxy room offer (nonce: {nonce}, type: {ct}, perm: {:?})", permission);
                                super::UserContext::consumer(
                                    user_id,
                                    ct.to_string(),
                                    permission,
                                    data["token_budget_daily"].as_i64(),
                                    data["room_name"].as_str().map(|s| s.to_string()),
                                )
                            } else {
                                // Owner — Admin permission
                                info!("Received remote offer (nonce: {nonce})");
                                super::UserContext::owner(config.user_id.clone())
                            };

                            let cfg = config.clone();
                            let st = state.clone();
                            let cl = client.clone();
                            tokio::spawn(async move {
                                handle_remote_offer(&cfg, &st, &cl, &nonce, &sdp, user_ctx).await;
                            });
                        }
                    }
                    Err(e) => {
                        warn!("Failed to parse offer response: {}", e);
                    }
                }
            }
            Ok(resp) if resp.status() == 401 || resp.status() == 403 => {
                warn!("Relay auth rejected ({}). Re-run `ling login` to fix. Stopping relay.", resp.status());
                return;
            }
            Ok(resp) => {
                warn!("Offer poll failed: {} (backoff {:?})", resp.status(), error_backoff);
                tokio::time::sleep(error_backoff).await;
                error_backoff = (error_backoff * 2).min(Duration::from_secs(120));
                continue; // skip the normal 2s poll delay
            }
            Err(e) => {
                warn!("Offer poll error: {} (backoff {:?})", e, error_backoff);
                tokio::time::sleep(error_backoff).await;
                error_backoff = (error_backoff * 2).min(Duration::from_secs(120));
                continue; // skip the normal 2s poll delay
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
    user_ctx: super::UserContext,
) {
    // Create peer connection for remote offer (binds to 0.0.0.0, ICE-lite)
    match super::peer::create_remote_peer(offer_sdp.to_string(), state.clone(), user_ctx).await {
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
