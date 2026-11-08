//! Bonjour — the daemon announces itself so phones list nearby Macs by name.
//!
//! The announcement is kept alive, not fired once: a responder can go mute
//! inside a long-running daemon (2026-11-07: a 16-hour-old engine still had
//! its mdns thread parked in `kevent`, sockets open, yet answered nothing,
//! while a fresh responder on the same Mac was found at once). So a watcher
//! looks for this Mac's own record from a second, short-lived responder every
//! few minutes and re-announces from scratch when it stays missing.

use std::time::Duration;

const SERVICE_TYPE: &str = "_linggen._tcp.local.";
/// How often the watcher looks for this Mac's own record.
const SELF_CHECK_EVERY: Duration = Duration::from_secs(180);
/// How long one look waits for an answer.
const SELF_CHECK_WAIT: Duration = Duration::from_secs(5);
/// Misses in a row before re-announcing — one miss may be a dropped packet.
const MISSES_BEFORE_READVERTISE: u32 = 2;

/// Whether a bind address can be reached from the LAN, so worth announcing.
/// Loopback-only binds skip Bonjour: nothing to discover that the LAN can reach.
pub fn is_lan_bound(host: &str) -> bool {
    !matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]")
}

/// Advertise `_linggen._tcp` on the LAN and keep it advertised. The phone's
/// pair sheet browses for this and shows "This-Mac · linggen" entries — no
/// addresses typed, and in an office of Linggens each entry is named. The
/// responder lives in the daemon process, so it dies (and the record
/// expires) with it.
pub fn advertise(port: u16, lan_bound: bool) {
    if !lan_bound {
        return;
    }
    let first = announce(port);
    let spawned = std::thread::Builder::new()
        .name("bonjour-watch".into())
        .spawn(move || watch(port, first));
    if let Err(e) = spawned {
        tracing::warn!("[bonjour] self-check not started: {e}");
    }
}

/// One live announcement: the responder and the record it answers for.
struct Announcement {
    daemon: mdns_sd::ServiceDaemon,
    fullname: String,
}

/// Start a responder announcing this Mac. Logs either way.
fn announce(port: u16) -> Option<Announcement> {
    let (mac_name, account) = super::identity::mac_identity();
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
            SERVICE_TYPE,
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
        let fullname = info.get_fullname().to_string();
        daemon.register(info)?;
        Ok(Announcement { daemon, fullname })
    });
    match result {
        Ok(a) => {
            tracing::info!("[bonjour] advertising _linggen._tcp as '{mac_name}' on port {port}");
            Some(a)
        }
        Err(e) => {
            tracing::warn!("[bonjour] advertise failed: {e}");
            None
        }
    }
}

/// What the watcher does after one look.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Keep the current responder; carry this many misses forward.
    Keep(u32),
    /// Tear it down and announce afresh.
    Readvertise,
}

/// The watcher's decision: a sighting clears the misses; enough misses in a
/// row — or no responder at all — means announce again.
fn judge(misses: u32, seen: bool, announced: bool) -> Verdict {
    if !announced {
        return Verdict::Readvertise;
    }
    if seen {
        return Verdict::Keep(0);
    }
    let misses = misses + 1;
    if misses >= MISSES_BEFORE_READVERTISE {
        Verdict::Readvertise
    } else {
        Verdict::Keep(misses)
    }
}

/// Runs for the daemon's lifetime: look, judge, re-announce when mute.
fn watch(port: u16, mut current: Option<Announcement>) {
    let mut misses = 0;
    loop {
        std::thread::sleep(SELF_CHECK_EVERY);
        let seen = current
            .as_ref()
            .is_some_and(|a| visible_on_lan(&a.fullname));
        match judge(misses, seen, current.is_some()) {
            Verdict::Keep(m) => misses = m,
            Verdict::Readvertise => {
                tracing::warn!("[bonjour] re-advertising: own record not seen on the LAN");
                if let Some(old) = current.take() {
                    let _ = old.daemon.shutdown();
                }
                current = announce(port);
                misses = 0;
            }
        }
    }
}

/// Whether a separate responder, listening only on real (non-loopback)
/// interfaces, hears `fullname` answer within `SELF_CHECK_WAIT`. A separate
/// one, because a responder that went deaf can't notice it from inside, and
/// its cache starts empty — a hit means a live answer.
fn visible_on_lan(fullname: &str) -> bool {
    let Ok(probe) = mdns_sd::ServiceDaemon::new() else {
        return true; // can't look: don't churn the announcement over it
    };
    let _ = probe.disable_interface(vec![
        mdns_sd::IfKind::LoopbackV4,
        mdns_sd::IfKind::LoopbackV6,
    ]);
    let seen = probe.browse(SERVICE_TYPE).is_ok_and(|rx| {
        let deadline = std::time::Instant::now() + SELF_CHECK_WAIT;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(left) {
                Ok(mdns_sd::ServiceEvent::ServiceResolved(s)) if s.fullname == fullname => {
                    break true;
                }
                Ok(mdns_sd::ServiceEvent::ServiceFound(_, name)) if name == fullname => {
                    break true;
                }
                Ok(_) => continue,
                Err(_) => break false,
            }
        }
    });
    let _ = probe.shutdown();
    seen
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

    #[test]
    fn a_sighting_clears_misses() {
        assert_eq!(judge(1, true, true), Verdict::Keep(0));
    }

    #[test]
    fn one_miss_is_tolerated_two_re_announce() {
        assert_eq!(judge(0, false, true), Verdict::Keep(1));
        assert_eq!(judge(1, false, true), Verdict::Readvertise);
    }

    #[test]
    fn a_failed_first_announce_is_retried() {
        assert_eq!(judge(0, false, false), Verdict::Readvertise);
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

    /// Real network round trip: announce, then find it from a second
    /// responder. Needs a LAN interface with multicast; run by hand.
    #[test]
    #[ignore]
    fn announce_is_visible_on_lan() {
        let a = announce(9).expect("announce");
        std::thread::sleep(Duration::from_secs(1));
        assert!(visible_on_lan(&a.fullname));
        let _ = a.daemon.shutdown();
    }
}
