//! What changed on this machine, and who did it.
//!
//! Append-only JSONL, one file per day under `~/.linggen/activity/`, local and
//! never uploaded — it carries song titles, filenames and import counts, and
//! answers a question ("what is true on this machine") that has no meaning
//! anywhere else. See `doc/perception-spec.md` §3 and §8.
//!
//! The record shape is the phone's, field for field, because two schemas for
//! one idea fork on contact.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Long enough that "recent" still means something late at night, short enough
/// that this never becomes a second memory. What is durable about the person is
/// promoted by the dream pass and lives in `ling-mem` instead.
pub const KEEP_DAYS: i64 = 3;

/// One thing that happened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    /// ISO 8601, UTC.
    pub at: String,
    /// `user`, `yinyue`, `ling`, `system` — the actor, not the device.
    pub by: String,
    /// Which machine this happened on. Absent in files written before hosts
    /// stamped themselves; a reader treats that as "the host whose file it is".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// `dj`, `photos`, `cfo`, `shifu`, `system`.
    pub app: String,
    /// A short slug — `delete`, `add`, `edit`, `sync`, `backup`, `clean`,
    /// `import`, `pair`, `connect`, `disconnect`.
    pub verb: String,
    /// What it happened to, in the user's terms: a song title, not a path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl Activity {
    /// One line, as a person would read it. What the doorbell shows and what
    /// `recent_activity` returns — the agent never sees the JSON.
    pub fn line(&self) -> String {
        let who = match self.by.as_str() {
            "user" => "you",
            "system" => "the Mac",
            other => other,
        };
        match &self.object {
            Some(o) => format!("{who} {} {o}", self.verb),
            None => format!("{who} {}", self.verb),
        }
    }

    /// Seconds since this happened, or `None` when the stamp is unreadable.
    pub fn age_secs(&self) -> Option<i64> {
        let at = chrono::DateTime::parse_from_rfc3339(&self.at).ok()?;
        Some((chrono::Utc::now() - at.with_timezone(&chrono::Utc)).num_seconds())
    }
}

/// "3m ago" — the only form a reader wants next to a line.
pub fn ago(secs: i64) -> String {
    match secs {
        s if s < 60 => "just now".to_string(),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

/// `~/.linggen/activity/`
pub fn activity_dir() -> PathBuf {
    // Tests exercise real writers, and a test run must never leave rows in the
    // user's own log — the one place that could happen is here, so it is the
    // one place that has to know.
    #[cfg(test)]
    {
        return std::env::temp_dir().join("ling-activity-test");
    }
    #[cfg(not(test))]
    crate::paths::linggen_home().join("activity")
}

fn day_key(t: chrono::DateTime<chrono::Local>) -> String {
    t.format("%Y-%m-%d").to_string()
}

/// The machine's log. One per process; every writer goes through it so the
/// in-memory window and the file can never disagree.
pub struct ActivityLog {
    /// Newest last. Small by construction, and every read wants the whole
    /// window anyway.
    recent: Mutex<Vec<Activity>>,
    /// Bumped on every append, so "since you last looked" can be per agent
    /// without storing a cursor per agent.
    revision: AtomicU64,
    dir: PathBuf,
}

static LOG: std::sync::OnceLock<ActivityLog> = std::sync::OnceLock::new();

/// The process-wide log, loaded from disk on first touch.
pub fn log() -> &'static ActivityLog {
    LOG.get_or_init(|| {
        let l = ActivityLog::new(activity_dir());
        l.load();
        l
    })
}

impl ActivityLog {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            recent: Mutex::new(Vec::new()),
            revision: AtomicU64::new(0),
            dir,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    /// Newest first — the order a reader wants.
    pub fn recent(&self) -> Vec<Activity> {
        let guard = self.recent.lock().unwrap_or_else(|e| e.into_inner());
        guard.iter().rev().cloned().collect()
    }

    /// The newest `limit` entries as lines a person could read.
    pub fn lines(&self, limit: usize) -> Vec<String> {
        self.recent()
            .into_iter()
            .take(limit)
            .map(|a| match a.age_secs() {
                Some(s) => format!("{} · {}", ago(s), a.line()),
                None => a.line(),
            })
            .collect()
    }

    /// Record something that happened. Never fails the caller's own work — a
    /// log that can break an action is worse than no log.
    pub fn add(&self, a: Activity) {
        // In memory first, so this process sees it even when the disk can't
        // take it. Deliberately does not reload: a load replaces the window
        // and would drop the entry just made.
        {
            let mut guard = self.recent.lock().unwrap_or_else(|e| e.into_inner());
            guard.push(a.clone());
        }
        self.revision.fetch_add(1, Ordering::Relaxed);

        let day = chrono::DateTime::parse_from_rfc3339(&a.at)
            .map(|t| day_key(t.with_timezone(&chrono::Local)))
            .unwrap_or_else(|_| day_key(chrono::Local::now()));
        if let Err(e) = self.append_line(&day, &a) {
            tracing::debug!("[perception] activity not recorded: {e}");
        }
    }

    fn append_line(&self, day: &str, a: &Activity) -> std::io::Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(&self.dir)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(format!("{day}.jsonl")))?;
        let body = serde_json::to_string(a).unwrap_or_default();
        writeln!(f, "{body}")
    }

    /// Read the retained window, dropping the days that fell out of it.
    ///
    /// Rotation is a side effect of reading: a day outside the window is swept
    /// the first time anyone looks, so nothing has to run on a schedule. The
    /// days swept are returned, newest first, so a caller can hand them
    /// somewhere before they go (§7).
    pub fn load(&self) -> Vec<(String, Vec<Activity>)> {
        let keep: std::collections::HashSet<String> = (0..KEEP_DAYS)
            .map(|i| day_key(chrono::Local::now() - chrono::Duration::days(i)))
            .collect();

        let mut fresh: Vec<Activity> = Vec::new();
        let mut swept: Vec<(String, Vec<Activity>)> = Vec::new();

        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return swept;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        files.sort();

        for path in files {
            let day = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let rows = read_day(&path);
            if keep.contains(&day) {
                fresh.extend(rows);
                continue;
            }
            let _ = std::fs::remove_file(&path);
            swept.push((day, rows));
        }

        fresh.sort_by(|a, b| a.at.cmp(&b.at));
        let mut guard = self.recent.lock().unwrap_or_else(|e| e.into_inner());
        *guard = fresh;
        swept.sort_by(|a, b| b.0.cmp(&a.0));
        swept
    }
}

