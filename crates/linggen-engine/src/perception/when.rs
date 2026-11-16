//! The times a reader of the log may name: an ISO time, a day, or a small set
//! of words and offsets a model writes without thinking.
//!
//! - `now`
//! - `today`, `yesterday` — the start of that day, in the local zone
//! - `-30m`, `-6h`, `-3d` — that long before now (`d` is 24 hours)
//! - `2026-11-02` — the start of that day, local
//! - `2026-11-02T14:30` (or with seconds, or a space) — local
//! - `2026-11-02T14:30:00Z`, `…+02:00` — exact
//!
//! `now` and the zone are arguments, so a day boundary is tested in any zone.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};

/// What the forms above look like, for an error a model can act on.
pub const FORMS: &str = "an ISO time, a date (YYYY-MM-DD), today, yesterday, now, or -30m / -6h / -3d";

/// The moment `text` names, read at `now` in `now`'s zone.
pub fn resolve<Tz: TimeZone>(text: &str, now: &DateTime<Tz>) -> Result<DateTime<Utc>, String> {
    let t = text.trim();
    let zone = now.timezone();
    let found = match t {
        "now" => Some(now.with_timezone(&Utc)),
        "today" => start_of(&zone, now.date_naive()),
        "yesterday" => start_of(&zone, now.date_naive() - Duration::days(1)),
        _ => offset(t, now).or_else(|| absolute(t, &zone)),
    };
    found.ok_or_else(|| format!("cannot read '{t}' as a time; use {FORMS}"))
}

/// `-6h` and its kin.
fn offset<Tz: TimeZone>(t: &str, now: &DateTime<Tz>) -> Option<DateTime<Utc>> {
    let rest = t.strip_prefix('-')?;
    let unit = rest.chars().last()?;
    let n: i64 = rest[..rest.len() - unit.len_utf8()].parse().ok()?;
    let span = match unit {
        'm' => Duration::minutes(n),
        'h' => Duration::hours(n),
        'd' => Duration::days(n),
        _ => return None,
    };
    Some(now.with_timezone(&Utc) - span)
}

fn absolute<Tz: TimeZone>(t: &str, zone: &Tz) -> Option<DateTime<Utc>> {
    if let Ok(exact) = DateTime::parse_from_rfc3339(t) {
        return Some(exact.with_timezone(&Utc));
    }
    if let Ok(day) = NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        return start_of(zone, day);
    }
    let local = ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"]
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(t, f).ok())?;
    in_zone(zone, local)
}

/// Local midnight of `day`. A zone that skips midnight (a DST change at
/// 00:00) starts the day at its first real minute.
pub fn start_of<Tz: TimeZone>(zone: &Tz, day: NaiveDate) -> Option<DateTime<Utc>> {
    in_zone(zone, day.and_hms_opt(0, 0, 0)?)
}

fn in_zone<Tz: TimeZone>(zone: &Tz, local: NaiveDateTime) -> Option<DateTime<Utc>> {
    let first = zone
        .from_local_datetime(&local)
        .earliest()
        .or_else(|| zone.from_local_datetime(&(local + Duration::hours(1))).earliest())?;
    Some(first.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    /// 00:30 on 2026-11-03 at UTC+9 — still 2026-11-02 in UTC, which is where
    /// a reader that forgot the zone would put "today".
    fn just_after_midnight() -> DateTime<FixedOffset> {
        let tz = FixedOffset::east_opt(9 * 3600).unwrap();
        tz.with_ymd_and_hms(2026, 11, 3, 0, 30, 0).unwrap()
    }

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn today_and_yesterday_start_at_local_midnight() {
        let now = just_after_midnight();
        assert_eq!(resolve("today", &now).unwrap(), utc("2026-11-02T15:00:00Z"));
        assert_eq!(resolve("yesterday", &now).unwrap(), utc("2026-11-01T15:00:00Z"));
        assert_eq!(resolve("now", &now).unwrap(), utc("2026-11-02T15:30:00Z"));
    }

    #[test]
    fn offsets_count_back_from_now_across_midnight() {
        let now = just_after_midnight();
        assert_eq!(resolve("-30m", &now).unwrap(), utc("2026-11-02T15:00:00Z"));
        assert_eq!(resolve("-6h", &now).unwrap(), utc("2026-11-02T09:30:00Z"));
        assert_eq!(resolve("-3d", &now).unwrap(), utc("2026-10-30T15:30:00Z"));
    }

    #[test]
    fn dates_and_times_are_local_unless_they_say_otherwise() {
        let now = just_after_midnight();
        assert_eq!(resolve("2026-11-01", &now).unwrap(), utc("2026-10-31T15:00:00Z"));
        assert_eq!(resolve("2026-11-01T14:30", &now).unwrap(), utc("2026-11-01T05:30:00Z"));
        assert_eq!(resolve("2026-11-01 14:30:10", &now).unwrap(), utc("2026-11-01T05:30:10Z"));
        assert_eq!(resolve("2026-11-01T14:30:00Z", &now).unwrap(), utc("2026-11-01T14:30:00Z"));
    }

    #[test]
    fn anything_else_is_refused_with_the_forms() {
        let now = just_after_midnight();
        for bad in ["", "last week", "-6x", "6h", "-h", "2026-13-01"] {
            let err = resolve(bad, &now).unwrap_err();
            assert!(err.contains("today"), "{bad}: {err}");
        }
    }
}
