//! What the Mac keeps for push (`~/.linggen/push/state.json`): each paired
//! phone's registration and what it has told, and the lines waiting to go.
//! One file, rewritten whole under one lock — a handful of phones and lines.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::engine::skill::NoticeTier;

/// Told keys and day counts older than this are forgotten.
const KEEP_SECS: u64 = 30 * 86400;

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct PushState {
    /// Paired device id → its push registration.
    #[serde(default)]
    pub phones: BTreeMap<String, Phone>,
    #[serde(default)]
    pub queue: Vec<Pending>,
}

/// One phone, as it last registered (`POST /api/push/device`).
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct Phone {
    pub token: String,
    /// `sandbox` or `production`, from the phone's signed profile.
    pub env: String,
    /// The 32-byte key the note is sealed with, base64.
    pub key: String,
    pub language: String,
    /// Shown when the phone cannot open the note.
    pub fallback: String,
    /// `HH:MM` — when her morning line is said.
    pub morning: String,
    /// `<app>.<line>` → switched on, as the phone's herald says.
    #[serde(default)]
    pub lines: BTreeMap<String, bool>,
    /// `<app>.<line>:<key>` → when it was told (by the phone, or by a push).
    #[serde(default)]
    pub told: BTreeMap<String, u64>,
    /// `YYYY-MM-DD` → pushes counted against that day's budget.
    #[serde(default)]
    pub spent: BTreeMap<String, u32>,
    #[serde(default)]
    pub registered_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<LastPush>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct LastPush {
    pub at: u64,
    pub told: String,
    /// `sent`, or the relay's code (`not_configured`, `apns_error`, …).
    pub outcome: String,
}

/// A line raised and not yet gone to every phone it should reach.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct Pending {
    pub app: String,
    pub line: String,
    pub key: String,
    pub tier: NoticeTier,
    pub facts: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    pub raised_at: u64,
    pub until: u64,
    /// Her words, per language, once written — a restart never asks again.
    #[serde(default)]
    pub words: BTreeMap<String, (String, String)>,
    /// Device id → not before this (a refused send waits a while).
    #[serde(default)]
    pub retry_at: BTreeMap<String, u64>,
}

impl Pending {
    /// `<app>.<line>` — the herald's id for the line.
    pub fn herald_id(&self) -> String {
        format!("{}.{}", self.app, self.line)
    }

    /// `<app>.<line>:<key>` — the one string both doors mark as told.
    pub fn told_key(&self) -> String {
        told_key(&self.app, &self.line, &self.key)
    }
}

pub(crate) fn told_key(app: &str, line: &str, key: &str) -> String {
    format!("{app}.{line}:{key}")
}

static LOCK: Mutex<()> = Mutex::new(());

fn path() -> PathBuf {
    crate::paths::linggen_home().join("push").join("state.json")
}

pub(crate) fn load() -> PushState {
    std::fs::read(path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save(state: &PushState) -> std::io::Result<()> {
    let file = path();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(state).unwrap_or_default())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // It holds every phone's push key.
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(tmp, file)
}

/// Read, change, write — under the lock, so the tick and the API never lose
/// each other's writes.
pub(crate) fn modify<T>(f: impl FnOnce(&mut PushState) -> T) -> T {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut state = load();
    let out = f(&mut state);
    if let Err(e) = save(&state) {
        tracing::warn!("[push] state not saved: {e}");
    }
    out
}

/// Forget what is older than a month.
pub(crate) fn prune(phone: &mut Phone, now: u64) {
    phone
        .told
        .retain(|_, at| now.saturating_sub(*at) < KEEP_SECS);
    let oldest = chrono::DateTime::from_timestamp((now.saturating_sub(KEEP_SECS)) as i64, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    phone.spent.retain(|day, _| *day >= oldest);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_told_key_is_app_dot_line_colon_key() {
        assert_eq!(
            told_key("cfo", "watch", "watch|2026-10-27"),
            "cfo.watch:watch|2026-10-27"
        );
    }

    #[test]
    fn prune_forgets_what_a_month_has_passed() {
        let now = 1_800_000_000;
        let mut p = Phone::default();
        p.told.insert("old".into(), now - 31 * 86400);
        p.told.insert("new".into(), now - 86400);
        p.spent.insert("2020-01-01".into(), 2);
        p.spent.insert("2027-01-15".into(), 1);
        prune(&mut p, now);
        assert_eq!(p.told.keys().collect::<Vec<_>>(), vec!["new"]);
        assert_eq!(p.spent.keys().collect::<Vec<_>>(), vec!["2027-01-15"]);
    }
}
