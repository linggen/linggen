//! The in-process responder (mdns-sd), for hosts without mDNSResponder.
//!
//! A watcher looks for this host's own record from a second, short-lived
//! responder every few minutes and re-announces from scratch when it stays
//! missing, so a responder gone quiet comes back.

use std::time::Duration;

/// mdns-sd wants the fully qualified type.
const SERVICE_TYPE: &str = "_linggen._tcp.local.";
/// How often the watcher looks for this host's own record.
const SELF_CHECK_EVERY: Duration = Duration::from_secs(180);
/// How long one look waits for an answer.
const SELF_CHECK_WAIT: Duration = Duration::from_secs(5);
/// Misses in a row before re-announcing — one miss may be a dropped packet.
const MISSES_BEFORE_READVERTISE: u32 = 2;

pub(super) fn advertise(port: u16) {
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

/// Start a responder announcing this host. Logs either way.
fn announce(port: u16) -> Option<Announcement> {
    let record = super::record();
    let result = mdns_sd::ServiceDaemon::new().and_then(|daemon| {
        // The SRV target must be a host label WE own, never the host's own
        // `.local` name: publishing A records for the OS's name makes the OS
        // see a second responder claiming it and rename itself.
        let host = format!("linggen-{}.local.", record.name.to_lowercase());
        let txt: Vec<(&str, &str)> = record.txt.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let mut info =
            mdns_sd::ServiceInfo::new(SERVICE_TYPE, &record.name, &host, (), port, &txt[..])?
                .enable_addr_auto();
        info.set_requires_probe(false);
        let fullname = info.get_fullname().to_string();
        daemon.register(info)?;
        Ok(Announcement { daemon, fullname })
    });
    match result {
        Ok(a) => {
            tracing::info!(
                "[bonjour] advertising _linggen._tcp as '{}' on port {port}",
                record.name
            );
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
/// interfaces, hears `fullname` answer within `SELF_CHECK_WAIT`.
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
}
