//! Who is calling, and who this Mac is: the device behind a request, the
//! Mac's own name and address (Bonjour itself lives in `bonjour`).

use super::*;

/// Header the WebRTC tunnel stamps on a request it forwards for an identified
/// peer. The phone speaks only WebRTC, so its calls reach the router from
/// loopback with no device token — this is how a handler still knows which
/// phone is asking.
///
/// Trust: only the tunnel writes it, and anything else able to reach loopback
/// is already the Mac's owner. It routes and attributes; it never authorizes.
pub const ACTOR_DEVICE_HEADER: &str = "x-linggen-actor-device";

/// Header the WebRTC tunnel stamps on EVERY request it re-issues to loopback,
/// identified peer or not. Loopback alone does not mean "someone at this Mac"
/// once the tunnel is involved; a handler that needs that (approving a machine
/// in `ling_mem_connect`) refuses anything carrying this.
pub const TUNNEL_HEADER: &str = "x-linggen-tunnel";

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
