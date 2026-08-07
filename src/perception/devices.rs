//! Which of the user's devices are on the other end of a wire right now.
//!
//! A condition belongs in state, its transition belongs in the log
//! (`doc/perception-spec.md` §3): "no phone connected" is a reading the agent
//! uses to stop offering a sync, while "lost the phone at 11:03, back at 11:14"
//! is what explains why something failed while the user was away.
//!
//! Both come from here, so they can never disagree.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

/// Device id → the peers currently holding it open.
///
/// The peers, not a flag: a phone that drops and redials has two peers alive
/// for a moment, and the old one's teardown lands *after* the new one's
/// identify. Keyed by device alone, that teardown would erase a device that is
/// sitting on a live connection — and the agent would spend the rest of the
/// call telling the user to plug their phone in.
static PRESENT: Mutex<Option<HashMap<String, HashSet<u64>>>> = Mutex::new(None);

fn table() -> std::sync::MutexGuard<'static, Option<HashMap<String, HashSet<u64>>>> {
    let mut g = PRESENT.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(HashMap::new());
    }
    g
}

/// A paired device identified itself on the peer `peer`.
///
/// Idempotent: a peer that re-identifies mid-connection is already counted, and
/// re-announcing it would put a second "connect" in the log for one arrival.
/// Only the first peer of a device is an arrival — the second is the same phone.
pub fn arrived(device: &str, peer: u64) {
    let first = {
        let mut g = table();
        let peers = g.as_mut().expect("seeded").entry(device.to_string()).or_default();
        peers.insert(peer);
        peers.len() == 1
    };
    if first {
        super::activity::record("system", "system", "connect", Some(label(device)));
        remember_present();
    }
}

/// A device's peer went away. Silent when this peer was never here — teardown
/// runs for every peer, including the browser tabs that are not devices at all
/// — and silent while any other peer still holds the device open.
pub fn left(device: &str, peer: u64) {
    let last = {
        let mut g = table();
        let map = g.as_mut().expect("seeded");
        let Some(peers) = map.get_mut(device) else {
            return;
        };
        if !peers.remove(&peer) {
            return;
        }
        let empty = peers.is_empty();
        if empty {
            map.remove(device);
        }
        empty
    };
    if last {
        super::activity::record("system", "system", "disconnect", Some(label(device)));
        remember_present();
    }
}

/// Devices connected right now, by id. Ids rather than names: the caller has
/// the paired list open anyway, and looking each one up here would re-read it
/// once per device on a path that runs every turn.
pub fn present_ids() -> Vec<String> {
    let g = table();
    let mut ids: Vec<String> = g.as_ref().expect("seeded").keys().cloned().collect();
    ids.sort();
    ids
}

/// The user's name for a device, falling back to its id — an unnamed device is
/// still worth saying, and an id is at least true.
fn label(id: &str) -> String {
    crate::server::api::pair::device_name(id).unwrap_or_else(|| id.to_string())
}

// --- surviving a restart ----------------------------------------------------
//
// Presence lives in memory, so a daemon that dies holding a device cannot write
// that device's departure — the log is left with two `connect` rows and nothing
// between them, which reads as a bug in the very record meant to explain things.
//
// The fix is not to invent the missing row: nobody knows when the connection
// actually ended. It is to say the one thing that IS known, at the moment it
// becomes known — this daemon started, and something was connected to the last
// one. That turns an unexplained gap into an explained one.

fn present_path() -> PathBuf {
    super::activity::activity_dir().join("present.json")
}

/// Keep the current presence on disk, so the next daemon can tell whether the
/// previous one died mid-connection. Best-effort: an unwritable file costs an
/// explanation, never a connection.
fn remember_present() {
    let ids = present_ids();
    let dir = super::activity::activity_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if ids.is_empty() {
        let _ = std::fs::remove_file(present_path());
        return;
    }
    if let Ok(body) = serde_json::to_vec(&ids) {
        let _ = std::fs::write(present_path(), body);
    }
}

/// Called once at startup. Records that this Mac restarted **only** when the
/// previous run left a device connected — a restart nobody was connected to
/// explains nothing, and this daemon restarts often enough that logging every
/// one would bury the entries a person actually wants to read.
pub fn note_restart() {
    note_restart_at(&present_path());
}

/// The body, against a named file. Split out for the tests: they share one
/// process with the tests that arrive and leave, and a departure rewrites the
/// real `present.json` out from under them.
fn note_restart_at(path: &std::path::Path) {
    let ids: Vec<String> = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
        .unwrap_or_default();
    let _ = std::fs::remove_file(path);
    if ids.is_empty() {
        return;
    }
    let names: Vec<String> = ids.iter().map(|id| label(id)).collect();
    super::activity::record(
        "system",
        "system",
        "restart",
        Some(format!("Linggen — {} had to reconnect", names.join(", "))),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrival_is_recorded_once_and_departure_is_silent_for_strangers() {
        // A stranger leaving is not news.
        left("never-here", 1);
        arrived("test-phone", 1);
        arrived("test-phone", 1);
        assert!(present_ids().contains(&"test-phone".to_string()));
        left("test-phone", 1);
        assert!(!present_ids().contains(&"test-phone".to_string()));
    }

    #[test]
    fn a_restart_is_recorded_only_when_it_explains_something() {
        // Matched by name rather than by position: these tests share one
        // process-wide log and run in parallel, so "the newest row" belongs to
        // whichever test wrote last. The presence file is this test's own for
        // the same reason — a sibling's `left()` rewrites the shared one.
        const ORPHAN: &str = "orphaned-phone-fixture";
        let path = super::super::activity::activity_dir().join("present-restart-test.json");
        let restarts_naming_it = || {
            super::super::activity::log()
                .recent()
                .into_iter()
                .filter(|a| {
                    a.verb == "restart"
                        && a.object.as_deref().is_some_and(|o| o.contains(ORPHAN))
                })
                .count()
        };

        // Nobody was connected: nothing to explain, and this daemon restarts
        // often enough that saying so every time would bury the real entries.
        let _ = std::fs::remove_file(&path);
        note_restart_at(&path);
        assert_eq!(restarts_naming_it(), 0);

        // A device was connected when the last daemon died — the departure it
        // could not write is exactly the gap this row accounts for.
        std::fs::create_dir_all(super::super::activity::activity_dir()).unwrap();
        std::fs::write(&path, format!(r#"["{ORPHAN}"]"#)).unwrap();
        note_restart_at(&path);
        assert_eq!(restarts_naming_it(), 1);

        // And claimed exactly once — the next start has nothing left to say.
        assert!(!path.exists());
        note_restart_at(&path);
        assert_eq!(restarts_naming_it(), 1);
    }

    #[test]
    fn a_redial_survives_the_old_peers_teardown() {
        // The phone drops and comes back before the dead peer finishes dying:
        // the new peer identifies, then the old one tears down. The phone is
        // connected the whole way through, and the Mac must say so.
        arrived("redial-phone", 10);
        arrived("redial-phone", 11);
        left("redial-phone", 10);
        assert!(
            present_ids().contains(&"redial-phone".to_string()),
            "the live peer still holds it open"
        );
        left("redial-phone", 11);
        assert!(!present_ids().contains(&"redial-phone".to_string()));
    }
}
