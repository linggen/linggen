//! The chronicle's input: what the monthly `chronicle` mission is handed to
//! read. The mission's agent writes the months (`memory_notes op=write
//! scope=chronicle`, through [`super::write`], like every note); this file
//! only READS — Linggen's own notes and moments, the person's own sessions,
//! and what other agents' notes changed that month — and builds the list of
//! what is still to be written:
//!
//! - a month that has ended and has something to say, and no chronicle file
//!   yet (at most [`MONTHS_PER_RUN`] a run, oldest first — a person's backfill
//!   is the same run, started by hand until none is left);
//! - a past year whose months are all written, and which has no year file.
//!
//! A month that has a file is never offered again: the person edits it, and
//! deleting the file is how they ask for it to be written afresh. A note
//! marked no-raise or void is left out of the sources, so it cannot come
//! back through the chronicle's "on this day" lines.
//!
//! [`prepare`] returns the text of the input file, or nothing when there is
//! nothing to write; the caller (`ling chronicle prepare`) saves it — this
//! module saves nothing.

use super::fence::{self, Audience, Seen};
use super::format::{one_line, Kind, Scope};
use super::{day_of, Store};
use crate::outside_memory::{self as outside, clean};
use crate::state_fs::sessions::{SessionMeta, SessionStore};
use crate::state_fs::RowAuthor;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// The input file, in the memory folder (not a note: nothing loads it).
pub const INPUT_FILE: &str = ".chronicle-input.jsonl";
/// Months written in one run; the next run (or a backfill) goes on.
pub const MONTHS_PER_RUN: usize = 12;

const NOTES_PER_MONTH: usize = 60;
const SESSIONS_PER_MONTH: usize = 40;
const OTHERS_PER_MONTH: usize = 30;
const BODY_CHARS: usize = 400;
const SAID_CHARS: usize = 160;
const SAID_PER_SESSION: usize = 3;

/// What a run has to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    /// The input file's text: a header line, then one line per month, then
    /// one per year.
    pub text: String,
    pub months: Vec<String>,
    pub years: Vec<String>,
}

