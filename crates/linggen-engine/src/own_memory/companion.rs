//! Yinyue's day: the dates in the person's own record that fall around
//! today, handed to her once with her prompt. The one place that builds this
//! context ([`today`]); `prompt::owner_block` is its only caller, and only for
//! the companion (`tests::the_days_context_is_built_in_one_place`).
//!
//! Mechanical, and only facts: which recurring days (a birthday, an
//! anniversary — a note with a `yearly` day) are today or close, and which
//! moments and chronicle lines happened on this date in past years. Whether
//! to speak of one, when and how gently, is hers. A note marked no-raise, and
//! a void one, never appears here.

use super::fence::{self, Audience, Seen};
use super::format::{Kind, Scope};
use super::Store;
use chrono::{Datelike, NaiveDate};

/// How many days ahead a recurring day is listed.
const COMING_UP_DAYS: i64 = 7;
/// Most "on this day" lines.
const MOST_PAST_LINES: usize = 6;

/// The lines for the day of `now` (epoch seconds), or nothing when no date in
/// the record falls on or near it.
pub fn today(store: &Store, now: i64) -> String {
    let Ok(date) = NaiveDate::parse_from_str(&super::day_of(now), "%Y-%m-%d") else {
        return String::new();
    };
    let (all, _) = fence::seen(store, Audience::Companion);
    let usable: Vec<&Seen> = all.iter().filter(|s| s.entry.void.is_none() && !s.entry.no_raise).collect();
    let coming = coming_up(&usable, date);
    let past = on_this_day(&usable, date);
    if coming.is_empty() && past.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "\n# Today, {date} — dates in the person's own record\n\
These are facts, not errands: speak of one only if it fits the moment, rarely and gently, as a good friend would, \
and drop it the moment the person turns away. Open the note (memory_notes op=read id=…) to see how it felt.\n"
    );
    if !coming.is_empty() {
        out.push_str("Coming up:\n");
        coming.iter().for_each(|l| out.push_str(&format!("{l}\n")));
    }
    if !past.is_empty() {
        out.push_str("On this day in past years:\n");
        past.iter().for_each(|l| out.push_str(&format!("{l}\n")));
    }
    out
}

/// Recurring days from today to [`COMING_UP_DAYS`] ahead, soonest first.
fn coming_up(notes: &[&Seen], today: NaiveDate) -> Vec<String> {
    let mut found: Vec<(i64, String)> = Vec::new();
    for s in notes {
        let Some(day) = s.entry.yearly.as_deref() else {
            continue;
        };
        let Some((first_year, month, dom)) = parts_of(day) else {
            continue;
        };
        for year in [today.year(), today.year() + 1] {
            let Some(when) = on_year(year, month, dom) else {
                continue;
            };
            let away = (when - today).num_days();
            if !(0..=COMING_UP_DAYS).contains(&away) {
                continue;
            }
            let when_words = match away {
                0 => "today".to_string(),
                1 => "tomorrow".to_string(),
                n => format!("in {n} days"),
            };
            let years = first_year.filter(|y| year > *y).map(|y| format!(", {} years", year - y)).unwrap_or_default();
            found.push((away, format!("- {when_words} · {} (every year {month:02}-{dom:02}{years} · id {})", s.entry.summary, s.entry.id)));
        }
    }
    found.sort();
    found.into_iter().map(|(_, line)| line).collect()
}

/// Moments and chronicle lines dated today's month and day in an earlier
/// year, the nearest year first.
fn on_this_day(notes: &[&Seen], today: NaiveDate) -> Vec<String> {
    let mut found: Vec<(i32, String)> = Vec::new();
    for s in notes {
        let Some((date, what)) = dated(s) else {
            continue;
        };
        if (date.month(), date.day()) != (today.month(), today.day()) || date.year() >= today.year() {
            continue;
        }
        let ago = today.year() - date.year();
        let years = if ago == 1 { "1 year ago".to_string() } else { format!("{ago} years ago") };
        found.push((ago, format!("- {years} ({date}) · {what} · {} · id {}", s.entry.kind.word(), s.entry.id)));
    }
    found.sort();
    found.into_iter().take(MOST_PAST_LINES).map(|(_, line)| line).collect()
}

/// The day a moment or a chronicle line happened, and what it says. A moment
/// is dated the day it was written; a chronicle line starts with its
/// `YYYY-MM-DD` (a line with only a month has no day to match).
fn dated(s: &Seen) -> Option<(NaiveDate, String)> {
    match (&s.scope, s.entry.kind) {
        (Scope::Moments, Kind::Moment) => Some((NaiveDate::parse_from_str(&s.entry.created, "%Y-%m-%d").ok()?, s.entry.summary.clone())),
        (Scope::Chronicle, Kind::Chronicle) => {
            let (day, what) = s.entry.summary.split_once(' ')?;
            Some((NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?, what.trim().to_string()))
        }
        _ => None,
    }
}

/// A recurring day as (first year if known, month, day).
fn parts_of(day: &str) -> Option<(Option<i32>, u32, u32)> {
    let (year, rest) = match day.len() {
        10 => (Some(day.get(..4)?.parse().ok()?), day.get(5..)?),
        _ => (None, day),
    };
    let (month, dom) = rest.split_once('-')?;
    Some((year, month.parse().ok()?, dom.parse().ok()?))
}

/// The date in `year`; February 29 falls on the 28th in a year without one.
fn on_year(year: i32, month: u32, day: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(year, month, day).or_else(|| NaiveDate::from_ymd_opt(year, month, day.checked_sub(1)?))
}
