//! The paired-device file, cached.
//!
//! Every LAN request and every tunnelled phone call asks "which device is
//! this?", so the list lives in memory and the file is re-read only when it
//! changed on disk (a hand edit — revoking by deleting a row is documented).
//! Writes go through [`DeviceStore::modify`]: one writer at a time — under
//! `ling_mem::hosted::device_list_lock`, the lock memory's Allow and Revoke
//! hold too — temp file + rename, so a reader never sees half a file.
//!
//! A file that no longer parses is never taken as "no devices": the last good
//! list stays in memory, and the next write moves the unreadable file aside
//! instead of overwriting it.

use super::PairedDevice;
use crate::util::RwLockExt;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Duration, Instant, SystemTime};

/// How long a cached read is trusted before the file's stamp is checked again.
/// Bounds per-request filesystem work to one `stat` per interval.
const RECHECK: Duration = Duration::from_secs(1);

type Stamp = Option<(SystemTime, u64)>;

#[derive(Default)]
struct Cache {
    devices: Vec<PairedDevice>,
    /// Stamp of the file the cache reflects (`None` = no file).
    stamp: Stamp,
    checked: Option<Instant>,
    /// The file on disk failed to parse; `devices` is the last good copy.
    corrupt: bool,
}

pub(super) struct DeviceStore {
    path: PathBuf,
    cache: RwLock<Option<Cache>>,
}

/// Seeds / migrates rows as they are read; true when a row changed and the
/// file should be rewritten.
pub(super) type Migrate = fn(&mut [PairedDevice]) -> bool;

impl DeviceStore {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            cache: RwLock::new(None),
        }
    }

    /// The current device list. Rows a migration changed are persisted
    /// through [`Self::modify`], under the one writer lock.
    pub(super) fn load(&self, migrate: Migrate) -> Vec<PairedDevice> {
        if let Some(c) = self.cache.read_ok().as_ref() {
            if c.checked.is_some_and(|t| t.elapsed() < RECHECK) {
                return c.devices.clone();
            }
        }
        let (devices, migrated) = self.refresh(migrate);
        if migrated {
            if let Err(e) = self.modify(migrate, |_| ()) {
                tracing::warn!("[pair] could not persist migrated device rows: {e}");
            }
        }
        devices
    }

    /// Drop what the cache knows of the file, so the next read checks it.
    pub(super) fn forget(&self) {
        if let Some(c) = self.cache.write_ok().as_mut() {
            c.checked = None;
            c.stamp = None;
        }
    }

    /// Apply `f` to the list and persist it atomically, holding the one lock
    /// every writer of the device file in this process holds (memory's
    /// Allow and Revoke too), so no save carries a list missing another's
    /// change.
    pub(super) fn modify<R>(
        &self,
        migrate: Migrate,
        f: impl FnOnce(&mut Vec<PairedDevice>) -> R,
    ) -> std::io::Result<R> {
        let _w = ling_mem::hosted::device_list_lock();
        let (mut devices, _) = self.refresh(migrate);
        let out = f(&mut devices);
        self.save(&devices)?;
        Ok(out)
    }

    /// The list as the file has it now, and whether a migration changed a
    /// row (the caller persists it; this never writes).
    fn refresh(&self, migrate: Migrate) -> (Vec<PairedDevice>, bool) {
        let stamp = stamp_of(&self.path);
        {
            let mut guard = self.cache.write_ok();
            if let Some(c) = guard.as_mut() {
                if c.stamp == stamp {
                    c.checked = Some(Instant::now());
                    return (c.devices.clone(), false);
                }
            }
        }
        match read_file(&self.path) {
            Ok(mut devices) => {
                let migrated = migrate(&mut devices);
                self.set_cache(devices.clone(), stamp, false);
                (devices, migrated)
            }
            Err(e) => {
                tracing::error!(
                    "[pair] {} does not parse ({e}) — keeping the last good device list",
                    self.path.display()
                );
                let mut guard = self.cache.write_ok();
                let c = guard.get_or_insert_with(Cache::default);
                c.stamp = stamp;
                c.checked = Some(Instant::now());
                c.corrupt = true;
                (c.devices.clone(), false)
            }
        }
    }

    fn save(&self, devices: &[PairedDevice]) -> std::io::Result<()> {
        let corrupt = self.cache.read_ok().as_ref().is_some_and(|c| c.corrupt);
        if corrupt && self.path.exists() {
            let aside = self
                .path
                .with_extension(format!("json.corrupt-{}", chrono::Utc::now().timestamp()));
            std::fs::rename(&self.path, &aside)?;
            tracing::warn!("[pair] moved unreadable device file to {}", aside.display());
        }
        write_atomic(
            &self.path,
            serde_json::to_string_pretty(devices)?.as_bytes(),
        )?;
        self.set_cache(devices.to_vec(), stamp_of(&self.path), false);
        Ok(())
    }

    fn set_cache(&self, devices: Vec<PairedDevice>, stamp: Stamp, corrupt: bool) {
        *self.cache.write_ok() = Some(Cache {
            devices,
            stamp,
            checked: Some(Instant::now()),
            corrupt,
        });
    }
}

