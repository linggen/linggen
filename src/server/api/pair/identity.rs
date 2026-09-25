//! Who is calling, and who this Mac is: the device behind a request, the
//! Mac's own name and address, and its Bonjour announcement.

use super::*;

/// Header the WebRTC tunnel stamps on a request it forwards for an identified
/// peer. The phone speaks only WebRTC, so its calls reach the router from
/// loopback with no device token — this is how a handler still knows which
/// phone is asking.
///
/// Trust: only the tunnel writes it, and anything else able to reach loopback
/// is already the Mac's owner. It routes and attributes; it never authorizes.
pub const ACTOR_DEVICE_HEADER: &str = "x-linggen-actor-device";

/// Same as [`actor_for_token`] for an HTTP caller carrying the device token.
pub fn actor_for_headers(headers: &axum::http::HeaderMap) -> Option<Actor> {
    if let Some(a) = device_token_from_headers(headers).and_then(|t| actor_for_token(&t)) {
        return Some(a);
    }
    let id = headers.get(ACTOR_DEVICE_HEADER)?.to_str().ok()?;
    load_devices()
        .into_iter()
        .find(|d| d.id == id)
        .map(|d| Actor {
            device: d.id,
            account: d.account.map(|a| a.id),
        })
}

/// The paired device behind a request, whole — for a handler that needs more
/// than the actor's ids (the device's name, the account's display name).
/// Same two doors as [`actor_for_headers`]: the device token, or the id the
/// WebRTC tunnel stamps for a peer it already identified.
pub fn device_for_headers(headers: &axum::http::HeaderMap) -> Option<PairedDevice> {
    if let Some(d) = device_token_from_headers(headers).and_then(|t| device_by_token(&t)) {
        return Some(d);
    }
    let id = headers.get(ACTOR_DEVICE_HEADER)?.to_str().ok()?;
    load_devices().into_iter().find(|d| d.id == id)
}

/// Which paired device is calling, if any — the id used to scope per-phone
/// state like the delete queue.
pub fn caller_device(headers: &axum::http::HeaderMap) -> Option<String> {
    actor_for_headers(headers).map(|a| a.device)
}

pub(super) fn mac_identity() -> (String, Option<String>) {
    let mac_name = local_host_name().unwrap_or_else(|| "Mac".to_string());
    let account = crate::account::load_account().and_then(|a| a.user_name);
    (mac_name, account)
}

pub(super) fn local_host_name() -> Option<String> {
    let out = std::process::Command::new("scutil")
        .args(["--get", "LocalHostName"])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

// ---------------------------------------------------------------------------
// Bonjour — the daemon announces itself so phones list nearby Macs by name.
// ---------------------------------------------------------------------------

/// Advertise `_linggen._tcp` on the LAN. The phone's pair sheet browses for
/// this and shows "This-Mac · linggen" entries — no addresses typed, and in
/// an office of Linggens each entry is named. The responder lives in the
/// daemon process, so it dies (and the record expires) with it. Loopback-only
/// binds skip it: nothing to discover that the LAN can reach.
pub fn advertise(port: u16, lan_bound: bool) {
    if !lan_bound {
        return;
    }
    let (mac_name, account) = mac_identity();
    let result = mdns_sd::ServiceDaemon::new().and_then(|daemon| {
        // The SRV target must be a host label WE own, never the Mac's own
        // `.local` name. enable_addr_auto() publishes A records for whatever
        // host we name here; naming it the OS hostname makes macOS see a
        // second responder claiming its own name and defensively rename the
        // computer (This-Mac → This-Mac-2 → …) on every daemon start. A
        // linggen-scoped label carries our addresses without touching the name
        // the OS owns. Resolvers read the IPs straight from the service info,
        // so the label never needs to be human-meaningful.
        let host = format!("linggen-{}.local.", mac_name.to_lowercase());
        let mut info = mdns_sd::ServiceInfo::new(
            "_linggen._tcp.local.",
            &mac_name,
            &host,
            (),
            port,
            &[
                ("name", mac_name.as_str()),
                ("account", account.as_deref().unwrap_or("")),
            ][..],
        )?
        .enable_addr_auto();
        info.set_requires_probe(false);
        daemon.register(info)?;
        // Leak the daemon handle — it must outlive this fn (daemon lifetime).
        std::mem::forget(daemon);
        Ok(())
    });
    match result {
        Ok(()) => {
            tracing::info!("[bonjour] advertising _linggen._tcp as '{mac_name}' on port {port}")
        }
        Err(e) => tracing::warn!("[bonjour] advertise failed: {e}"),
    }
}

/// The device token a phone presents on every LAN call — `x-linggen-device`
/// or `Authorization: Bearer`.
pub(super) fn device_token_from_headers(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Some(t) = headers
        .get("x-linggen-device")
        .and_then(|v| v.to_str().ok())
    {
        return Some(t.to_string());
    }
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.to_string())
}
