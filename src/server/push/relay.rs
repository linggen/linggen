//! The Mac's side of linggen.dev's push relay: one sealed note for one
//! phone, under this Mac's account token. The relay checks that phone is on
//! this account and forwards; it never reads the note.

use std::time::Duration;

/// What the relay said, reduced to what the Mac does next.
#[derive(Debug, PartialEq)]
pub(crate) enum Sent {
    Delivered,
    /// The phone is gone (APNs Unregistered, or unpaired): drop its token.
    Gone,
    /// Not now: signed out, no key yet, rate-limited, APNs refused. The
    /// code names which, for the log and the settings line.
    Later(String),
}

pub(crate) struct Ring<'a> {
    pub token: &'a str,
    pub sealed: &'a str,
    pub collapse_id: &'a str,
    pub priority: u8,
    pub expires_at: u64,
    pub fallback: &'a str,
}

pub(crate) async fn send(ring: &Ring<'_>) -> Sent {
    let Some((bearer, _)) = crate::account::resolve_token() else {
        return Sent::Later("signed_out".into());
    };
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => return Sent::Later(format!("client: {e}")),
    };
    let body = serde_json::json!({
        "token": ring.token,
        "sealed": ring.sealed,
        "collapse_id": ring.collapse_id,
        "priority": ring.priority,
        "expires_at": ring.expires_at,
        "fallback": ring.fallback,
    });
    let res = client
        .post(format!("{}/api/push/send", crate::account::site_url()))
        .bearer_auth(bearer)
        .json(&body)
        .send()
        .await;
    match res {
        Ok(r) => {
            let status = r.status().as_u16();
            let v: serde_json::Value = r.json().await.unwrap_or_default();
            outcome(status, &v)
        }
        Err(e) => Sent::Later(format!("unreachable: {e}")),
    }
}

/// The relay's answer (doc/phone-push-spec.md, "Mac → relay").
pub(crate) fn outcome(status: u16, body: &serde_json::Value) -> Sent {
    let code = body.get("code").and_then(|c| c.as_str()).unwrap_or("");
    match status {
        200 => Sent::Delivered,
        410 => Sent::Gone,
        _ if code.is_empty() => Sent::Later(format!("http_{status}")),
        _ => {
            let reason = body.get("reason").and_then(|r| r.as_str());
            Sent::Later(reason.map_or(code.to_string(), |r| format!("{code}: {r}")))
        }
    }
}

/// A collapse id that names nothing: the told key's hash. A line sent again
/// replaces itself on the lock screen.
pub(crate) fn collapse_id(told: &str) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(told.as_bytes());
    h.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_relays_answers_map_to_what_the_mac_does() {
        assert_eq!(outcome(200, &json!({"ok": true})), Sent::Delivered);
        assert_eq!(outcome(410, &json!({"code": "gone"})), Sent::Gone);
        assert_eq!(
            outcome(503, &json!({"code": "not_configured"})),
            Sent::Later("not_configured".into())
        );
        assert_eq!(
            outcome(
                502,
                &json!({"code": "apns_error", "reason": "TopicDisallowed"})
            ),
            Sent::Later("apns_error: TopicDisallowed".into())
        );
        assert_eq!(outcome(500, &json!({})), Sent::Later("http_500".into()));
    }

    #[test]
    fn the_collapse_id_is_sixteen_hex_and_stable() {
        let a = collapse_id("cfo.watch:watch|2026-10-27");
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(a, collapse_id("cfo.watch:watch|2026-10-27"));
        assert_ne!(a, collapse_id("cfo.watch:watch|2026-10-28"));
    }
}
