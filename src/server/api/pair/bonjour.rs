//! Bonjour — the daemon announces itself so phones list nearby Macs by name.
//!
//! On macOS the record is registered with the OS's own responder
//! (mDNSResponder, through the DNS-SD API), never sent from this process.
//! A daemon launched from Linggen.app may not send multicast at all: on
//! 2026-11-08 every send to 224.0.0.251 / ff02::fb on en0 from the engine's
//! process tree failed with `No route to host`, so the in-process responder
//! answered on loopback only — the Mac itself listed it, the LAN never did.
//! The same binary run from a terminal sent fine, which is why a fresh
//! responder "worked" while the daemon's looked mute. mDNSResponder does the
//! multicast for every app, so asking it is the one path that holds; it also
//! follows interfaces, sleep and network changes on its own.
//!
//! Elsewhere (Linux) the in-process mdns-sd responder stays.

#[cfg(target_os = "macos")]
mod apple;
#[cfg(not(target_os = "macos"))]
mod mdns;

const SERVICE_TYPE: &str = "_linggen._tcp";

/// Whether a bind address can be reached from the LAN, so worth announcing.
/// Loopback-only binds skip Bonjour: nothing to discover that the LAN can reach.
pub fn is_lan_bound(host: &str) -> bool {
    !matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]")
}

/// What the record says: the instance name phones list, and its TXT pairs.
struct Record {
    name: String,
    txt: Vec<(&'static str, String)>,
}

fn record() -> Record {
    let (mac_name, account) = super::identity::mac_identity();
    Record {
        txt: vec![
            ("name", mac_name.clone()),
            ("account", account.unwrap_or_default()),
        ],
        name: mac_name,
    }
}

/// Advertise `_linggen._tcp` on the LAN and keep it advertised. The phone's
/// pair sheet browses for this and shows "This-Mac · linggen" entries — no
/// addresses typed, and in an office of Linggens each entry is named. The
/// record lives as long as the daemon process.
pub fn advertise(port: u16, lan_bound: bool) {
    if !lan_bound {
        return;
    }
    #[cfg(target_os = "macos")]
    apple::advertise(port);
    #[cfg(not(target_os = "macos"))]
    mdns::advertise(port);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_and_lan_binds_announce_loopback_does_not() {
        assert!(is_lan_bound("0.0.0.0"));
        assert!(is_lan_bound("::"));
        assert!(is_lan_bound("192.168.2.130"));
        for loopback in ["127.0.0.1", "localhost", "::1", "[::1]"] {
            assert!(!is_lan_bound(loopback), "{loopback}");
        }
    }

    /// The startup path announces: `start_server` must route its bind host
    /// through `is_lan_bound` into `advertise`. Source-level, so a refactor
    /// that drops the call fails here instead of silently in a phone's sheet.
    #[test]
    fn server_startup_calls_advertise() {
        let src = include_str!("../../mod.rs");
        assert!(
            src.contains("api::pair::advertise(actual_port, api::pair::is_lan_bound(host))"),
            "src/server/mod.rs no longer announces Bonjour at startup"
        );
    }
}
