//! Who is asking memory, and so whose rows they see — the one rule every
//! memory route on the engine's port uses (`/api/memory/<verb>`, a skill
//! page's memory capability, `/mem/*`, `/mcp/memory`).
//!
//! - **This machine** (a loopback caller that is not a phone): the owner.
//! - **An approved machine** (a `machine` row — the Allow ceremony): the
//!   owner. It is the owner's other computer and sees all of their memory.
//! - **A phone** (a `phone` row — the code-pairing ceremony): its own
//!   person, as the stamped door decides (`server::api::memory`).
//! - Anything else — no token, an unknown token, a tunnel peer that has not
//!   said which device it is — is no one, and memory refuses it.
//!
//! Off this machine the device is the token's (header, Bearer or cookie — the
//! LAN gate's own reading). On loopback the WebRTC tunnel names the device it
//! re-issues a request for (`ACTOR_DEVICE_HEADER`), and marks every request
//! it re-issues (`TUNNEL_HEADER`): a tunnelled request is the owner only when
//! the tunnel says its peer is (`TUNNEL_LOCAL`, `TUNNEL_OWNER`).

use crate::state_fs::devices::{
    load_devices, token_matches, DeviceKind, PairedDevice, ACTOR_DEVICE_HEADER, TUNNEL_HEADER,
    TUNNEL_LOCAL, TUNNEL_OWNER,
};
use axum::http::HeaderMap;
use std::net::IpAddr;

/// The caller, as memory sees it.
#[derive(Clone, Debug)]
pub enum Caller {
    /// This machine.
    Owner,
    /// The owner's other computer, approved with Allow.
    Machine(PairedDevice),
    /// A phone; it sees its own person's rows.
    Phone(PairedDevice),
    /// No one memory knows.
    Unknown,
}

impl Caller {
    /// Does this caller see all of the owner's memory?
    pub fn is_owner(&self) -> bool {
        matches!(self, Caller::Owner | Caller::Machine(_))
    }

    /// The caller a paired row is, by its kind.
    pub fn of(device: PairedDevice) -> Self {
        match device.kind() {
            DeviceKind::Machine => Caller::Machine(device),
            DeviceKind::Phone => Caller::Phone(device),
        }
    }
}

/// The caller behind a request: its socket's address and its headers.
pub fn resolve(peer: IpAddr, headers: &HeaderMap) -> Caller {
    resolve_in(peer, headers, &load_devices())
}

