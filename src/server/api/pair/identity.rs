//! Who is calling, and who this Mac is: the device behind a request, the
//! Mac's own name and address (Bonjour itself lives in `bonjour`).

use super::*;

/// The tunnel's headers (`state_fs::devices`): which device a re-issued
/// request is for, and what the tunnel knows of a peer that named none.
pub use crate::state_fs::devices::{
    ACTOR_DEVICE_HEADER, TUNNEL_HEADER, TUNNEL_LOCAL, TUNNEL_OWNER,
};

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
    crate::memory_host::caller::token_from_headers(headers)
}
