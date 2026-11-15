//! The lock every writer of a skill's cloud save takes — the engine pulling
//! the account's copy in, and the skill's own scripts writing a move. Shared
//! by convention, not by code (`doc/skill-spec.md` § Cloud):
//!
//! - the lock is `<save path>.lock` (the first path `cloud.save` names),
//!   created exclusively (O_CREAT|O_EXCL) and holding the writer's pid;
//! - a lock older than 10 s is a dead writer's: removed, and taken again;
//! - a writer that finds it held retries every 25 ms, for up to 5 s.

use anyhow::{bail, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

const STALE_AFTER: Duration = Duration::from_secs(10);
const RETRY_EVERY: Duration = Duration::from_millis(25);
const GIVE_UP_AFTER: Duration = Duration::from_secs(5);

/// Held while it lives; the lockfile goes with it.
pub struct SaveLock {
    path: PathBuf,
}

impl Drop for SaveLock {
    fn drop(&mut self) {
        // Only our own: a lock that went stale under us may be someone else's now.
        if holder(&self.path) == Some(std::process::id()) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// `data/state.json` → `data/state.json.lock`.
pub fn lock_path(save: &Path) -> PathBuf {
    let mut name = save.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    save.with_file_name(name)
}

/// Take the lock beside `save`, waiting for another writer to finish.
pub async fn acquire(save: &Path) -> Result<SaveLock> {
    let path = lock_path(save);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let start = Instant::now();
    loop {
        if try_create(&path)? {
            return Ok(SaveLock { path });
        }
        if is_stale(&path) {
            tracing::warn!("{} outlived its writer; taking it", path.display());
            let _ = std::fs::remove_file(&path);
            continue;
        }
        if start.elapsed() >= GIVE_UP_AFTER {
            bail!("{} held for {GIVE_UP_AFTER:?}", path.display());
        }
        tokio::time::sleep(RETRY_EVERY).await;
    }
}

/// True when the lockfile was created here, false when someone holds it.
fn try_create(path: &Path) -> Result<bool> {
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut f) => {
            f.write_all(std::process::id().to_string().as_bytes())?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn is_stale(path: &Path) -> bool {
    let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return false; // gone already: the next try takes it
    };
    SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|age| age > STALE_AFTER)
}

fn holder(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("save-lock-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("state.json")
    }

    #[tokio::test]
    async fn one_writer_at_a_time_and_the_lock_goes_with_it() {
        let save = scratch("one");
        let held = acquire(&save).await.unwrap();
        assert_eq!(holder(&lock_path(&save)), Some(std::process::id()));
        assert!(!try_create(&lock_path(&save)).unwrap(), "held");
        drop(held);
        assert!(!lock_path(&save).exists());
        drop(acquire(&save).await.unwrap());
    }

    #[tokio::test]
    async fn a_dead_writers_lock_is_taken_over() {
        let save = scratch("stale");
        let path = lock_path(&save);
        std::fs::write(&path, "999999").unwrap();
        let old = SystemTime::now() - Duration::from_secs(30);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let held = acquire(&save).await.unwrap();
        assert_eq!(holder(&path), Some(std::process::id()));
        drop(held);
    }

    #[test]
    fn the_lock_sits_beside_the_save() {
        assert_eq!(
            lock_path(Path::new("/s/g/data/state.json")),
            PathBuf::from("/s/g/data/state.json.lock")
        );
    }
}
