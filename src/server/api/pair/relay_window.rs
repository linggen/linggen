//! The relay pairing window: while a QR is on screen, linggen.dev will
//! broker a phone that is not on this Wi-Fi, and grants it a relay
//! credential that unpairing revokes.

use super::*;

pub(super) fn relay_instance() -> Option<String> {
    crate::account::resolve_token()?;
    crate::account::instance_id().ok()
}

/// Tell linggen.dev this Mac is showing a pairing QR, so a phone that presents
/// the matching secret may signal to us.
///
/// We register only the SHA-256 of the secret: the relay can then tell a real
/// scan from a guess without ever holding something that would let it — or
/// anyone reading its database — pair with this Mac.
pub(super) fn open_pair_window(secret: &str) {
    let Some(instance) = relay_instance() else {
        return;
    };
    let hash = sha256_hex(secret);
    tokio::spawn(async move {
        post_pair_window(&instance, Some(hash)).await;
    });
}

/// Shut the window: the QR is no longer in front of anybody.
pub(super) fn close_pair_window() {
    let Some(instance) = relay_instance() else {
        return;
    };
    tokio::spawn(async move {
        post_pair_window(&instance, None).await;
    });
}

pub(super) async fn post_pair_window(instance: &str, hash: Option<String>) {
    let Some((token, _)) = crate::account::resolve_token() else {
        return;
    };
    let url = format!(
        "{}/api/instances/{}/pair-window",
        crate::account::site_url(),
        instance
    );
    let open = hash.is_some();
    let body = serde_json::json!({ "pair_hash": hash });
    match reqwest::Client::new()
        .post(&url)
        .bearer_auth(token)
        .json(&body)
        .timeout(Duration::from_secs(10))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            tracing::info!(
                "[pair] relay pairing window {}",
                if open { "open" } else { "closed" }
            );
        }
        Ok(r) => tracing::warn!("[pair] pairing window update failed: {}", r.status()),
        Err(e) => tracing::warn!("[pair] pairing window update error: {e}"),
    }
}

pub(super) fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Ask linggen.dev for a relay credential belonging to a phone we have just
/// paired, so it can reach us from outside this network without an account.
///
/// Pairing deliberately does not require the phone to sign in — whoever stood
/// at this screen decides who gets in. But the relay authorized account
/// holders only, so such a phone could pair from a hotel and then never reach
/// us again. We are signed in by definition (no account, no relay presence,
/// nothing to pair over), so we vouch, and the credential is scoped to this
/// Mac alone. `None` when signed out, which is also when there is no relay to
/// reach us on.
/// Issue a relay credential for a freshly paired phone and remember it on that
/// phone's row, so Revoke can take it back.
pub(super) async fn grant_for(device: &PairedDevice) -> Option<String> {
    // A re-pair replaced this phone's row; the credential the old row carried
    // is now unreachable and would outlive it on the relay.
    if let Some(stale) = device.superseded_grant.clone() {
        revoke_relay_grant(&stale).await;
    }
    let grant = mint_relay_grant(&device.name).await?;
    let stored = modify_devices(|devices| {
        if let Some(d) = devices.iter_mut().find(|d| d.id == device.id) {
            d.relay_grant = Some(grant.clone());
        }
    });
    if let Err(e) = stored {
        tracing::warn!(
            "[pair] could not remember relay grant for {}: {e}",
            device.id
        );
    }
    Some(grant)
}

/// Ask the relay to forget a phone's credential. Best effort by nature — the
/// Mac may be offline when the user revokes — so the row is dropped either
/// way and the credential is re-asserted from `paired-devices.json` on the
/// next heartbeat rather than trusted to one call.
pub(super) async fn revoke_relay_grant(grant: &str) {
    let Some(instance) = relay_instance() else {
        return;
    };
    let Some((token, _)) = crate::account::resolve_token() else {
        return;
    };
    let url = format!(
        "{}/api/instances/{}/device-grants",
        crate::account::site_url(),
        instance
    );
    match reqwest::Client::new()
        .delete(&url)
        .bearer_auth(token)
        .json(&serde_json::json!({ "grant": grant }))
        .timeout(Duration::from_secs(10))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => tracing::info!("[pair] relay credential revoked"),
        Ok(r) => tracing::warn!("[pair] relay revoke failed: {}", r.status()),
        Err(e) => tracing::warn!("[pair] relay revoke error: {e}"),
    }
}

pub(super) async fn mint_relay_grant(device_name: &str) -> Option<String> {
    let Some(instance) = relay_instance() else {
        // Pairing still succeeds — but say why the device will be LAN-only,
        // or "works on Wi-Fi, dies off it" reads as a mystery, not a state.
        tracing::info!(
            "[pair] no relay grant for '{device_name}': this Mac is not signed in \
             to linggen.dev, so the device can reach it on the local network only"
        );
        return None;
    };
    let (token, _) = crate::account::resolve_token()?;
    let url = format!(
        "{}/api/instances/{}/device-grants",
        crate::account::site_url(),
        instance
    );
    let res = reqwest::Client::new()
        .post(&url)
        .bearer_auth(token)
        .json(&serde_json::json!({ "device_label": device_name }))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        tracing::warn!("[pair] relay grant refused: {}", res.status());
        return None;
    }
    let body: serde_json::Value = res.json().await.ok()?;
    let grant = body.get("grant")?.as_str()?.to_string();
    tracing::info!("[pair] minted a relay grant for '{device_name}'");
    Some(grant)
}

/// The pair surface says it is still on screen. Extends the window; a surface
/// that goes quiet lets [`pair_window_watch`] shut it.
pub(crate) async fn post_pair_window_keepalive() -> impl IntoResponse {
    let alive = {
        let mut pending = QR_PENDING.lock_ok();
        match pending.as_mut() {
            Some(p) => {
                p.seen = std::time::Instant::now();
                true
            }
            None => false,
        }
    };
    if !alive {
        return err(StatusCode::GONE, "no QR is being shown");
    }
    // The surface compares this with what it drew; a change means the code it
    // is showing has been spent and a new one is waiting.
    Json(serde_json::json!({ "ok": true, "generation": qr_generation() })).into_response()
}

/// Close the pairing window once the surface showing the QR goes away.
///
/// This is what keeps "someone was standing at that Mac" literally true. While
/// the QR only carried a LAN address a photograph of it was nearly harmless —
/// you would also have to be on the network. Carrying relay coordinates makes
/// that same photograph work from anywhere, so the code has to stop working
/// when the screen showing it does.
pub fn pair_window_watch() {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let expired = {
                let mut pending = QR_PENDING.lock_ok();
                match pending.as_ref() {
                    Some(p) if p.seen.elapsed() > QR_IDLE_SHUT => {
                        *pending = None;
                        true
                    }
                    _ => false,
                }
            };
            if expired {
                tracing::info!("[pair] QR surface closed — pairing window shut");
                close_pair_window();
            }
        }
    });
}
