//! Who is connected right now — in memory, never on disk.
//!
//! `lan_gate` stamps a paired device each time its token gets it in: when,
//! from which ip, and whether it was a replica's pull (`/api/sync/*`). The
//! console's devices pill reads it back through `/api/connect/devices`.
//! A restart forgets everything; a replica pulls every ~30s, so it is back
//! within a minute. Requests from this machine are never stamped — the CLI,
//! the engine and the local app are not devices.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Seen within this long = online. A replica pulls every ~30s.
pub const ONLINE_WINDOW: Duration = Duration::from_secs(120);

/// A device is a replica if its last pull came this close to its last
/// contact. Measured against the last contact, not now, so a replica that
/// goes offline still reads as a replica.
const REPLICA_WINDOW: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seen {
    pub at: Instant,
    pub ip: IpAddr,
    pub sync_at: Option<Instant>,
}

impl Seen {
    pub fn online(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.at) <= ONLINE_WINDOW
    }

    pub fn is_replica(&self) -> bool {
        self.sync_at
            .is_some_and(|s| self.at.saturating_duration_since(s) <= REPLICA_WINDOW)
    }

    /// Seconds since this device was last seen.
    pub fn age_secs(&self, now: Instant) -> i64 {
        now.saturating_duration_since(self.at).as_secs() as i64
    }
}

fn is_sync_path(path: &str) -> bool {
    path.starts_with("/api/sync/")
}

/// Last contact per paired row id (the `id` in `paired-devices.json`).
#[derive(Default)]
pub struct Presence {
    seen: Mutex<HashMap<String, Seen>>,
}

impl Presence {
    /// One lock, one insert — cheap enough for the gate.
    pub fn record(&self, id: &str, ip: IpAddr, path: &str, now: Instant) {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let prior_sync = seen.get(id).and_then(|s| s.sync_at);
        let sync_at = if is_sync_path(path) {
            Some(now)
        } else {
            prior_sync
        };
        seen.insert(
            id.to_string(),
            Seen {
                at: now,
                ip,
                sync_at,
            },
        );
    }

    pub fn get(&self, id: &str) -> Option<Seen> {
        let seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.get(id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lan() -> IpAddr {
        "192.168.20.242".parse().unwrap()
    }

    #[test]
    fn an_unseen_device_has_no_presence() {
        assert!(Presence::default().get("a").is_none());
    }

    #[test]
    fn online_means_seen_within_two_minutes() {
        let p = Presence::default();
        let t0 = Instant::now();
        p.record("a", lan(), "/mcp", t0);
        let s = p.get("a").unwrap();
        assert_eq!(s.ip, lan());
        assert!(s.online(t0));
        assert!(s.online(t0 + Duration::from_secs(120)));
        assert!(!s.online(t0 + Duration::from_secs(121)));
        assert_eq!(s.age_secs(t0 + Duration::from_secs(300)), 300);
    }

    #[test]
    fn a_device_that_pulls_changes_is_a_replica() {
        let p = Presence::default();
        let t0 = Instant::now();
        p.record("a", lan(), "/mcp", t0);
        assert!(!p.get("a").unwrap().is_replica(), "a client until it pulls");
        p.record(
            "a",
            lan(),
            "/api/sync/changes",
            t0 + Duration::from_secs(30),
        );
        p.record("a", lan(), "/api/memory/add", t0 + Duration::from_secs(40));
        let s = p.get("a").unwrap();
        assert!(s.is_replica(), "a forwarded write keeps it a replica");
        assert!(!s.online(t0 + Duration::from_secs(3600)));
        assert!(s.is_replica(), "still a replica once offline");

        p.record("a", lan(), "/mcp", t0 + Duration::from_secs(40 + 601));
        assert!(!p.get("a").unwrap().is_replica(), "stopped pulling");
    }
}
