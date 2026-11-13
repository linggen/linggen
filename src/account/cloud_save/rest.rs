//! How long a save's progress has stood still here — the "at rest" a
//! device's copy waits for before it replaces changes made here
//! (`peer.rs` [`super::peer::AT_REST`]).
//!
//! Measured from the last write that changed the save's progress
//! fingerprint (`files.rs` `progress_fingerprint`: the keys the skill
//! declares in `cloud.ignore` left out), never from the files' mtime alone: a
//! page left open that stamps its save (a hold renewed every few minutes)
//! writes the file without a move, and by mtime the save never came to rest —
//! on 2026-11-12 that held a phone's further copy off this Mac all evening.
//!
//! Every read of the save notes its fingerprint (in memory, per save root).
//! When it differs from the one last noted, the change is dated by the
//! newest file's mtime — no earlier than the change itself, so the rest is
//! never overstated. Unnoted (a fresh start), the mtime is all there is.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use super::files::Files;
use super::SaveTarget;

/// Per save root: the progress fingerprint last noted, and when it last changed (unix secs).
static MARKS: Mutex<Option<HashMap<PathBuf, (u64, u64)>>> = Mutex::new(None);

fn secs(t: SystemTime) -> Option<u64> {
    t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// Unix secs of the newest write of the save's files.
fn newest_write(t: &SaveTarget, files: &Files) -> Option<u64> {
    files
        .keys()
        .filter_map(|rel| {
            std::fs::metadata(t.layout.root.join(rel))
                .ok()?
                .modified()
                .ok()
        })
        .max()
        .and_then(secs)
}

/// Note the save as read (`files`, its progress fingerprint `progress`) and
/// answer how many seconds its progress has stood still; `u64::MAX` when it
/// has no files.
pub fn note(t: &SaveTarget, files: &Files, progress: u64) -> u64 {
    let Some(written) = newest_write(t, files) else {
        return u64::MAX;
    };
    let now = secs(SystemTime::now()).unwrap_or(written);
    let mut guard = MARKS.lock().unwrap_or_else(|e| e.into_inner());
    let marks = guard.get_or_insert_with(HashMap::new);
    let at = match marks.get(&t.layout.root) {
        Some(&(hash, at)) if hash == progress => at,
        _ => {
            marks.insert(t.layout.root.clone(), (progress, written));
            written
        }
    };
    now.saturating_sub(at)
}
