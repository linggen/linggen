//! `memory/backups/` housekeeping — what is kept, and for how long.
//!
//! Two kinds of backup live there, side by side:
//!
//! - **Daily snapshots** — `store-YYYY-MM-DD.ndjson`, one `ling-mem export`
//!   per memory-agent run day (written by the Linggen engine). The newest
//!   [`DAILY_KEEP`] are kept.
//! - **Migration backups** — a full copy of the store taken before a
//!   store-schema or scope step, one directory per run, named
//!   `<kind>-<UTC stamp>/` with a kind from [`MIGRATION_BACKUP_KINDS`].
//!   Each is kept [`MIGRATION_KEEP_DAYS`] days, then removed.
//!
//! The daemon runs [`prune`] from its maintenance loop, so every host's
//! store is kept tidy, not only the one Linggen runs on. Nothing outside
//! `backups/` is ever touched, and inside it only entries matching one of
//! the two patterns above: an unknown file or directory stays.

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use std::path::{Path, PathBuf};

/// Every directory-name prefix a migration backup is written under — the
/// one place the pattern is named. `schema-v2` is the gated v1 → v2 step
/// (`/api/schema/apply`); `scope-migration` is the 2026-10-04 scope review
/// it replaced, kept so the copies that build left behind still expire.
/// A new migration adds its kind here and builds its path with
/// [`migration_backup_dir`].
pub const MIGRATION_BACKUP_KINDS: &[&str] = &[SCHEMA_V2, "scope-migration"];

/// The gated v1 → v2 schema step's backup kind.
pub const SCHEMA_V2: &str = "schema-v2";

/// How long a migration backup is kept.
pub const MIGRATION_KEEP_DAYS: i64 = 14;

/// How many daily `store-*.ndjson` snapshots are kept.
pub const DAILY_KEEP: usize = 7;

/// The stamp format in a migration backup's name.
const STAMP_FORMAT: &str = "%Y%m%dT%H%M%SZ";

/// `<data_dir>/memory/backups/`.
pub fn backups_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join("backups")
}

/// Where a migration backup of `kind` taken at `now` goes:
/// `<data_dir>/memory/backups/<kind>-<UTC stamp>/`. `kind` must be one of
/// [`MIGRATION_BACKUP_KINDS`], or the backup would never expire.
pub fn migration_backup_dir(data_dir: &Path, kind: &str, now: DateTime<Utc>) -> PathBuf {
    debug_assert!(
        MIGRATION_BACKUP_KINDS.contains(&kind),
        "unregistered kind {kind}"
    );
    backups_dir(data_dir).join(format!("{kind}-{}", now.format(STAMP_FORMAT)))
}

/// Remove expired backups from `dir` (a `backups/` directory) as of `now`:
/// migration backups older than [`MIGRATION_KEEP_DAYS`], and daily
/// snapshots beyond the newest [`DAILY_KEEP`]. Returns what was removed;
/// each removal is logged. Best-effort: an entry that cannot be read or
/// removed is logged and left.
pub fn prune(dir: &Path, now: DateTime<Utc>) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut migrations = Vec::new();
    let mut dailies = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // Never follow a link: only real files and directories are ours.
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if meta.is_dir() && migration_kind(name).is_some() {
            migrations.push((path, meta));
        } else if meta.is_file() && is_daily(name) {
            dailies.push(path);
        }
    }

    let mut removed = expired_migrations(migrations, now);
    removed.extend(surplus_dailies(dailies));
    removed.retain(|path| remove(path));
    removed
}

/// The migration kind a directory name was written under, if any.
fn migration_kind(name: &str) -> Option<&'static str> {
    MIGRATION_BACKUP_KINDS.iter().copied().find(|kind| {
        name.strip_prefix(kind)
            .is_some_and(|rest| rest.starts_with('-'))
    })
}

fn is_daily(name: &str) -> bool {
    name.starts_with("store-") && name.ends_with(".ndjson")
}

/// Migration backups older than the keep window.
fn expired_migrations(dirs: Vec<(PathBuf, std::fs::Metadata)>, now: DateTime<Utc>) -> Vec<PathBuf> {
    let cutoff = now - chrono::Duration::days(MIGRATION_KEEP_DAYS);
    dirs.into_iter()
        .filter(|(path, meta)| taken_at(path, meta).is_some_and(|t| t < cutoff))
        .map(|(path, _)| path)
        .collect()
}

/// When a migration backup was taken: the stamp in its name, else the
/// directory's modified time. `None` (neither readable) keeps it.
fn taken_at(path: &Path, meta: &std::fs::Metadata) -> Option<DateTime<Utc>> {
    let name = path.file_name()?.to_str()?;
    let stamp = &name[migration_kind(name)?.len() + 1..];
    parse_stamp(stamp).or_else(|| meta.modified().ok().map(DateTime::<Utc>::from))
}

