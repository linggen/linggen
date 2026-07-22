//! Remote relay client — heartbeat + offer polling for linggen.dev signaling.
//!
//! Driven entirely by the account credential: while signed in, the server
//! 1. registers this machine (idempotent upsert on the instance id),
//! 2. heartbeats every 5 minutes so the dashboard shows it online,
//! 3. polls for incoming SDP offers and feeds them into `create_peer()`.
//!
//! Signing in on any surface therefore brings the machine online, and signing
//! out takes it offline — there is no separate link to keep in sync.

use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn, debug};

use crate::server::ServerState;

/// Everything a relay round needs: the one account credential plus this
/// machine's address. Rebuilt from disk each cycle, never cached to a file.
#[derive(Clone)]
struct Link {
    token: String,
    instance_id: String,
    base: String,
}

impl Link {
    fn current() -> Option<Self> {
        let (token, _) = crate::account::resolve_token()?;
        Some(Self {
            token,
            instance_id: crate::account::instance_id().ok()?,
            base: crate::account::site_url(),
        })
    }
}

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

/// Supervisor: follows the account credential. Starts relay tasks when signed
/// in, restarts them when the token changes, stops when signed out.
async fn relay_supervisor(state: Arc<ServerState>) {
    let mut last_token: Option<String> = None;

    loop {
        let link = Link::current();

        match (&link, &last_token) {
            (Some(l), None) => {
                info!("Remote access enabled: {} ({})", crate::account::instance_name(), l.instance_id);
            }
            (Some(l), Some(old)) if l.token != *old => {
                info!("Account credential changed — restarting relay tasks");
            }
            (Some(_), Some(_)) => {
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }
            (None, _) => {
                if last_token.is_some() {
                    info!("Signed out — remote access is off");
                }
                last_token = None;
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }
        }

        let link = link.unwrap();
        last_token = Some(link.token.clone());

        // Assert the machine's registration before serving. Idempotent, so this
        // both creates the row on first sign-in and refreshes a renamed host.
        match crate::account::register_instance().await {
            Ok(_) => debug!("Instance registered"),
            Err(e) => warn!("Instance registration failed: {e}"),
        }

        let l1 = link.clone();
        let l2 = link.clone();
        let st = state.clone();

        let hb = tokio::spawn(async move { heartbeat_loop(&l1).await });
        let poll = tokio::spawn(async move { offer_poll_loop(&l2, st).await });

        tokio::select! {
            _ = hb => {}
            _ = poll => {}
        }

        info!("Relay tasks stopped — will re-check credentials in 30s");
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

/// Send heartbeats every 5 minutes to keep the instance online.
async fn heartbeat_loop(link: &Link) {
    let client = build_relay_client();
    let url = format!(
        "{}/api/instances/{}/heartbeat",
        link.base, link.instance_id
    );

    loop {
        match client
            .post(&url)
            .bearer_auth(&link.token)
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
async fn offer_poll_loop(link: &Link, state: Arc<ServerState>) {
    let client = build_relay_client();
    let url = format!(
        "{}/api/signaling/{}/offer",
        link.base, link.instance_id
    );
    let mut error_backoff = Duration::from_secs(5);

    loop {
        match client
            .get(&url)
            .bearer_auth(&link.token)
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
                                // Consumer — check if room is enabled first
                                let room_cfg = super::room_config::load_room_config();
                                if !room_cfg.room_enabled {
                                    info!("Rejecting consumer offer (room disabled, nonce: {nonce})");
                                    continue;
                                }
                                // Derive permission from room_config
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
                                super::UserContext::owner(crate::account::load_account().and_then(|a| a.user_id))
                            };

                            let lk = link.clone();
                            let st = state.clone();
                            let cl = client.clone();
                            tokio::spawn(async move {
                                handle_remote_offer(&lk, &st, &cl, &nonce, &sdp, user_ctx).await;
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
    link: &Link,
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
                link.base, link.instance_id
            );

            match client
                .post(&answer_url)
                .bearer_auth(&link.token)
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
