//! Cron expression validation + parsing for mission schedules.
//!
//! Linggen accepts 5-field cron (`min hour dom month dow`); the
//! underlying `cron` crate wants 7. This module owns that
//! translation plus the day-of-week renumbering (standard cron counts
//! Sunday as 0 or 7, the crate as 1) and exposes two functions the rest
//! of the codebase calls: `validate_cron` (yes/no) and `parse_cron`
//! (typed schedule).

use anyhow::{bail, Result};
use std::collections::BTreeSet;

/// Convert a 5-field cron expression to the 7-field format the
/// `cron` crate expects, renumbering the day-of-week field.
pub(super) fn to_seven_field(schedule: &str) -> Result<String> {
    let fields: Vec<&str> = schedule.split_whitespace().collect();
    if fields.len() != 5 {
        bail!(
            "Invalid cron expression '{}': expected 5 fields (min hour dom month dow)",
            schedule
        );
    }
    let dow = fields[4]
        .split(',')
        .map(crate_dow_part)
        .collect::<Vec<_>>()
        .join(",");

    Ok(format!(
        "0 {} {} {} {} {} *",
        fields[0], fields[1], fields[2], fields[3], dow
    ))
}

/// One day-of-week list item, from standard cron (0 or 7 = Sunday,
/// 1 = Monday … 6 = Saturday) to the crate's numbering (1 = Sunday …
/// 7 = Saturday). Numbers, ranges and their steps become the explicit
/// days; names and `*` pass through — they mean the same days in both
/// (`*/2` is Sun, Tue, Thu, Sat either way). Anything malformed passes
/// through too, for the crate to reject.
fn crate_dow_part(part: &str) -> String {
    let (base, step) = match part.split_once('/') {
        Some((base, step)) => (base, Some(step)),
        None => (part, None),
    };
    let Some((first, last)) = numeric_dow_span(base, step.is_some()) else {
        return part.to_string();
    };
    let step = match step.map(|s| s.trim().parse::<usize>()) {
        None => 1,
        Some(Ok(n)) if n > 0 => n,
        Some(_) => return part.to_string(),
    };
    let days: BTreeSet<u8> = (first..=last).step_by(step).map(|d| d % 7 + 1).collect();
    days.iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// The standard-cron day numbers a list item's base covers: `a-b`, or `a`
/// alone (through Saturday when a step follows, as `a/n` means).
fn numeric_dow_span(base: &str, stepped: bool) -> Option<(u8, u8)> {
    let (first, last) = match base.split_once('-') {
        Some((a, b)) => (a.trim().parse::<u8>().ok()?, b.trim().parse::<u8>().ok()?),
        None => {
            let a = base.trim().parse::<u8>().ok()?;
            (a, if stepped { 6.max(a) } else { a })
        }
    };
    (first <= last && last <= 7).then_some((first, last))
}

pub fn validate_cron(schedule: &str) -> Result<()> {
    let seven = to_seven_field(schedule)?;
    seven
        .parse::<cron::Schedule>()
        .map_err(|e| anyhow::anyhow!("Invalid cron expression '{}': {}", schedule, e))?;
    Ok(())
}

pub fn parse_cron(schedule: &str) -> Result<cron::Schedule> {
    let seven = to_seven_field(schedule)?;
    seven
        .parse::<cron::Schedule>()
        .map_err(|e| anyhow::anyhow!("Invalid cron expression '{}': {}", schedule, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, TimeZone, Utc, Weekday};

    /// The weekdays of the next `n` fires, starting Sunday 2026-09-13.
    fn next_days(schedule: &str, n: usize) -> Vec<Weekday> {
        let from = Utc.with_ymd_and_hms(2026, 9, 12, 23, 0, 0).unwrap();
        parse_cron(schedule)
            .unwrap()
            .after(&from)
            .take(n)
            .map(|t| t.weekday())
            .collect()
    }

    #[test]
    fn weekdays_are_monday_to_friday() {
        use Weekday::*;
        assert_eq!(next_days("0 9 * * 1-5", 6), [Mon, Tue, Wed, Thu, Fri, Mon]);
        assert_eq!(
            next_days("0 9 * * mon-fri", 6),
            [Mon, Tue, Wed, Thu, Fri, Mon]
        );
    }

    #[test]
    fn sunday_is_zero_or_seven() {
        use Weekday::*;
        assert_eq!(next_days("0 0 * * 0", 2), [Sun, Sun]);
        assert_eq!(next_days("0 0 * * 7", 2), [Sun, Sun]);
        assert_eq!(next_days("0 0 * * 6", 1), [Sat]);
    }

    #[test]
    fn ranges_lists_and_steps_keep_their_days() {
        use Weekday::*;
        assert_eq!(next_days("0 0 * * 5-7", 3), [Sun, Fri, Sat]);
        assert_eq!(next_days("0 0 * * 1,3", 3), [Mon, Wed, Mon]);
        assert_eq!(next_days("0 0 * * 1-5/2", 4), [Mon, Wed, Fri, Mon]);
        assert_eq!(next_days("0 0 * * */2", 4), [Sun, Tue, Thu, Sat]);
        assert_eq!(next_days("0 0 * * 0-6", 7).len(), 7);
    }

    #[test]
    fn a_bad_day_is_still_rejected() {
        assert!(validate_cron("0 0 * * 8").is_err());
        assert!(validate_cron("0 0 * * 5-2").is_err());
    }
}
