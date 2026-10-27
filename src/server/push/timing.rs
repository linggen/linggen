//! When a raised line goes by push — pure, so the rules are tested, not
//! trusted. Decided 2026-10-27 (doc/phone-push-spec.md, "When the Mac pushes"):
//!
//! - warning / hand: two minutes after it is raised. A connected phone
//!   picks it up live and acks it first, and then no push goes.
//! - record (a morning line): at the phone's morning hour when raised before
//!   it, else thirty minutes after raising. The Watch brief is made at 01:00;
//!   the phone's 02:00 wake normally takes it and schedules its own morning
//!   line, and that ack stops the push. The push is for the night the wake
//!   never came — so it lands when the morning line would have.
//! - never after `until`: by default noon for a record line raised before
//!   noon, else six hours after raising.

use chrono::{DateTime, Local, NaiveTime, TimeZone};

use crate::engine::skill::NoticeTier;

pub(crate) const HAND_GRACE_SECS: u64 = 120;
pub(crate) const RECORD_AFTER_SECS: u64 = 30 * 60;
const LATE_UNTIL_SECS: u64 = 6 * 3600;
/// Pushes a phone gets a day, a warning never counted (the herald's budget).
pub(crate) const DAY_BUDGET: u32 = 2;
/// A refused send waits this long before it is tried again.
pub(crate) const RETRY_SECS: u64 = 10 * 60;

/// `HH:MM`, or 07:00 when the phone sent nothing usable.
pub(crate) fn morning_time(morning: &str) -> NaiveTime {
    NaiveTime::parse_from_str(morning.trim(), "%H:%M")
        .unwrap_or_else(|_| NaiveTime::from_hms_opt(7, 0, 0).expect("07:00"))
}

fn local(ts: u64) -> DateTime<Local> {
    Local
        .timestamp_opt(ts as i64, 0)
        .single()
        .unwrap_or_else(Local::now)
}

/// `at` on the day of `ts`, local time.
fn same_day_at(ts: u64, at: NaiveTime) -> u64 {
    let day = local(ts).date_naive().and_time(at);
    Local
        .from_local_datetime(&day)
        .earliest()
        .map(|d| d.timestamp().max(0) as u64)
        .unwrap_or(ts)
}

/// When the push for a line raised at `raised` goes to a phone whose morning
/// is `morning`.
pub(crate) fn due_at(tier: NoticeTier, raised: u64, morning: &str) -> u64 {
    match tier {
        NoticeTier::Warning | NoticeTier::Hand => raised + HAND_GRACE_SECS,
        NoticeTier::Record => {
            let morning_at = same_day_at(raised, morning_time(morning));
            if raised < morning_at {
                morning_at
            } else {
                raised + RECORD_AFTER_SECS
            }
        }
    }
}

/// When a line stops being worth saying, when its raiser named no time.
pub(crate) fn default_until(tier: NoticeTier, raised: u64) -> u64 {
    let noon = same_day_at(raised, NaiveTime::from_hms_opt(12, 0, 0).expect("noon"));
    match tier {
        NoticeTier::Record if raised < noon => noon,
        _ => raised + LATE_UNTIL_SECS,
    }
}

/// The local day a push counts against.
pub(crate) fn day_of(ts: u64) -> String {
    local(ts).format("%Y-%m-%d").to_string()
}

/// APNs priority: 10 lights the screen now, 5 lets iOS choose.
pub(crate) fn priority(tier: NoticeTier) -> u8 {
    match tier {
        NoticeTier::Record => 5,
        _ => 10,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(h: u32, m: u32) -> u64 {
        let d = Local::now()
            .date_naive()
            .and_hms_opt(h, m, 0)
            .expect("time");
        Local
            .from_local_datetime(&d)
            .earliest()
            .expect("local")
            .timestamp() as u64
    }

    #[test]
    fn the_nights_brief_goes_at_the_morning_hour() {
        assert_eq!(due_at(NoticeTier::Record, at(1, 12), "07:00"), at(7, 0));
        assert_eq!(due_at(NoticeTier::Record, at(1, 12), "06:30"), at(6, 30));
    }

    #[test]
    fn a_brief_made_after_the_morning_waits_half_an_hour() {
        assert_eq!(due_at(NoticeTier::Record, at(9, 0), "07:00"), at(9, 30));
    }

    #[test]
    fn a_hand_goes_after_two_minutes_whatever_the_hour() {
        assert_eq!(due_at(NoticeTier::Hand, at(1, 0), "07:00"), at(1, 2));
        assert_eq!(due_at(NoticeTier::Warning, at(15, 0), "07:00"), at(15, 2));
    }

    #[test]
    fn a_bad_morning_reads_as_seven() {
        assert_eq!(due_at(NoticeTier::Record, at(2, 0), "7am"), at(7, 0));
    }

    #[test]
    fn a_morning_line_is_not_worth_saying_after_noon() {
        assert_eq!(default_until(NoticeTier::Record, at(1, 0)), at(12, 0));
        assert_eq!(default_until(NoticeTier::Record, at(13, 0)), at(19, 0));
        assert_eq!(default_until(NoticeTier::Hand, at(1, 0)), at(7, 0));
    }

    #[test]
    fn record_is_passive_and_the_rest_light_the_screen() {
        assert_eq!(priority(NoticeTier::Record), 5);
        assert_eq!(priority(NoticeTier::Hand), 10);
        assert_eq!(priority(NoticeTier::Warning), 10);
    }
}
