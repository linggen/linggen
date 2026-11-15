//! Fetched objects, kept by content: `<linggen home>/assets/sha256/<hash>.<ext>`.
//! Shared by every skill and every version of one.

use super::manifest::FileEntry;
use super::Pack;
use std::collections::HashSet;
use std::path::PathBuf;

/// A half-written download. Never served, never listed; swept.
const PART: &str = ".part";
/// An object nothing lists is kept this long before a sweep takes it. Another
/// engine on this device may list it (a second skills folder, a version being
/// installed): removing it at once would only make that one fetch it again.
const UNLISTED_KEPT: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 3600);

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The device's store, under the Linggen home.
    pub fn in_home() -> Self {
        Self::new(crate::paths::linggen_home().join("assets").join("sha256"))
    }

    pub fn path_of(&self, file: &FileEntry) -> PathBuf {
        self.root.join(file.object())
    }

    pub fn temp_of(&self, file: &FileEntry) -> PathBuf {
        self.root.join(format!("{}{PART}", file.object()))
    }

    pub fn holds(&self, file: &FileEntry) -> bool {
        self.path_of(file).is_file()
    }

    pub fn ensure_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)
    }

    /// Remove objects no pack has listed for a month, and any half-written
    /// download. Returns how many files went. Only called with the complete
    /// set of installed packs: an object that is still listed is never removed.
    pub fn sweep(&self, packs: &[Pack]) -> usize {
        self.sweep_at(packs, std::time::SystemTime::now())
    }

    fn sweep_at(&self, packs: &[Pack], now: std::time::SystemTime) -> usize {
        let keep: HashSet<String> = packs
            .iter()
            .flat_map(|p| p.manifest.files.iter().map(FileEntry::object))
            .collect();
        let Ok(dir) = std::fs::read_dir(&self.root) else {
            return 0;
        };
        let mut removed = 0;
        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stale = name.ends_with(PART) || (!keep.contains(&name) && old_enough(&entry, now));
            if stale && std::fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
        removed
    }
}

/// Written longer ago than an unlisted object is kept. Unknown age: kept.
fn old_enough(entry: &std::fs::DirEntry, now: std::time::SystemTime) -> bool {
    let Ok(written) = entry.metadata().and_then(|m| m.modified()) else {
        return false;
    };
    now.duration_since(written)
        .is_ok_and(|age| age >= UNLISTED_KEPT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::manifest::Manifest;

    fn entry(path: &str, sha: &str) -> FileEntry {
        FileEntry {
            path: path.into(),
            sha256: sha.repeat(64),
            size: 1,
            pack: "p".into(),
            order: 1,
        }
    }

    #[test]
    fn sweep_keeps_listed_objects_and_removes_the_rest() {
        let dir = tempfile::tempdir().expect("dir");
        let store = Store::new(dir.path().to_path_buf());
        let (listed, other) = (entry("a.png", "a"), entry("b.png", "b"));
        std::fs::write(store.path_of(&listed), b"1").expect("w");
        std::fs::write(store.path_of(&other), b"2").expect("w");
        std::fs::write(store.temp_of(&listed), b"3").expect("w");
        let pack = Pack {
            skill: "s".into(),
            dir: PathBuf::new(),
            manifest: Manifest {
                version: 1,
                base: "https://x.test/".into(),
                files: vec![listed.clone()],
            },
        };

        // Just written: the unlisted object stays, the half-written download goes.
        assert_eq!(store.sweep(std::slice::from_ref(&pack)), 1);
        assert!(store.holds(&other));
        assert!(!store.temp_of(&listed).exists());

        // A month on: the unlisted object goes, the listed one never does.
        let later =
            std::time::SystemTime::now() + UNLISTED_KEPT + std::time::Duration::from_secs(60);
        assert_eq!(store.sweep_at(&[pack], later), 1);
        assert!(store.holds(&listed));
        assert!(!store.holds(&other));
    }

    #[test]
    fn sweeping_a_store_that_is_not_there_is_nothing() {
        let dir = tempfile::tempdir().expect("dir");
        assert_eq!(Store::new(dir.path().join("none")).sweep(&[]), 0);
    }
}
