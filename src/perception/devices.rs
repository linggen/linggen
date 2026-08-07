//! Which of the user's devices are on the other end of a wire right now.
//!
//! A condition belongs in state, its transition belongs in the log
//! (`doc/perception-spec.md` §3): "no phone connected" is a reading the agent
//! uses to stop offering a sync, while "lost the phone at 11:03, back at 11:14"
//! is what explains why something failed while the user was away.
//!
//! Both come from here, so they can never disagree.

use std::collections::HashMap;
use std::sync::Mutex;

static PRESENT: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

fn table() -> std::sync::MutexGuard<'static, Option<HashMap<String, u64>>> {
    let mut g = PRESENT.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(HashMap::new());
    }
    g
}

/// A paired device identified itself on a fresh connection.
///
/// Idempotent: a peer that re-identifies mid-connection is already here, and
/// re-announcing it would put a second "connect" in the log for one arrival.
pub fn arrived(device: &str) {
    let mut g = table();
    let map = g.as_mut().expect("seeded");
    if map.contains_key(device) {
        return;
    }
    map.insert(device.to_string(), crate::util::now_ts_secs());
    drop(g);
    super::activity::record("system", "system", "connect", Some(label(device)));
}

/// A device's peer went away. Silent when it was never here — teardown runs for
/// every peer, including the browser tabs that are not devices at all.
pub fn left(device: &str) {
    let gone = {
        let mut g = table();
        g.as_mut().expect("seeded").remove(device).is_some()
    };
    if gone {
        super::activity::record("system", "system", "disconnect", Some(label(device)));
    }
}

/// Devices connected right now, as the user names them.
pub fn present() -> Vec<String> {
    let ids: Vec<String> = {
        let g = table();
        g.as_ref().expect("seeded").keys().cloned().collect()
    };
    let mut names: Vec<String> = ids.iter().map(|id| label(id)).collect();
    names.sort();
    names
}

/// The user's name for a device, falling back to its id — an unnamed device is
/// still worth saying, and an id is at least true.
fn label(id: &str) -> String {
    crate::server::api::pair::device_name(id).unwrap_or_else(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrival_is_recorded_once_and_departure_is_silent_for_strangers() {
        // A stranger leaving is not news.
        left("never-here");
        arrived("test-phone");
        arrived("test-phone");
        assert!(present().contains(&"test-phone".to_string()));
        left("test-phone");
        assert!(!present().contains(&"test-phone".to_string()));
    }
}