fn resolve_in(peer: IpAddr, headers: &HeaderMap, devices: &[PairedDevice]) -> Caller {
    let found = |hit: &dyn Fn(&PairedDevice) -> bool| {
        devices
            .iter()
            .find(|d| hit(d))
            .cloned()
            .map_or(Caller::Unknown, Caller::of)
    };
    let token = token_from_headers(headers);
    if !peer.is_loopback() {
        return match token {
            Some(t) => found(&|d| token_matches(d, &t)),
            None => Caller::Unknown,
        };
    }
    if let Some(t) = token {
        let by_token = found(&|d| token_matches(d, &t));
        if !matches!(by_token, Caller::Unknown) {
            return by_token;
        }
    }
    if let Some(id) = header(headers, ACTOR_DEVICE_HEADER) {
        return found(&|d| d.id == id);
    }
    match header(headers, TUNNEL_HEADER) {
        None => Caller::Owner,
        Some(v) if v == TUNNEL_LOCAL || v == TUNNEL_OWNER => Caller::Owner,
        Some(_) => Caller::Unknown,
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// The device token a request carries: `x-linggen-device`, a Bearer token,
/// or the `linggen_device` cookie (webviews cannot add headers to their
/// subresource requests). The LAN gate reads the same three.
pub fn token_from_headers(headers: &HeaderMap) -> Option<String> {
    header(headers, crate::state_fs::devices::DEVICE_HEADER)
        .or_else(|| {
            header(headers, axum::http::header::AUTHORIZATION.as_str())
                .and_then(|v| v.strip_prefix("Bearer "))
        })
        .or_else(|| {
            header(headers, axum::http::header::COOKIE.as_str()).and_then(|c| {
                c.split(';')
                    .map(str::trim)
                    .find_map(|kv| kv.strip_prefix("linggen_device="))
            })
        })
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_fs::devices::AccountRef;
    use axum::http::HeaderValue;

    fn device(id: &str, kind: DeviceKind, account: Option<&str>) -> PairedDevice {
        PairedDevice {
            id: id.into(),
            name: id.into(),
            secret: format!("token-{id}"),
            created_at: 0,
            kind: Some(kind),
            account: account.map(|a| AccountRef {
                id: a.into(),
                email: None,
                name: None,
                at: None,
            }),
            device_id: None,
            settings: Default::default(),
            relay_grant: None,
            superseded_grant: None,
        }
    }

    fn devices() -> Vec<PairedDevice> {
        vec![
            device("mac2", DeviceKind::Machine, None),
            device("phone", DeviceKind::Phone, Some("u-other")),
        ]
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    fn who(peer: &str, pairs: &[(&'static str, &str)]) -> Caller {
        resolve_in(peer.parse().unwrap(), &headers(pairs), &devices())
    }

    const LAN: &str = "192.168.20.31";
    const LO: &str = "127.0.0.1";

    #[test]
    fn this_machine_is_the_owner() {
        assert!(matches!(who(LO, &[]), Caller::Owner));
        assert!(matches!(who("::1", &[]), Caller::Owner));
        // A plugin on this machine that sends a token nobody issued.
        assert!(matches!(
            who(LO, &[("x-linggen-device", "stale")]),
            Caller::Owner
        ));
    }

    #[test]
    fn an_approved_machine_is_the_owner() {
        let c = who(LAN, &[("x-linggen-device", "token-mac2")]);
        assert!(matches!(&c, Caller::Machine(d) if d.id == "mac2"));
        assert!(c.is_owner());
        let bearer = who(LAN, &[("authorization", "Bearer token-mac2")]);
        assert!(bearer.is_owner());
    }

    #[test]
    fn a_phone_is_its_own_person_however_it_sends_its_token() {
        for h in [
            ("x-linggen-device", "token-phone"),
            ("authorization", "Bearer token-phone"),
            ("cookie", "a=b; linggen_device=token-phone"),
        ] {
            let c = who(LAN, &[h]);
            assert!(matches!(&c, Caller::Phone(d) if d.id == "phone"), "{h:?}");
            assert!(!c.is_owner());
        }
    }

    #[test]
    fn an_unknown_or_missing_token_off_this_machine_is_no_one() {
        assert!(matches!(who(LAN, &[]), Caller::Unknown));
        assert!(matches!(
            who(LAN, &[("x-linggen-device", "guess")]),
            Caller::Unknown
        ));
        // Off this machine the tunnel's headers mean nothing.
        assert!(matches!(
            who(
                LAN,
                &[(ACTOR_DEVICE_HEADER, "mac2"), (TUNNEL_HEADER, TUNNEL_LOCAL)]
            ),
            Caller::Unknown
        ));
    }

    /// The phone speaks WebRTC; the tunnel re-issues its call from loopback,
    /// naming the device. It stays a phone.
    #[test]
    fn a_phone_through_the_tunnel_is_still_a_phone() {
        let c = who(LO, &[(TUNNEL_HEADER, "1"), (ACTOR_DEVICE_HEADER, "phone")]);
        assert!(matches!(&c, Caller::Phone(d) if d.id == "phone"));
        // A device id nobody paired is no one, not the owner.
        assert!(matches!(
            who(LO, &[(TUNNEL_HEADER, "1"), (ACTOR_DEVICE_HEADER, "gone")]),
            Caller::Unknown
        ));
    }

    #[test]
    fn a_tunnel_peer_is_the_owner_only_when_the_tunnel_says_so() {
        assert!(matches!(
            who(LO, &[(TUNNEL_HEADER, TUNNEL_LOCAL)]),
            Caller::Owner
        ));
        assert!(matches!(
            who(LO, &[(TUNNEL_HEADER, TUNNEL_OWNER)]),
            Caller::Owner
        ));
        // A paired peer that has not said which device it is yet.
        assert!(matches!(who(LO, &[(TUNNEL_HEADER, "1")]), Caller::Unknown));
    }
}
