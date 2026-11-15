//! The store's change log — what a replica pulls (`/api/sync/changes`).
//!
//! Every write primitive in `store.rs` (`upsert`, `insert`, `delete_where`)
//! records the row ids it touched here, under a per-store sequence number that
//! only grows. A replica keeps the last sequence it applied as its cursor and
//! asks for everything after it — a cursor, never a wall clock, so a clock
//! step on either machine cannot skip a change.
//!
//! The log names ids, not row bodies: the sync route reads each id's CURRENT
//! row from the tables (or reports a tombstone when it is gone). So the log
//! may repeat an id freely, and it is compacted by keeping only the newest
//! entry per id.
//!
//! On disk: `<data_dir>/memory/.changes.jsonl`, one entry per line, appended
//! and flushed per write. A torn last line (a crash mid-append) is skipped on
//! open. It is outside the LanceDB tables, so the frozen store schema is
//! untouched. When the file does not exist yet, the daemon seeds it with one
//! entry per existing row, so `since=0` is a full copy.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Rewrite the file once it holds this many entries per live id.
const COMPACT_RATIO: usize = 4;
const COMPACT_MIN: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Upsert,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub seq: u64,
    pub id: String,
    pub table: String,
    pub op: Op,
}

struct Inner {
    entries: Vec<Entry>,
    next_seq: u64,
}

pub struct ChangeLog {
    path: PathBuf,
    inner: Mutex<Inner>,
    /// True when the file did not exist at open — the caller seeds it.
    fresh: bool,
}

pub fn log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join(".changes.jsonl")
}

impl ChangeLog {
    pub fn open(path: PathBuf) -> std::io::Result<Self> {
        let (entries, fresh) = match std::fs::read_to_string(&path) {
            Ok(text) => (parse(&text), false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Vec::new(), true),
            Err(e) => return Err(e),
        };
        let next_seq = entries.last().map_or(1, |e| e.seq + 1);
        Ok(Self {
            path,
            inner: Mutex::new(Inner { entries, next_seq }),
            fresh,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The file did not exist when opened: seed it with every existing row.
    pub fn needs_seed(&self) -> bool {
        self.fresh && self.lock().entries.is_empty()
    }

    /// The newest sequence number, 0 when empty.
    pub fn head(&self) -> u64 {
        self.lock().next_seq - 1
    }

    /// Record that `ids` in `table` changed. A failure to persist is logged,
    /// never fatal to the write that already happened.
    pub fn record(&self, table: &str, op: Op, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        let mut inner = self.lock();
        let mut lines = String::new();
        for id in ids {
            let entry = Entry {
                seq: inner.next_seq,
                id: id.clone(),
                table: table.to_string(),
                op,
            };
            inner.next_seq += 1;
            lines.push_str(&serde_json::to_string(&entry).unwrap_or_default());
            lines.push('\n');
            inner.entries.push(entry);
        }
        if let Err(e) = append(&self.path, &lines) {
            tracing::warn!(
                "change log: could not append to {}: {e}",
                self.path.display()
            );
        }
        self.maybe_compact(&mut inner);
    }

    /// Entries after `cursor`, at most `limit` of them, newest per id kept.
    /// Returns them with the cursor to ask from next and whether more remain.
    pub fn since(&self, cursor: u64, limit: usize) -> (Vec<Entry>, u64, bool) {
        let inner = self.lock();
        let start = inner.entries.partition_point(|e| e.seq <= cursor);
        let page = &inner.entries[start..];
        let taken = &page[..page.len().min(limit.max(1))];
        let next = taken.last().map_or(cursor, |e| e.seq);
        let more = page.len() > taken.len();
        (latest_per_id(taken), next, more)
    }

    fn maybe_compact(&self, inner: &mut Inner) {
        let n = inner.entries.len();
        if n < COMPACT_MIN {
            return;
        }
        let kept = latest_per_id(&inner.entries);
        if n < kept.len() * COMPACT_RATIO {
            return;
        }
        match rewrite(&self.path, &kept) {
            Ok(()) => inner.entries = kept,
            Err(e) => tracing::warn!("change log: compaction failed: {e}"),
        }
    }
}

fn parse(text: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    entries.sort_by_key(|e| e.seq);
    entries
}

/// The newest entry for each id, in sequence order.
fn latest_per_id(entries: &[Entry]) -> Vec<Entry> {
    let mut last: HashMap<&str, usize> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        last.insert(e.id.as_str(), i);
    }
    entries
        .iter()
        .enumerate()
        .filter(|(i, e)| last.get(e.id.as_str()) == Some(i))
        .map(|(_, e)| e.clone())
        .collect()
}

fn append(path: &Path, lines: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(lines.as_bytes())?;
    f.flush()
}

fn rewrite(path: &Path, entries: &[Entry]) -> std::io::Result<()> {
    let mut text = String::new();
    for e in entries {
        text.push_str(&serde_json::to_string(e).map_err(std::io::Error::other)?);
        text.push('\n');
    }
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_cursor_only_moves_forward_and_survives_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".changes.jsonl");
        let log = ChangeLog::open(path.clone()).unwrap();
        assert!(log.needs_seed());
        log.record("semantic", Op::Upsert, &ids(&["a", "b"]));
        log.record("episodic", Op::Delete, &ids(&["c"]));
        assert_eq!(log.head(), 3);

        let again = ChangeLog::open(path).unwrap();
        assert!(!again.needs_seed());
        assert_eq!(again.head(), 3);
        again.record("semantic", Op::Upsert, &ids(&["d"]));
        assert_eq!(again.head(), 4);
        let (page, next, more) = again.since(3, 100);
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].id, "d");
        assert_eq!(next, 4);
        assert!(!more);
    }

    #[test]
    fn a_page_keeps_the_newest_entry_per_id_and_says_if_more_remain() {
        let dir = tempfile::tempdir().unwrap();
        let log = ChangeLog::open(dir.path().join("c.jsonl")).unwrap();
        log.record("episodic", Op::Upsert, &ids(&["a"]));
        log.record("semantic", Op::Upsert, &ids(&["b"]));
        log.record("episodic", Op::Delete, &ids(&["a"]));
        log.record("semantic", Op::Upsert, &ids(&["c"]));

        let (page, next, more) = log.since(0, 3);
        assert_eq!(next, 3);
        assert!(more);
        let a: Vec<_> = page.iter().filter(|e| e.id == "a").collect();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].op, Op::Delete);

        let (rest, next, more) = log.since(next, 3);
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].id, "c");
        assert_eq!(next, 4);
        assert!(!more);
        assert_eq!(log.since(4, 3).0.len(), 0);
    }

    #[test]
    fn a_torn_last_line_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.jsonl");
        std::fs::write(
            &path,
            "{\"seq\":1,\"id\":\"a\",\"table\":\"semantic\",\"op\":\"upsert\"}\n{\"seq\":2,\"id\"",
        )
        .unwrap();
        let log = ChangeLog::open(path).unwrap();
        assert_eq!(log.head(), 1);
    }
}