/// A torn line is one lost entry, not a lost day.
fn read_day(path: &std::path::Path) -> Vec<Activity> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Activity>(l).ok())
        .collect()
}

/// Record one thing that happened on this machine.
///
/// The one call every writer uses. `object` is what it happened to in the
/// user's terms — a song title, not a path.
pub fn record(by: &str, app: &str, verb: &str, object: Option<String>) {
    record_detail(by, app, verb, object, None);
}

pub fn record_detail(
    by: &str,
    app: &str,
    verb: &str,
    object: Option<String>,
    detail: Option<serde_json::Value>,
) {
    log().add(Activity {
        at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        by: by.to_string(),
        device: Some(super::host_name()),
        app: app.to_string(),
        verb: verb.to_string(),
        object,
        detail,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(offset_days: i64) -> String {
        (chrono::Utc::now() - chrono::Duration::days(offset_days))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }

    fn activity(at: String, verb: &str, object: &str) -> Activity {
        Activity {
            at,
            by: "user".into(),
            device: Some("test-mac".into()),
            app: "dj".into(),
            verb: verb.into(),
            object: Some(object.into()),
            detail: None,
        }
    }

    #[test]
    fn appends_and_reads_back() {
        let dir = std::env::temp_dir().join(format!("ling-act-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let log = ActivityLog::new(dir.clone());

        log.add(activity(at(0), "delete", "三天三夜"));
        assert_eq!(log.revision(), 1);

        let reread = ActivityLog::new(dir.clone());
        reread.load();
        let lines = reread.lines(10);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("you delete 三天三夜"), "{lines:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_sweeps_days_outside_the_window_and_hands_them_back() {
        let dir = std::env::temp_dir().join(format!("ling-act-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let old_day = (chrono::Local::now() - chrono::Duration::days(9))
            .format("%Y-%m-%d")
            .to_string();
        let row = serde_json::to_string(&activity(at(9), "backup", "200 photos")).unwrap();
        std::fs::write(dir.join(format!("{old_day}.jsonl")), format!("{row}\n")).unwrap();

        let log = ActivityLog::new(dir.clone());
        log.add(activity(at(0), "sync", "12 songs"));
        let swept = log.load();

        assert_eq!(swept.len(), 1, "the aged-out day is handed back once");
        assert_eq!(swept[0].0, old_day);
        assert_eq!(swept[0].1.len(), 1);
        assert!(!dir.join(format!("{old_day}.jsonl")).exists(), "and deleted");
        assert_eq!(log.recent().len(), 1, "the window holds only today");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_torn_line_costs_one_entry_not_the_day() {
        let dir = std::env::temp_dir().join(format!("ling-act-torn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let good = serde_json::to_string(&activity(at(0), "clean", "40 GB")).unwrap();
        std::fs::write(
            dir.join(format!("{today}.jsonl")),
            format!("{{ not json\n{good}\n"),
        )
        .unwrap();

        let log = ActivityLog::new(dir.clone());
        log.load();
        assert_eq!(log.recent().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ago_reads_like_a_person() {
        assert_eq!(ago(5), "just now");
        assert_eq!(ago(180), "3m ago");
        assert_eq!(ago(7200), "2h ago");
        assert_eq!(ago(200_000), "2d ago");
    }
}
