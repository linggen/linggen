//! Daily samples, and what they say about where this machine is heading.
//!
//! Some conditions are about slope, not level: "full in a day" needs yesterday
//! and the day before (`doc/perception-spec.md` §5). A handful of numbers a
//! day, kept a couple of weeks, is enough — without it the best an agent can
//! say is "your disk is nearly full", which is the kind of line that makes an
//! assistant feel dumb.
//!
//! Same shape and same arithmetic as the phone's `SpaceTrend`, so the two
//! machines forecast alike.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// The day key is the activity log's format — one definition, because a sample
// and a day file are the same day.
use super::activity::parse_day;

/// A fortnight is long enough to see a slope and short enough that a Mac the
/// user cleared six weeks ago is not still judged on its old one.
///
/// Days, not samples. A Mac used a few days a month accumulates fourteen
/// samples spanning half a year, and the oldest of them would then anchor a
/// "filling at" line about this week.
const MAX_DAYS: i64 = 14;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    pub day: String,
    /// Bytes free on the volume the user's home lives on.
    pub free: u64,
    pub total: u64,
}

fn path() -> PathBuf {
    super::activity::activity_dir().join("trend.json")
}

fn read_all() -> Vec<Sample> {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<Sample>>(&t).ok())
        .unwrap_or_default()
}

/// Record today's numbers. One sample per calendar day — a daemon restarted
/// five times in an afternoon must not read as five days of history, so the
/// last value of the day wins.
pub fn sample(free: u64, total: u64) {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut samples = read_all();
    samples.retain(|s| s.day != today);
    samples.push(Sample {
        day: today,
        free,
        total,
    });
    samples.sort_by(|a, b| a.day.cmp(&b.day));
    // Retained by age: a sample we cannot date cannot be part of a slope, and a
    // sample older than the window is the past being asked about the present.
    let cutoff = chrono::Local::now().date_naive() - chrono::Duration::days(MAX_DAYS - 1);
    samples.retain(|s| parse_day(&s.day).is_some_and(|d| d >= cutoff));
    let dir = super::activity::activity_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(body) = serde_json::to_vec_pretty(&samples) {
        let _ = std::fs::write(path(), body);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Forecast {
    pub bytes_per_day: u64,
    pub days_until_full: i64,
    pub span_days: i64,
}

/// `None` when there is nothing to say yet, or when the disk is not filling.
///
/// Measured across the whole retained window rather than the last two samples:
/// one day where a big build landed and was cleaned would otherwise swing the
/// estimate wildly, and a forecast that jumps between "full in 4 days" and "not
/// filling" teaches the user to ignore it.
pub fn forecast() -> Option<Forecast> {
    slope(&read_all())
}

fn slope(samples: &[Sample]) -> Option<Forecast> {
    let first = samples.first()?;
    let last = samples.last()?;
    let span = (parse_day(&last.day)? - parse_day(&first.day)?).num_days();
    if span < 1 {
        return None;
    }
    let consumed = first.free.checked_sub(last.free)?;
    if consumed == 0 {
        return None; // holding steady, or getting emptier
    }
    let per_day = consumed as f64 / span as f64;
    Some(Forecast {
        bytes_per_day: per_day.round() as u64,
        days_until_full: (last.free as f64 / per_day).round() as i64,
        span_days: span,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(day: &str, free_gb: u64) -> Sample {
        Sample {
            day: day.into(),
            free: free_gb * 1_000_000_000,
            total: 2_000_000_000_000,
        }
    }

    #[test]
    fn no_forecast_from_a_single_day() {
        assert_eq!(slope(&[s("2026-08-01", 100)]), None);
    }

    #[test]
    fn emptying_disks_forecast_nothing() {
        assert_eq!(slope(&[s("2026-08-01", 50), s("2026-08-05", 90)]), None);
    }

    #[test]
    fn a_filling_disk_says_when() {
        let f = slope(&[s("2026-08-01", 50), s("2026-08-05", 30)]).unwrap();
        assert_eq!(f.span_days, 4);
        assert_eq!(f.bytes_per_day, 5_000_000_000);
        assert_eq!(f.days_until_full, 6);
    }

    #[test]
    fn the_whole_window_sets_the_slope_not_the_last_hop() {
        // A day that dropped hard and recovered must not dominate.
        let f = slope(&[
            s("2026-08-01", 100),
            s("2026-08-02", 10),
            s("2026-08-11", 90),
        ])
        .unwrap();
        assert_eq!(f.span_days, 10);
        assert_eq!(f.days_until_full, 90);
    }
}