/// The input for a run at `now`, or `None` when there is nothing to write.
pub fn prepare(store: &Store, sessions: &SessionStore, now: i64) -> Option<Prepared> {
    let today = day_of(now);
    let this_month = today.get(..7)?.to_string();
    let this_year = today.get(..4)?.to_string();
    let (all, _) = fence::seen(store, Audience::Chronicler);
    let written = written_periods(&all);

    let mut notes = notes_by_month(&all);
    let mut said = sessions_by_month(sessions);
    let mut others = others_by_month(store);
    let candidates: BTreeSet<String> =
        notes.keys().chain(said.keys()).chain(others.keys()).filter(|m| **m < this_month).cloned().collect();
    let pending: Vec<String> = candidates.iter().filter(|m| !written.contains(*m)).cloned().collect();
    let months: Vec<String> = pending.iter().take(MONTHS_PER_RUN).cloned().collect();
    let years = years_to_write(&written, &months, &pending, &this_year);
    if months.is_empty() && years.is_empty() {
        return None;
    }

    let mut lines = vec![json!({"header": true, "today": today, "months": months, "years": years}).to_string()];
    for month in &months {
        let line = json!({
            "month": month,
            "notes": cut(notes.remove(month).unwrap_or_default(), NOTES_PER_MONTH),
            "sessions": cut(said.remove(month).unwrap_or_default(), SESSIONS_PER_MONTH),
            "other_agents": cut(others.remove(month).unwrap_or_default(), OTHERS_PER_MONTH),
        });
        lines.push(line.to_string());
    }
    for year in &years {
        let files: Vec<String> = written
            .iter()
            .chain(months.iter())
            .filter(|p| p.len() == 7 && p.starts_with(year.as_str()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|m| format!("chronicle/{year}/{m}.md"))
            .collect();
        lines.push(json!({"year": year, "month_files": files}).to_string());
    }
    Some(Prepared { text: lines.join("\n") + "\n", months, years })
}

/// The months and years that have a chronicle file (`2026-11`, `2026`), by
/// the file's name: a file split in parts (`2026-11-part2`) is its month's.
fn written_periods(all: &[Seen]) -> BTreeSet<String> {
    all.iter().filter(|s| s.scope == Scope::Chronicle).filter_map(|s| period_of_rel(&s.rel)).collect()
}

/// `chronicle/2026/2026-11-part2.md` -> `2026-11`.
pub fn period_of_rel(rel: &str) -> Option<String> {
    let stem = rel.rsplit('/').next()?.strip_suffix(".md")?;
    let period = stem.split("-part").next()?;
    matches!(period.len(), 4 | 7).then(|| period.to_string())
}

/// Past years with no year file whose months are all written (or in this
/// run): no month of the year is still waiting for a later run.
fn years_to_write(written: &BTreeSet<String>, months: &[String], pending: &[String], this_year: &str) -> Vec<String> {
    let mut years: BTreeSet<String> = BTreeSet::new();
    for period in written.iter().chain(months.iter()).filter(|p| p.len() == 7) {
        years.insert(period[..4].to_string());
    }
    years
        .into_iter()
        .filter(|y| y.as_str() < this_year && !written.contains(y))
        .filter(|y| !pending.iter().any(|m| m.starts_with(y.as_str()) && !months.contains(m)))
        .collect()
}

/// `value`'s first `most` items.
fn cut(mut items: Vec<Value>, most: usize) -> Vec<Value> {
    items.truncate(most);
    items
}

fn month_of(day: &str) -> Option<String> {
    let month = day.get(..7)?;
    let ok = month.char_indices().all(|(i, c)| if i == 4 { c == '-' } else { c.is_ascii_digit() });
    ok.then(|| month.to_string())
}

/// Linggen's own notes by the month they were made or changed in, newest
/// change first. Void, no-raise and the chronicle itself are left out.
fn notes_by_month(all: &[Seen]) -> BTreeMap<String, Vec<Value>> {
    let mut sorted: Vec<&Seen> = all
        .iter()
        .filter(|s| s.entry.void.is_none() && !s.entry.no_raise && s.scope != Scope::Chronicle && s.entry.kind != Kind::Chronicle)
        .collect();
    sorted.sort_by(|a, b| b.entry.updated.cmp(&a.entry.updated).then(a.entry.id.cmp(&b.entry.id)));
    let mut out: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for s in sorted {
        let line = json!({
            "id": s.entry.id, "scope": s.scope.word(), "topic": s.topic, "kind": s.entry.kind.word(),
            "summary": s.entry.summary, "created": s.entry.created, "updated": s.entry.updated,
            "body": s.entry.body.chars().take(BODY_CHARS).collect::<String>(),
        });
        let months: BTreeSet<String> =
            [&s.entry.created, &s.entry.updated].into_iter().filter_map(|d| month_of(d)).collect();
        for month in months {
            out.entry(month).or_default().push(line.clone());
        }
    }
    out
}

/// The person's own words in their sessions, by month: per session, how many
/// turns that month and the first few things said. Same rule as the memory
/// scan's (`crates/linggen-memory/scan/collect_sessions.sh`, "The rule"),
/// kept narrower: only a session the person made themselves.
fn sessions_by_month(store: &SessionStore) -> BTreeMap<String, Vec<Value>> {
    let Ok(list) = store.list_sessions() else {
        return BTreeMap::new();
    };
    let mut found: BTreeMap<String, Vec<(usize, Value)>> = BTreeMap::new();
    for meta in list.iter().filter(|m| is_the_persons(m)) {
        let Ok(rows) = store.get_chat_history(&meta.id) else {
            continue;
        };
        let mut by_month: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for row in rows.iter().filter(|r| r.author() == RowAuthor::Person && !r.is_observation && !r.content.starts_with("[HIDDEN]")) {
            let day = day_of(row.timestamp as i64);
            if let Some(month) = month_of(&day) {
                by_month.entry(month).or_default().push((day, row.content.clone()));
            }
        }
        for (month, said) in by_month {
            let turns = said.len();
            let first_day = said[0].0.clone();
            let said: Vec<String> = said.iter().take(SAID_PER_SESSION).map(|(_, text)| clean(&one_line(text, SAID_CHARS)).into_string()).collect();
            let line = json!({"date": first_day, "title": meta.title, "folder": meta.cwd, "turns": turns, "said": said});
            found.entry(month).or_default().push((turns, line));
        }
    }
    found
        .into_iter()
        .map(|(month, mut items)| {
            items.sort_by(|a, b| b.0.cmp(&a.0));
            (month, items.into_iter().map(|(_, v)| v).collect())
        })
        .collect()
}

/// A session the person made and spoke in: not a mission's, a host's, an
/// app's, a delegate's, or another person's day.
fn is_the_persons(meta: &SessionMeta) -> bool {
    let others_day = meta.id.strip_prefix("sess-day-").is_some_and(|rest| rest.get(10..).is_some_and(|r| r.starts_with("-p-")));
    meta.creator == "user" && meta.skill.is_none() && meta.parent_session_id.is_none() && !others_day
}

/// What other agents' notes were changed in each month: their first lines.
fn others_by_month(store: &Store) -> BTreeMap<String, Vec<Value>> {
    let mut touched = outside::touched(store.env());
    touched.sort_by(|a, b| b.changed.cmp(&a.changed).then(a.path.cmp(&b.path)));
    let mut out: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for note in touched {
        let Some(month) = month_of(&day_of(note.changed as i64)) else {
            continue;
        };
        let line = json!({"agent": note.source, "note": store.env().tilde(&note.path), "first_line": note.first_line.as_str()});
        out.entry(month).or_default().push(line);
    }
    out
}
