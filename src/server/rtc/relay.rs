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
use tracing::{debug, info, warn};

use crate::server::ServerState;

/// Everything a relay round needs: the one account credential plus this
/// machine's address. Rebuilt from disk each cycle, never cached to a file.
#[derive(Clone, PartialEq)]
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
    let run = move |link: Link| {
        let state = state.clone();
        tokio::spawn(async move { run_relay(link, state).await })
    };
    supervise(Link::current, run, || tokio::time::sleep(Duration::from_secs(30))).await;
}

/// One relay round on `link`: register, then heartbeat and poll for offers
/// until either stops. Both run in this one task, so stopping it stops both.
async fn run_relay(link: Link, state: Arc<ServerState>) {
    info!(
        "Remote access enabled: {} ({})",
        crate::account::instance_name(),
        link.instance_id
    );
    // Assert the machine's registration before serving. Idempotent, so this
    // both creates the row on first sign-in and refreshes a renamed host.
    match crate::account::register_instance().await {
        Ok(_) => debug!("Instance registered"),
        Err(e) => warn!("Instance registration failed: {e}"),
    }
    tokio::select! {
        _ = heartbeat_loop(&link) => {}
        _ = offer_poll_loop(&link, state) => {}
    }
    info!("Relay tasks stopped — will re-check credentials in 30s");
}

/// Run `run` on the credential `current` reads, checking it every `tick`:
/// a change of account (a switch, a sign-out) stops the running task at
/// once and a new one starts on the new credential; a task that exits is
/// started again after one tick.
async fn supervise<L, R, T, TF>(mut current: impl FnMut() -> Option<L>, mut run: R, mut tick: T)
where
    L: Clone + PartialEq,
    R: FnMut(L) -> tokio::task::JoinHandle<()>,
    T: FnMut() -> TF,
    TF: std::future::Future<Output = ()>,
{
    let mut signed_in = false;
    loop {
        let Some(link) = current() else {
            if std::mem::take(&mut signed_in) {
                info!("Signed out — remote access is off");
            }
            tick().await;
            continue;
        };
        signed_in = true;
        let mut task = run(link.clone());
        let exited = loop {
            tokio::select! {
                _ = &mut task => break true,
                _ = tick() => {
                    if current().as_ref() != Some(&link) {
                        info!("Account credential changed — stopping relay tasks");
                        task.abort();
                        let _ = (&mut task).await;
                        break false;
                    }
                }
            }
        };
        if exited {
            tick().await;
        }
    }
}

