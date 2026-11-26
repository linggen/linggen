//! Expiry of episodic rows — "second chance". THE one place that decides it.
//!
//! A row lives [`DEFAULT_TTL_DAYS`] days (the user sets it; `ops::config`),
//! counted from its life clock: `updated_at`, else the time it entered
//! (`occurred_at ?? created_at`) — so a row from before `updated_at` was
//! stamped needs no migration. Past its life:
//!
//! - never used (`used == 0`) → [`Verdict::Delete`];
//! - used (`used > 0`, a reply cited it) → [`Verdict::Renew`]: the sweep sets
//!   `used = 0` and `updated_at = now`, one more period.
//!
//! `used` is read ONLY here: not by recall ranking, not by the prompt, not by
//! promotion. `recalled` (pushed into a turn) counts for nothing at all.
//! Rows of a day the dream has not judged are never reaching this function
//! (the sweep filters them first).

use chrono::{DateTime, Duration, Utc};

use super::types::Memory;

/// Default life of an episodic row, in days. The only copy of this number.
pub const DEFAULT_TTL_DAYS: u32 = 30;

/// Settings range for the user's choice.
pub const TTL_DAYS_RANGE: std::ops::RangeInclusive<u32> = 1..=365;

/// What the sweep does with one judged episodic row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Keep,
    Delete,
    /// Heat back to 0 and `updated_at` to now.
    Renew,
}

/// When the row's life started: last touched, else when it entered.
pub fn life_clock(row: &Memory) -> DateTime<Utc> {
    row.updated_at.unwrap_or_else(|| row.effective_timestamp())
}

/// The second-chance rule.
pub fn verdict(row: &Memory, now: DateTime<Utc>, ttl_days: u32) -> Verdict {
    if now - life_clock(row) <= Duration::days(ttl_days as i64) {
        Verdict::Keep
    } else if row.used == 0 {
        Verdict::Delete
    } else {
        Verdict::Renew
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::{MemoryType, Origin};

    fn row(age_days: i64, used: u32) -> Memory {
        let mut m = Memory::new("x", MemoryType::Fact, Origin::Derived);
        m.created_at = Utc::now() - Duration::days(age_days);
        m.used = used;
        m
    }

    #[test]
    fn unexpired_is_kept_even_cold() {
        assert_eq!(verdict(&row(29, 0), Utc::now(), 30), Verdict::Keep);
    }

    #[test]
    fn expired_cold_is_deleted_warm_is_renewed() {
        assert_eq!(verdict(&row(31, 0), Utc::now(), 30), Verdict::Delete);
        assert_eq!(verdict(&row(31, 3), Utc::now(), 30), Verdict::Renew);
    }

    #[test]
    fn updated_at_wins_over_the_entry_time() {
        let mut m = row(90, 0);
        m.updated_at = Some(Utc::now() - Duration::days(1));
        assert_eq!(verdict(&m, Utc::now(), 30), Verdict::Keep);
    }

    #[test]
    fn recalled_alone_changes_nothing() {
        let mut m = row(31, 0);
        m.recalled = 50;
        assert_eq!(verdict(&m, Utc::now(), 30), Verdict::Delete);
    }
}
