//! The day-state sidecar, `<data_dir>/memory/.days.json`, and local-day
//! attribution: a row belongs to the local calendar day of its
//! `effective_timestamp()`. The `days` verbs live in `http::days`.

use crate::error::MemError;
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One day's dream record. Counts accumulate across re-remembers of the
/// same day (late-arriving rows re-pend a day; a follow-up pass adds to
/// the totals rather than overwriting them).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DayRecord {
    /// When a harvest (session backfill) pass last covered this day.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub harvested_at: Option<DateTime<Utc>>,
    /// When a remember pass last judged this day's rows. The judged set
    /// is exactly the rows with `created_at < remembered_at`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub remembered_at: Option<DateTime<Utc>>,
    /// Total rows judged across all remember passes.
    #[serde(default)]
    pub judged: u32,
    /// Total rows promoted to semantic across all remember passes.
    #[serde(default)]
    pub promoted: u32,
    /// Total rows evicted by the forget sweep.
    #[serde(default)]
    pub forgotten: u32,
}

/// The `.days.json` sidecar — `{"days": {"YYYY-MM-DD": record}}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaysState {
    #[serde(default)]
    pub days: BTreeMap<String, DayRecord>,
}

pub(crate) fn days_path(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join(".days.json")
}

/// Read-or-default, mirroring `config::load` — a missing/corrupt file
/// never blocks the store.
pub(crate) async fn load(data_dir: &Path) -> DaysState {
    let Ok(bytes) = tokio::fs::read(days_path(data_dir)).await else {
        return DaysState::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub(crate) async fn save(data_dir: &Path, state: &DaysState) -> Result<(), std::io::Error> {
    let path = days_path(data_dir);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(state).expect("DaysState serializes");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(tmp, path).await?;
    Ok(())
}

/// Local calendar day of a UTC instant, as the canonical `YYYY-MM-DD` key.
pub(crate) fn local_day(ts: DateTime<Utc>) -> String {
    ts.with_timezone(&Local).format("%Y-%m-%d").to_string()
}

pub(crate) fn today_local() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// Parse a `YYYY-MM-DD` day key, rejecting anything else.
pub(crate) fn parse_day(s: &str) -> Result<NaiveDate, MemError> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|_| MemError::bad_request(format!("invalid day {s:?}: expected YYYY-MM-DD")))
}

/// UTC bounds `[start, end)` of a local calendar day. DST folds/gaps
/// resolve to the earliest valid instant.
pub(crate) fn local_day_bounds(day: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
    let start_naive = day.and_hms_opt(0, 0, 0).expect("midnight exists");
    let end_naive = (day + chrono::Days::new(1))
        .and_hms_opt(0, 0, 0)
        .expect("midnight exists");
    let to_utc = |naive| {
        Local
            .from_local_datetime(&naive)
            .earliest()
            .map(|dt| dt.with_timezone(&Utc))
            // A DST gap at midnight (rare, but real in some zones):
            // fall back to interpreting the naive time as UTC.
            .unwrap_or_else(|| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
    };
    (to_utc(start_naive), to_utc(end_naive))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut s = DaysState::default();
            s.days.insert(
                "2026-07-01".into(),
                DayRecord {
                    remembered_at: Some(Utc::now()),
                    judged: 12,
                    promoted: 3,
                    ..Default::default()
                },
            );
            save(dir.path(), &s).await.unwrap();
            let loaded = load(dir.path()).await;
            let r = loaded.days.get("2026-07-01").unwrap();
            assert_eq!(r.judged, 12);
            assert_eq!(r.promoted, 3);
            assert!(r.remembered_at.is_some());
            assert!(r.harvested_at.is_none());
        });
    }

    #[test]
    fn missing_file_loads_default() {
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let loaded = load(dir.path()).await;
            assert!(loaded.days.is_empty());
        });
    }

    #[test]
    fn local_day_bounds_cover_exactly_one_day() {
        let d = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        let (start, end) = local_day_bounds(d);
        assert_eq!(end - start, chrono::Duration::days(1));
        // The local rendering of the start instant is the day itself.
        assert_eq!(local_day(start), "2026-07-01");
    }

    #[test]
    fn parse_day_rejects_garbage() {
        assert!(parse_day("2026-07-01").is_ok());
        // chrono's %m/%d tolerate missing zero-padding — fine leniency.
        assert!(parse_day("2026-7-1").is_ok());
        assert!(parse_day("yesterday").is_err());
        assert!(parse_day("2026-13-40").is_err());
        assert!(parse_day("").is_err());
    }
}