/// `20261004T151910Z`, or a bare `2026-10-04` / `20261004` date.
fn parse_stamp(stamp: &str) -> Option<DateTime<Utc>> {
    if let Ok(t) = NaiveDateTime::parse_from_str(stamp, STAMP_FORMAT) {
        return Some(t.and_utc());
    }
    ["%Y-%m-%d", "%Y%m%d"]
        .iter()
        .find_map(|f| NaiveDate::parse_from_str(stamp, f).ok())
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|t| t.and_utc())
}

/// Daily snapshots beyond the newest [`DAILY_KEEP`] — date-named, so
/// lexicographic order is chronological.
fn surplus_dailies(mut files: Vec<PathBuf>) -> Vec<PathBuf> {
    files.sort();
    let surplus = files.len().saturating_sub(DAILY_KEEP);
    files.truncate(surplus);
    files
}

fn remove(path: &Path) -> bool {
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match result {
        Ok(()) => {
            tracing::info!(path = %path.display(), "backups: removed expired backup");
            true
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "backups: could not remove");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 22, 12, 0, 0).unwrap()
    }

    fn mkdir(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::create_dir_all(p.join("memory.lancedb")).unwrap();
        std::fs::write(p.join("memory.lancedb").join("data.lance"), b"x").unwrap();
        p
    }

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, b"{}\n").unwrap();
        p
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn migration_backup_dir_is_named_by_kind_and_stamp() {
        let p = migration_backup_dir(Path::new("/d"), "schema-v2", now());
        assert_eq!(p, Path::new("/d/memory/backups/schema-v2-20261022T120000Z"));
        assert_eq!(
            migration_kind("schema-v2-20261022T120000Z"),
            Some("schema-v2")
        );
        let stamp = parse_stamp("20261022T120000Z").unwrap();
        assert_eq!(stamp, now());
    }

    #[test]
    fn old_migration_backups_are_removed_young_ones_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let old_schema = mkdir(dir, "schema-v2-20261004T151910Z");
        let old_scope = mkdir(dir, "scope-migration-20261004T150000Z");
        let edge = mkdir(dir, "schema-v2-20261008T130000Z"); // 13 d 23 h
        let newest = mkdir(dir, "schema-v2-20261021T090000Z");

        let removed = prune(dir, now());

        assert_eq!(removed.len(), 2, "{removed:?}");
        assert!(!old_schema.exists() && !old_scope.exists());
        assert!(edge.exists(), "younger than 14 days stays");
        assert!(newest.exists(), "the newest, young backup stays");
    }

    #[test]
    fn a_lone_young_migration_backup_is_never_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let only = mkdir(tmp.path(), "schema-v2-20261020T000000Z");
        assert!(prune(tmp.path(), now()).is_empty());
        assert!(only.exists());
    }

    #[test]
    fn unstamped_migration_backup_falls_back_to_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let fresh = mkdir(tmp.path(), "schema-v2-manual");
        // Just written: its mtime is now, so it is young.
        assert!(prune(tmp.path(), Utc::now()).is_empty());
        assert!(fresh.exists());
        // Seen from a month later, it has expired.
        let later = Utc::now() + chrono::Duration::days(30);
        assert_eq!(prune(tmp.path(), later), vec![fresh.clone()]);
        assert!(!fresh.exists());
    }

    #[test]
    fn daily_backups_still_keep_seven() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        for day in 10..=18 {
            touch(dir, &format!("store-2026-10-{day}.ndjson"));
        }
        let removed = prune(dir, now());
        assert_eq!(removed.len(), 2);
        let left = names(dir);
        assert_eq!(left.len(), 7);
        assert_eq!(left[0], "store-2026-10-12.ndjson");
        assert_eq!(left[6], "store-2026-10-18.ndjson");
    }

    #[test]
    fn unknown_entries_are_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        touch(dir, "README");
        touch(dir, "schema-v2-20200101T000000Z"); // a file, not a backup dir
        mkdir(dir, "pre-scope-deploy-2026-10-04"); // hand-made, not ours
        mkdir(dir, "my-copy-20200101T000000Z");
        mkdir(dir, "schema-v20-20200101T000000Z"); // not the `schema-v2-` kind
        mkdir(dir, "store-2026-01-01.ndjson"); // a dir, not a snapshot
        let before = names(dir);

        assert!(prune(dir, now()).is_empty());
        assert_eq!(names(dir), before);
    }

    #[cfg(unix)]
    #[test]
    fn links_are_never_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside");
        let target = mkdir(&outside, "keep-me");
        let dir = tmp.path().join("backups");
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&target, dir.join("schema-v2-20200101T000000Z")).unwrap();

        assert!(prune(&dir, now()).is_empty());
        assert!(target.join("memory.lancedb").exists());
    }

    #[test]
    fn missing_dir_is_a_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(prune(&tmp.path().join("backups"), now()).is_empty());
    }
}
