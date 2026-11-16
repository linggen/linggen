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
//!
//! A time ahead ([`resolve_ahead`], a note's look-again) is read with its own
//! small set: `+40m`, `+3h`, `+2d`; `HH:MM` (the next time the clock reads
//! it); `tomorrow HH:MM`; and the dated forms above. A moment already past is
//! refused, never moved.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};

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

/// The forms a time ahead may take, for an error a model can act on.
pub const AHEAD_FORMS: &str =
    "+40m, +3h, +2d, HH:MM (the next time the clock reads it), tomorrow HH:MM, or YYYY-MM-DD HH:MM";

/// The moment ahead `text` names, read at `now` in `now`'s zone. A moment
/// that is not after `now` is refused.
pub fn resolve_ahead<Tz: TimeZone>(text: &str, now: &DateTime<Tz>) -> Result<DateTime<Utc>, String> {
    let t = text.trim();
    let zone = now.timezone();
    let found = ahead(t, now)
        .or_else(|| clock(t, now))
        .or_else(|| tomorrow_at(t, now))
        .or_else(|| absolute(t, &zone));
    let Some(at) = found else {
        return Err(format!("cannot read '{t}' as a time ahead; use {AHEAD_FORMS}"));
    };
    if at <= now.with_timezone(&Utc) {
        return Err(format!("'{t}' has already passed; give a time ahead ({AHEAD_FORMS})"));
    }
    Ok(at)
}

/// `-6h` and its kin.
fn offset<Tz: TimeZone>(t: &str, now: &DateTime<Tz>) -> Option<DateTime<Utc>> {
    Some(now.with_timezone(&Utc) - span(t.strip_prefix('-')?)?)
}

/// `+40m` and its kin.
fn ahead<Tz: TimeZone>(t: &str, now: &DateTime<Tz>) -> Option<DateTime<Utc>> {
    Some(now.with_timezone(&Utc) + span(t.strip_prefix('+')?)?)
}

/// `40m`, `6h`, `3d` (`d` is 24 hours).
fn span(rest: &str) -> Option<Duration> {
    let unit = rest.chars().last()?;
    let n: i64 = rest[..rest.len() - unit.len_utf8()].parse().ok()?;
    match unit {
        'm' => Some(Duration::minutes(n)),
        'h' => Some(Duration::hours(n)),
        'd' => Some(Duration::days(n)),
        _ => None,
    }
}

/// `HH:MM`: the next time the local clock reads it — today, or tomorrow
/// when today's has passed.
fn clock<Tz: TimeZone>(t: &str, now: &DateTime<Tz>) -> Option<DateTime<Utc>> {
    let time = NaiveTime::parse_from_str(t, "%H:%M").ok()?;
    let zone = now.timezone();
    let today = in_zone(&zone, now.date_naive().and_time(time))?;
    if today > now.with_timezone(&Utc) {
        return Some(today);
    }
    in_zone(&zone, (now.date_naive() + Duration::days(1)).and_time(time))
}

/// `tomorrow HH:MM`, local.
fn tomorrow_at<Tz: TimeZone>(t: &str, now: &DateTime<Tz>) -> Option<DateTime<Utc>> {
    let rest = t.strip_prefix("tomorrow")?.trim();
    let time = NaiveTime::parse_from_str(rest, "%H:%M").ok()?;
    in_zone(&now.timezone(), (now.date_naive() + Duration::days(1)).and_time(time))
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

    /// 23:50 on 2026-11-02 at UTC+9: ten minutes before the local day turns.
    fn just_before_midnight() -> DateTime<FixedOffset> {
        let tz = FixedOffset::east_opt(9 * 3600).unwrap();
        tz.with_ymd_and_hms(2026, 11, 2, 23, 50, 0).unwrap()
    }

    #[test]
    fn a_time_ahead_counts_forward_and_crosses_the_local_midnight() {
        let now = just_before_midnight();
        assert_eq!(resolve_ahead("+40m", &now).unwrap(), utc("2026-11-02T15:30:00Z"));
        assert_eq!(resolve_ahead("+3h", &now).unwrap(), utc("2026-11-02T17:50:00Z"));
        assert_eq!(resolve_ahead("+2d", &now).unwrap(), utc("2026-11-04T14:50:00Z"));
        // Ten minutes from now, local: still today.
        assert_eq!(resolve_ahead("23:59", &now).unwrap(), utc("2026-11-02T14:59:00Z"));
        // Already passed today: the next time the clock reads it, tomorrow.
        assert_eq!(resolve_ahead("10:00", &now).unwrap(), utc("2026-11-03T01:00:00Z"));
        // Tomorrow is the local day after, not the UTC one.
        assert_eq!(resolve_ahead("tomorrow 00:30", &now).unwrap(), utc("2026-11-02T15:30:00Z"));
        assert_eq!(resolve_ahead("2026-11-05 09:00", &now).unwrap(), utc("2026-11-05T00:00:00Z"));
    }

    #[test]
    fn a_time_ahead_that_has_passed_is_refused_not_moved() {
        let now = just_before_midnight();
        for past in ["2026-11-02 23:00", "2026-11-01", "+0m", "2026-11-02T14:00:00Z"] {
            let err = resolve_ahead(past, &now).unwrap_err();
            assert!(err.contains("already passed"), "{past}: {err}");
        }
        for bad in ["", "soon", "-30m", "40m", "tomorrow", "25:00"] {
            let err = resolve_ahead(bad, &now).unwrap_err();
            assert!(err.contains("+40m"), "{bad}: {err}");
        }
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