fn stamp_of(path: &Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Missing file = no devices yet. Anything unreadable or unparseable is an
/// error, never an empty list.
fn read_file(path: &Path) -> std::io::Result<Vec<PairedDevice>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    serde_json::from_str(&text).map_err(std::io::Error::other)
}

/// Temp file in the same directory, synced, then renamed over `path` — a
/// reader sees the old file or the new one, never half of either.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    std::io::Write::write_all(&mut tmp, bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_migrate(_: &mut [PairedDevice]) -> bool {
        false
    }

    fn device(id: &str) -> PairedDevice {
        PairedDevice {
            id: id.into(),
            name: format!("phone {id}"),
            secret: format!("secret-{id}"),
            created_at: 0,
            kind: None,
            account: None,
            device_id: None,
            settings: Default::default(),
            relay_grant: None,
            superseded_grant: None,
            ..Default::default()
        }
    }

    #[test]
    fn missing_file_is_empty_and_writes_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = DeviceStore::new(dir.path().join("paired-devices.json"));
        assert!(store.load(no_migrate).is_empty());
        store.modify(no_migrate, |d| d.push(device("a"))).unwrap();
        let fresh = DeviceStore::new(dir.path().join("paired-devices.json"));
        assert_eq!(fresh.load(no_migrate).len(), 1);
    }

    #[test]
    fn a_corrupt_file_keeps_the_last_good_list_and_is_moved_aside_not_wiped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("paired-devices.json");
        let store = DeviceStore::new(path.clone());
        store
            .modify(no_migrate, |d| d.extend([device("a"), device("b")]))
            .unwrap();

        // Something scribbles on the file (a torn hand edit, a bad tool).
        std::fs::write(&path, "{ not json").unwrap();
        store.cache.write_ok().as_mut().unwrap().checked = None;

        let seen = store.load(no_migrate);
        assert_eq!(seen.len(), 2, "last good list survives a parse error");

        // The next write keeps both rows and preserves the bad file.
        store.modify(no_migrate, |d| d.push(device("c"))).unwrap();
        let fresh = DeviceStore::new(path.clone());
        assert_eq!(fresh.load(no_migrate).len(), 3);
        let aside = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains("corrupt"));
        assert!(aside, "unreadable file is moved aside, never overwritten");
    }

    /// The device file has one writer lock in this process,
    /// `ling_mem::hosted::device_list_lock`. Its writers are this store's
    /// `modify` and memory's Allow and Revoke; each holds that lock while it
    /// reads, changes and saves. A write outside them, or one that does not
    /// hold the lock, fails here.
    #[test]
    fn every_write_of_the_device_file_holds_the_one_lock() {
        let engine = include_str!("store.rs");
        let memory = include_str!("../../../../linggen-memory/src/http/connect/devices.rs");
        let cases: [(&str, &str, &[&str], usize); 2] = [
            ("store.rs", engine, &["self.save(", "write_atomic(&self.path"], 1),
            ("devices.rs", memory, &["modify(&paired_path(", "write_list(&paired_path("], 2),
        ];
        for (file, text, writes, expected) in cases {
            let code = text.split("#[cfg(test)]").next().unwrap_or("");
            let writers: Vec<&str> = code
                .split("fn ")
                .filter(|body| writes.iter().any(|w| body.contains(w)))
                // The two save primitives themselves, each called only by
                // the writers counted here.
                .filter(|body| !body.starts_with("save(") && !body.starts_with("modify<T, R>("))
                .collect();
            for body in &writers {
                let name = body.split('(').next().unwrap_or(body);
                assert!(
                    body.contains("device_list_lock()"),
                    "{file}: {name} writes the device file without the one lock"
                );
            }
            assert_eq!(writers.len(), expected, "{file}: writers of the device file");
        }
    }

    #[test]
    fn a_fresh_store_on_a_corrupt_file_never_saves_an_empty_list_over_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("paired-devices.json");
        std::fs::write(&path, "[{\"id\": broken").unwrap();
        let store = DeviceStore::new(path.clone());
        assert!(store.load(no_migrate).is_empty());
        store.modify(no_migrate, |d| d.push(device("n"))).unwrap();
        let kept = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().contains("corrupt"))
            .expect("original bytes preserved");
        assert_eq!(
            std::fs::read_to_string(kept.path()).unwrap(),
            "[{\"id\": broken"
        );
    }
}
