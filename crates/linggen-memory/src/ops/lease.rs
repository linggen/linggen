//! The dream-lease sidecar, `<data_dir>/memory/.dream-lease.json`: one
//! executor per day (`{day: {holder, expires_at}}`). The `dream_lease` verb
//! lives in `http::lease`; `remember_day` refuses a day another holder leased.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::MemError;

pub const LEASE_FOR_MINUTES: i64 = 30;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Lease {
    pub holder: String,
    pub expires_at: DateTime<Utc>,
}

type Leases = BTreeMap<String, Lease>;

fn path(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join(".dream-lease.json")
}

fn load(data_dir: &Path) -> Leases {
    std::fs::read_to_string(path(data_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(data_dir: &Path, leases: &Leases) -> std::io::Result<()> {
    let p = path(data_dir);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(leases).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(tmp, p)
}

/// One process at a time edits the lease file.
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug, PartialEq)]
pub enum Verdict {
    Granted(Lease),
    Held(Lease),
    Released,
}

/// The live lease on `day` held by someone other than `holder`, if any.
pub fn held_by_other(
    data_dir: &Path,
    day: &str,
    holder: &str,
    now: DateTime<Utc>,
) -> Option<Lease> {
    load(data_dir)
        .get(day)
        .filter(|l| l.expires_at > now && l.holder != holder)
        .cloned()
}

pub fn decide(
    data_dir: &Path,
    day: &str,
    holder: &str,
    action: &str,
    now: DateTime<Utc>,
) -> std::io::Result<Verdict> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut leases = load(data_dir);
    leases.retain(|_, l| l.expires_at > now);
    if let Some(other) = leases.get(day).filter(|l| l.holder != holder) {
        return Ok(Verdict::Held(other.clone()));
    }
    let verdict = match action {
        "release" => {
            leases.remove(day);
            Verdict::Released
        }
        _ => {
            let lease = Lease {
                holder: holder.to_string(),
                expires_at: now + Duration::minutes(LEASE_FOR_MINUTES),
            };
            leases.insert(day.to_string(), lease.clone());
            Verdict::Granted(lease)
        }
    };
    save(data_dir, &leases)?;
    Ok(verdict)
}

pub fn busy(day: &str, l: &Lease) -> MemError {
    MemError::conflict(format!(
        "day {day} is being dreamed by {} (until {})",
        l.holder,
        l.expires_at.to_rfc3339()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_executor_is_refused_until_release_or_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let day = "2026-10-20";
        assert!(matches!(
            decide(dir.path(), day, "engine@mac", "acquire", now).unwrap(),
            Verdict::Granted(_)
        ));
        let held = decide(dir.path(), day, "claude-code@mac", "acquire", now).unwrap();
        assert!(matches!(held, Verdict::Held(ref l) if l.holder == "engine@mac"));
        assert!(held_by_other(dir.path(), day, "claude-code@mac", now).is_some());
        assert!(held_by_other(dir.path(), day, "engine@mac", now).is_none());

        // Renewal is the holder's; another day is free.
        assert!(matches!(
            decide(dir.path(), day, "engine@mac", "renew", now).unwrap(),
            Verdict::Granted(_)
        ));
        assert!(matches!(
            decide(dir.path(), "2026-10-19", "claude-code@mac", "acquire", now).unwrap(),
            Verdict::Granted(_)
        ));

        // Expired leases free themselves.
        let later = now + Duration::minutes(LEASE_FOR_MINUTES + 1);
        assert!(matches!(
            decide(dir.path(), day, "claude-code@mac", "acquire", later).unwrap(),
            Verdict::Granted(_)
        ));

        // Only the holder releases.
        assert!(matches!(
            decide(dir.path(), day, "engine@mac", "release", later).unwrap(),
            Verdict::Held(_)
        ));
        assert_eq!(
            decide(dir.path(), day, "claude-code@mac", "release", later).unwrap(),
            Verdict::Released
        );
    }
}
