//! The day's session: one session a day that every app's chat, Yinyue's
//! unprompted moments and the phone's hand-offs are part of, when
//! `agent.one_session_a_day` is on (`doc/one-ling-spec.md` § One session a
//! day). What it is called and when a day begins; who resolves it is the
//! server's (`server::day_session`).
//!
//! A day begins at 04:00 local, so late-night play stays in one day. Its id
//! is fixed by the day — `sess-day-YYYY-MM-DD` — so a restart finds it again
//! without a record. It does not roll to segments as Yinyue's own thread
//! does: a full thread compacts, as every shared session's does.

use super::sessions::{SessionMember, SessionMeta};
use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone};

/// The hour, local, a new day begins.
pub const DAY_STARTS_AT_HOUR: i64 = 4;

const PREFIX: &str = "sess-day-";
/// What follows the date in the id of a day that is not the builder's own:
/// another person's day on this engine is their own session.
const PERSON: &str = "-p-";
/// How much of a person's id their day's id keeps.
const PERSON_MAX: usize = 64;

/// The day `now` belongs to: the calendar date of `now` four hours earlier.
pub fn day_of<Tz: TimeZone>(now: &DateTime<Tz>) -> NaiveDate {
    (now.clone() - Duration::hours(DAY_STARTS_AT_HOUR)).date_naive()
}

/// The day it is now, on this machine's clock.
pub fn today() -> NaiveDate {
    day_of(&Local::now())
}

/// The id of `day`'s session.
pub fn id_of(day: NaiveDate) -> String {
    format!("{PREFIX}{}", day.format("%Y-%m-%d"))
}

/// The id of `day`'s session for `person`: `None` is the builder's own
/// ([`id_of`]); anyone else's day is theirs, its id naming them.
pub fn id_for(day: NaiveDate, person: Option<&str>) -> String {
    match person.map(person_key).filter(|k| !k.is_empty()) {
        Some(key) => format!("{}{PERSON}{key}", id_of(day)),
        None => id_of(day),
    }
}

/// A person's id as their day's id keeps it: letters, digits and `_`.
fn person_key(person: &str) -> String {
    person
        .chars()
        .take(PERSON_MAX)
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect()
}

/// Whether `session_id` is a day's session.
pub fn is_day_session(session_id: &str) -> bool {
    day_in(session_id).is_some()
}

/// The day a day's session id names.
pub fn day_in(session_id: &str) -> Option<NaiveDate> {
    let rest = session_id.strip_prefix(PREFIX)?;
    let date = rest.get(..10)?;
    let person = &rest[10..];
    let named = person.is_empty()
        || person.strip_prefix(PERSON).is_some_and(|k| {
            !k.is_empty() && k.len() <= PERSON_MAX && k.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        });
    if !named {
        return None;
    }
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

/// A new day's session as it is first kept: Ling seated as its lead (she
/// answers every message not addressed to Yinyue), Yinyue seated when she
/// first speaks in it, home's folder `cwd`. Made by `agent`, as Yinyue's own
/// thread is: the memory scan reads only a person's own chats (`user`), and
/// app chats and her thread were never read by it — nor is the day, which
/// holds a private app's talk.
/// `person`: whose day it is (`None`: the builder's own); a page lists it
/// only to them.
pub fn new_meta(session_id: &str, day: NaiveDate, cwd: &str, (lead, person): (&str, Option<&str>)) -> SessionMeta {
    SessionMeta {
        user_id: person.map(str::to_string),
        id: session_id.to_string(),
        title: format!("Linggen · {}", day.format("%Y-%m-%d")),
        created_at: crate::util::now_ts_secs(),
        creator: "agent".to_string(),
        cwd: cwd.to_string(),
        title_locked: true,
        agents: vec![SessionMember::new(lead)],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn at(s: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(s).unwrap()
    }

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn a_day_begins_at_four_in_the_morning_local() {
        assert_eq!(day_of(&at("2026-11-16T03:59:59+08:00")), date("2026-11-15"));
        assert_eq!(day_of(&at("2026-11-16T04:00:00+08:00")), date("2026-11-16"));
        assert_eq!(day_of(&at("2026-11-16T23:59:59+08:00")), date("2026-11-16"));
        assert_eq!(day_of(&at("2026-11-17T00:30:00+08:00")), date("2026-11-16"));
        // Local, not UTC: the same instant is another day elsewhere.
        assert_eq!(day_of(&at("2026-11-16T03:00:00-05:00")), date("2026-11-15"));
    }

    #[test]
    fn the_id_names_the_day() {
        let d = date("2026-11-16");
        assert_eq!(id_of(d), "sess-day-2026-11-16");
        assert_eq!(day_in("sess-day-2026-11-16"), Some(d));
        assert!(is_day_session("sess-day-2026-11-16"));
        assert!(!is_day_session("sess-yinyue-2026-11-16"));
        assert!(!is_day_session("sess-day-2026-11-16x"));
        assert!(!is_day_session("sess-day-2026-11-16-2"));
        assert!(!is_day_session("sess-1758700000-abcd1234"));
    }

    /// Each person has a day of their own; the builder's keeps its id.
    #[test]
    fn a_day_is_a_persons() {
        let d = date("2026-11-16");
        assert_eq!(id_for(d, None), "sess-day-2026-11-16");
        let theirs = id_for(d, Some("U-42/x"));
        assert_eq!(theirs, "sess-day-2026-11-16-p-u_42_x");
        assert_ne!(theirs, id_for(d, Some("u-43")));
        assert_eq!(day_in(&theirs), Some(d));
        assert!(!is_day_session("sess-day-2026-11-16-p-"));
        assert!(!is_day_session("sess-day-2026-11-16-p-A"));
        let meta = new_meta(&theirs, d, "/w", ("ling", Some("U-42/x")));
        assert_eq!(meta.user_id.as_deref(), Some("U-42/x"));
    }

    #[test]
    fn a_new_day_seats_its_lead_and_names_the_day() {
        let meta = new_meta("sess-day-2026-11-16", date("2026-11-16"), "/w", ("ling", None));
        assert_eq!(meta.title, "Linggen · 2026-11-16");
        assert_eq!(meta.agents.len(), 1);
        assert_eq!(meta.agents[0].id, "ling");
        assert_eq!(meta.skill, None);
        assert!(meta.title_locked);
        assert_eq!(meta.creator, "agent", "the memory scan never reads the day");
    }
}
