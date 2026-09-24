//! Per-device sync ledger.
//!
//! `~/.linggen/sync/<skill>.json`: which of the skill's items each paired
//! device holds. The skill's own UI reads it back via `/devices` to show true
//! per-phone coverage. Keyed by `PairedDevice.id`, so revoking a device
//! orphans (not corrupts) its row.
//!
//! Held in memory once loaded. A fetch only marks it dirty — a sync of a few
//! hundred files used to parse and rewrite the whole file once per file — and
//! a single flush follows [`FLUSH_DELAY`] later. A `/have` report replaces a
//! row and is persisted before it is answered.

use crate::util::LockExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub(super) struct DeviceSync {
    pub files: Vec<String>,
    pub last_fetch: i64,
}

pub(super) type Ledger = HashMap<String, DeviceSync>;

/// How long fetch records gather before one write.
const FLUSH_DELAY: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Slot {
    ledger: Ledger,
    dirty: bool,
    flush_pending: bool,
}

pub(super) struct LedgerStore {
    dir: PathBuf,
    slots: Mutex<HashMap<String, Slot>>,
    /// One writer at a time, so an older snapshot never lands after a newer.
    write: Mutex<()>,
}

pub(super) static STORE: LazyLock<Arc<LedgerStore>> =
    LazyLock::new(|| Arc::new(LedgerStore::new(crate::paths::linggen_home().join("sync"))));

impl LedgerStore {
    pub(super) fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            slots: Mutex::new(HashMap::new()),
            write: Mutex::new(()),
        }
    }

    fn path(&self, skill: &str) -> PathBuf {
        self.dir.join(format!("{skill}.json"))
    }

    fn read_disk(&self, skill: &str) -> Ledger {
        std::fs::read_to_string(self.path(skill))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn with_slot<R>(&self, skill: &str, f: impl FnOnce(&mut Slot) -> R) -> R {
        let mut slots = self.slots.lock_ok();
        let slot = slots.entry(skill.to_string()).or_insert_with(|| Slot {
            ledger: self.read_disk(skill),
            ..Slot::default()
        });
        f(slot)
    }

    /// The ledger as it stands, including records not yet flushed.
    pub(super) fn snapshot(&self, skill: &str) -> Ledger {
        self.with_slot(skill, |s| s.ledger.clone())
    }

    /// Note that `device` fetched `name`. Returns true when the caller should
    /// schedule a flush (none is pending yet).
    pub(super) fn record_fetch(&self, skill: &str, device: &str, name: &str, now: i64) -> bool {
        self.with_slot(skill, |s| {
            let entry = s.ledger.entry(device.to_string()).or_default();
            if !entry.files.iter().any(|f| f == name) {
                entry.files.push(name.to_string());
            }
            entry.last_fetch = now;
            s.dirty = true;
            !std::mem::replace(&mut s.flush_pending, true)
        })
    }

    /// Replace a device's row with its reported inventory, persisted now.
    pub(super) fn replace(&self, skill: &str, device: &str, files: Vec<String>, now: i64) -> bool {
        self.with_slot(skill, |s| {
            s.ledger.insert(
                device.to_string(),
                DeviceSync {
                    files,
                    last_fetch: now,
                },
            );
            s.dirty = true;
        });
        self.flush(skill)
    }

    /// Write the ledger if it has unsaved changes. True when it is on disk.
    pub(super) fn flush(&self, skill: &str) -> bool {
        let _w = self.write.lock_ok();
        let snapshot = self.with_slot(skill, |s| {
            s.flush_pending = false;
            std::mem::take(&mut s.dirty).then(|| s.ledger.clone())
        });
        let Some(ledger) = snapshot else {
            return true;
        };
        let ok = self.write_disk(skill, &ledger);
        if !ok {
            self.with_slot(skill, |s| s.dirty = true);
        }
        ok
    }

    fn write_disk(&self, skill: &str, ledger: &Ledger) -> bool {
        let Ok(text) = serde_json::to_string_pretty(ledger) else {
            return false;
        };
        let path = self.path(skill);
        let tmp = path.with_extension("json.tmp");
        std::fs::create_dir_all(&self.dir).is_ok()
            && std::fs::write(&tmp, text).is_ok()
            && std::fs::rename(&tmp, &path).is_ok()
    }
}

/// Record a fetch and, when none is queued, flush once after [`FLUSH_DELAY`].
pub(super) fn record_fetch(skill: &str, device: &str, name: &str) {
    let store = STORE.clone();
    let now = chrono::Utc::now().timestamp();
    if !store.record_fetch(skill, device, name, now) {
        return;
    }
    let skill = skill.to_string();
    tokio::spawn(async move {
        tokio::time::sleep(FLUSH_DELAY).await;
        let _ = tokio::task::spawn_blocking(move || store.flush(&skill)).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetches_batch_until_flush() {
        let d = tempfile::tempdir().unwrap();
        let store = LedgerStore::new(d.path().to_path_buf());
        assert!(store.record_fetch("s", "dev", "a", 1));
        // Later fetches ride the already-scheduled flush.
        assert!(!store.record_fetch("s", "dev", "b", 2));
        assert!(!store.record_fetch("s", "dev", "a", 3));
        assert!(!d.path().join("s.json").exists());
        assert_eq!(store.snapshot("s")["dev"].files, vec!["a", "b"]);

        assert!(store.flush("s"));
        let reread = LedgerStore::new(d.path().to_path_buf()).snapshot("s");
        assert_eq!(
            reread["dev"],
            DeviceSync {
                files: vec!["a".into(), "b".into()],
                last_fetch: 3
            }
        );
        // After a flush the next fetch schedules again.
        assert!(store.record_fetch("s", "dev", "c", 4));
    }

    #[test]
    fn have_report_is_persisted_at_once() {
        let d = tempfile::tempdir().unwrap();
        let store = LedgerStore::new(d.path().to_path_buf());
        store.record_fetch("s", "dev", "old", 1);
        assert!(store.replace("s", "dev", vec!["x".into()], 5));
        let reread = LedgerStore::new(d.path().to_path_buf()).snapshot("s");
        assert_eq!(reread["dev"].files, vec!["x"]);
        assert_eq!(reread["dev"].last_fetch, 5);
    }
}