/// Send heartbeats every 5 minutes to keep the instance online.
async fn heartbeat_loop(link: &Link) {
    let client = build_relay_client();
    let url = format!("{}/api/instances/{}/heartbeat", link.base, link.instance_id);

    loop {
        match client.post(&url).bearer_auth(&link.token).send().await {
            Ok(resp) if resp.status().is_success() => {
                debug!("Heartbeat sent");
            }
            Ok(resp) if resp.status() == 401 || resp.status() == 403 => {
                warn!(
                    "Heartbeat auth rejected ({}). Will retry after credential reload.",
                    resp.status()
                );
                if resp.status() == 401 {
                    crate::account::note_token_rejected(&link.token);
                }
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
    let url = format!("{}/api/signaling/{}/offer", link.base, link.instance_id);
    let mut error_backoff = Duration::from_secs(5);

    loop {
        match client.get(&url).bearer_auth(&link.token).send().await {
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
                            let Some(user_ctx) = context_for_offer(&data, &nonce, crate::server::rooms_switch::on()) else {
                                continue;
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
                warn!(
                    "Relay auth rejected ({}). Re-run `ling login` to fix. Stopping relay.",
                    resp.status()
                );
                if resp.status() == 401 {
                    crate::account::note_token_rejected(&link.token);
                }
                return;
            }
            Ok(resp) => {
                warn!(
                    "Offer poll failed: {} (backoff {:?})",
                    resp.status(),
                    error_backoff
                );
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

/// Who an offer's peer is, by what the relay said of it; `None` for a room
/// consumer while rooms are off (`rooms_on`, the switch) or this room is.
fn context_for_offer(
    data: &serde_json::Value,
    nonce: &str,
    rooms_on: bool,
) -> Option<super::UserContext> {
    if data["pairing"].as_bool() == Some(true) {
        // A phone that scanned our QR: the relay let it through because our
        // pairing window is open and its secret hashes to ours. It still has
        // to hand that secret over on the channel to become a paired device —
        // until then it can call nothing else. See UserContext::pairing_only.
        info!("Received pairing offer (nonce: {nonce})");
        return Some(super::UserContext::pairing());
    }
    if data["paired"].as_bool() == Some(true) {
        // A device we paired, dialling on the grant we minted for it. Its
        // standing follows its account (`person::standing_of`); the relay
        // says which account the credential it sent beside the grant is.
        let vouched = vouched_by_relay(data);
        info!("Received paired-device offer (nonce: {nonce}, account vouched: {})", vouched != super::Vouched::Nobody);
        return Some(super::UserContext {
            vouched,
            ..super::UserContext::paired()
        });
    }
    if let Some(ct) = data["consumer_type"].as_str() {
        if !rooms_on {
            info!("Rejecting consumer offer ({}, nonce: {nonce})", crate::server::rooms_switch::OFF);
            return None;
        }
        // Consumer — check if room is enabled first
        let room_cfg = super::room_config::load_room_config();
        if !room_cfg.room_enabled {
            info!("Rejecting consumer offer (room disabled, nonce: {nonce})");
            return None;
        }
        // Derive permission from room_config
        let permission = super::room_config::tools_to_permission(&room_cfg.allowed_tools);
        let user_id = data["consumer_user_id"].as_str().unwrap_or("").to_string();
        info!(
            "Received proxy room offer (nonce: {nonce}, type: {ct}, perm: {:?})",
            permission
        );
        return Some(super::UserContext::consumer(
            user_id,
            ct.to_string(),
            permission,
            data["token_budget_daily"].as_i64(),
            data["room_name"].as_str().map(|s| s.to_string()),
        ));
    }
    // The owner's own account dialled: the relay checked it owns this instance.
    info!("Received remote offer (nonce: {nonce})");
    Some(super::UserContext {
        vouched: super::Vouched::Owner,
        ..super::UserContext::owner(None)
    })
}

/// What the relay vouched for on a paired offer: the account it names
/// (`vouched_account`), or — from a relay that names none — this Mac's own
/// when it says the credential is the instance owner's (`owner`). Only the
/// relay writes either.
fn vouched_by_relay(data: &serde_json::Value) -> super::Vouched {
    let named = data["vouched_account"].as_str().map(str::trim).filter(|id| !id.is_empty());
    match (named, data["owner"].as_bool() == Some(true)) {
        (Some(id), _) => super::Vouched::Account(id.to_string()),
        (None, true) => super::Vouched::Owner,
        (None, false) => super::Vouched::Nobody,
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

            let answer_url = format!("{}/api/signaling/{}/answer", link.base, link.instance_id);

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

#[cfg(test)]
mod tests {
    use super::context_for_offer;
    use serde_json::json;

    /// A paired offer's account is the one the relay names (`vouched_account`),
    /// or this Mac's own when only `owner` says so (a relay that names none);
    /// nothing a body says adds one; the owner's own offer is this Mac's
    /// account; a pairing offer has none.
    #[test]
    fn an_offer_is_vouched_only_as_the_relay_says() {
        use crate::server::rtc::Vouched::*;
        let vouched = |data: serde_json::Value| context_for_offer(&data, "n", true).unwrap().vouched;
        assert_eq!(vouched(json!({"paired": true})), Nobody);
        assert_eq!(vouched(json!({"paired": true, "owner": false})), Nobody);
        assert_eq!(vouched(json!({"paired": true, "owner": true})), Owner);
        assert_eq!(vouched(json!({"paired": true, "vouched_account": "u-friend"})), Account("u-friend".into()));
        assert_eq!(
            vouched(json!({"paired": true, "owner": true, "vouched_account": "u-owner"})),
            Account("u-owner".into())
        );
        assert_eq!(vouched(json!({"paired": true, "vouched_account": "  "})), Nobody);
        // Nothing but the relay's two fields counts.
        assert_eq!(vouched(json!({"paired": true, "account": "u-friend", "sdp": "vouched_account"})), Nobody);
        let owners = context_for_offer(&json!({"paired": true, "owner": true}), "n", true).unwrap();
        assert_eq!(owners.kind, crate::server::rtc::PeerKind::Paired);
        assert_eq!(vouched(json!({})), Owner, "the owner's own offer");
        assert_eq!(vouched(json!({"pairing": true, "owner": true, "vouched_account": "u-friend"})), Nobody);
    }

    /// What the relay vouched for is an account only with this Mac's account
    /// to name for the owner.
    #[test]
    fn a_vouch_names_an_account() {
        use crate::server::rtc::Vouched::*;
        assert_eq!(Nobody.account(Some("u-owner")), None);
        assert_eq!(Owner.account(Some("u-owner")).as_deref(), Some("u-owner"));
        assert_eq!(Owner.account(None), None, "a Mac signed out has no account to vouch");
        assert_eq!(Account("u-friend".into()).account(Some("u-owner")).as_deref(), Some("u-friend"));
    }

    /// With rooms off no room consumer gets a peer, while the owner, a
    /// paired phone and one pairing are let in exactly as before.
    #[test]
    fn a_consumer_offer_needs_rooms_on_and_no_one_else_does() {
        let consumer = json!({"consumer_type": "linggen", "consumer_user_id": "u-x"});
        assert!(context_for_offer(&consumer, "n", false).is_none());
        let on = context_for_offer(&consumer, "n", true).unwrap();
        assert_eq!(on.kind, crate::server::rtc::PeerKind::Consumer);
        for own in [json!({}), json!({"paired": true}), json!({"pairing": true})] {
            assert!(context_for_offer(&own, "n", false).is_some(), "{own}");
        }
    }
}

#[cfg(test)]
mod supervise_tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::sync::{mpsc, Notify};

    /// Sends `Stopped` when its task ends, however it ends.
    struct Ends(&'static str, mpsc::UnboundedSender<(&'static str, &'static str)>);
    impl Drop for Ends {
        fn drop(&mut self) {
            let _ = self.1.send(("stopped", self.0));
        }
    }

    async fn next(events: &mut mpsc::UnboundedReceiver<(&'static str, &'static str)>) -> (&'static str, &'static str) {
        tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("the supervisor did nothing")
            .unwrap()
    }

    /// A change of credential stops the running task at once and starts one
    /// on the new; a task that exits is started again; signing out stops it.
    #[tokio::test]
    async fn the_relay_follows_the_credential_and_restarts_an_exited_task() {
        let creds = Arc::new(Mutex::new(Some("a")));
        let exit = Arc::new(Notify::new());
        let (tx, mut events) = mpsc::unbounded_channel();
        let (tick_tx, tick_rx) = mpsc::unbounded_channel::<()>();
        let tick_rx = Arc::new(tokio::sync::Mutex::new(tick_rx));
        let current = {
            let creds = creds.clone();
            move || *creds.lock().unwrap()
        };
        let run = {
            let exit = exit.clone();
            move |link: &'static str| {
                let (tx, exit) = (tx.clone(), exit.clone());
                tokio::spawn(async move {
                    let _ends = Ends(link, tx.clone());
                    let _ = tx.send(("started", link));
                    exit.notified().await;
                })
            }
        };
        let tick = move || {
            let rx = tick_rx.clone();
            async move {
                rx.lock().await.recv().await;
            }
        };
        let supervisor = tokio::spawn(super::supervise(current, run, tick));

        assert_eq!(next(&mut events).await, ("started", "a"));
        *creds.lock().unwrap() = Some("b");
        tick_tx.send(()).unwrap();
        assert_eq!(next(&mut events).await, ("stopped", "a"), "the old account's task went on");
        assert_eq!(next(&mut events).await, ("started", "b"));

        exit.notify_one();
        assert_eq!(next(&mut events).await, ("stopped", "b"));
        tick_tx.send(()).unwrap();
        assert_eq!(next(&mut events).await, ("started", "b"), "an exited task was not restarted");

        *creds.lock().unwrap() = None;
        tick_tx.send(()).unwrap();
        assert_eq!(next(&mut events).await, ("stopped", "b"), "signed out, the task went on");
        supervisor.abort();
    }
}
