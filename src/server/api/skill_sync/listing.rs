//! Directory listings for `/items`, cached.
//!
//! A phone asks for the whole inventory on every sync, and a synced dir can
//! hold thousands of files. One pass over the directory yields each file's
//! name and size (no second `stat` per item or per companion), and the result
//! is kept until something says the directory changed:
//!
//! - a change-watcher on the dir (see [`super::arm_watcher`]) bumps a
//!   generation on every relevant event, dropping every cached listing;
//! - the dir's own mtime moving (an add, remove or rename) drops that one;
//! - a dir no watcher covers is trusted only for [`UNWATCHED_TTL`], since an
//!   in-place rewrite of a file changes its size without touching the dir.

use crate::util::LockExt;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// File name → size in bytes, ordered by name. Regular files only (symlinks
/// to files count, as they always did).
pub(super) type Listing = BTreeMap<String, u64>;

/// How long a listing of a dir no watcher covers is trusted.
const UNWATCHED_TTL: Duration = Duration::from_secs(2);
/// Upper bound even for a watched dir — a watcher can drop events.
const WATCHED_TTL: Duration = Duration::from_secs(60);

struct Entry {
    listing: Arc<Listing>,
    mtime: Option<SystemTime>,
    built: Instant,
    generation: u64,
}

static GENERATION: AtomicU64 = AtomicU64::new(0);
static CACHE: Mutex<Option<HashMap<PathBuf, Entry>>> = Mutex::new(None);
/// Base dirs with a live recursive watcher.
static WATCHED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

/// Something under a watched dir changed: every cached listing is stale.
pub(super) fn invalidate() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

pub(super) fn mark_watched(dir: &Path) {
    WATCHED
        .lock_ok()
        .get_or_insert_with(HashSet::new)
        .insert(dir.to_path_buf());
}

fn is_watched(dir: &Path) -> bool {
    WATCHED
        .lock_ok()
        .as_ref()
        .is_some_and(|w| w.iter().any(|base| dir.starts_with(base)))
}

fn dir_mtime(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir).and_then(|m| m.modified()).ok()
}

/// The listing of `dir`, from cache when it is still good. Blocking — call
/// from `spawn_blocking`.
pub(super) fn list(dir: &Path) -> Arc<Listing> {
    let generation = GENERATION.load(Ordering::SeqCst);
    let mtime = dir_mtime(dir);
    let ttl = if is_watched(dir) {
        WATCHED_TTL
    } else {
        UNWATCHED_TTL
    };
    if let Some(e) = CACHE.lock_ok().as_ref().and_then(|c| c.get(dir)) {
        if e.generation == generation && e.mtime == mtime && e.built.elapsed() < ttl {
            return e.listing.clone();
        }
    }
    let listing = Arc::new(scan(dir));
    CACHE.lock_ok().get_or_insert_with(HashMap::new).insert(
        dir.to_path_buf(),
        Entry {
            listing: listing.clone(),
            mtime,
            built: Instant::now(),
            generation,
        },
    );
    listing
}

/// One pass over `dir`: name and size of every regular file.
fn scan(dir: &Path) -> Listing {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Listing::new();
    };
    rd.filter_map(Result::ok)
        .filter_map(|e| {
            let ft = e.file_type().ok()?;
            // DirEntry metadata doesn't follow symlinks; a linked file still
            // counts, so only those pay for a second stat.
            let meta = if ft.is_symlink() {
                std::fs::metadata(e.path()).ok()?
            } else {
                e.metadata().ok()?
            };
            if !meta.is_file() {
                return None;
            }
            Some((e.file_name().into_string().ok()?, meta.len()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_lists_files_with_sizes_and_skips_dirs() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.txt"), b"abc").unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        let l = scan(d.path());
        assert_eq!(l.len(), 1);
        assert_eq!(l.get("a.txt"), Some(&3));
    }

    #[test]
    fn cached_listing_drops_on_invalidate() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_path_buf();
        mark_watched(&dir);
        std::fs::write(dir.join("a.txt"), b"abc").unwrap();
        assert_eq!(list(&dir).get("a.txt"), Some(&3));

        // In-place rewrite: the dir's mtime does not move, so only the
        // watcher's invalidation can reveal the new size.
        std::fs::write(dir.join("a.txt"), b"abcdef").unwrap();
        let before = list(&dir).get("a.txt").copied();
        invalidate();
        assert_eq!(list(&dir).get("a.txt"), Some(&6));
        // Cached until then (or the mtime happened to tick — both fine).
        assert!(before == Some(3) || before == Some(6));
    }

    #[test]
    fn cached_listing_drops_when_dir_changes() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_path_buf();
        mark_watched(&dir);
        assert!(list(&dir).is_empty());
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("new.txt"), b"x").unwrap();
        assert_eq!(list(&dir).get("new.txt"), Some(&1));
    }
}
