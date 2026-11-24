//! What Settings → Memory shows of Linggen's own notes: the topics, one
//! topic's notes, one note, and the review list. Read only; the page's
//! edits go through [`super::write::write`] like every other writer's.
//!
//! The person at Settings sees their whole record ([`OWNER`]) — the fence
//! keeps notes from coding hosts, not from their owner. Looking here counts
//! no use (`find::read` does), or opening the page would empty the review
//! list of the notes nobody has used.

use super::fence::{self, Audience, OpenError, Seen};
use super::format::{Entry, Scope};
use super::index::USE_WINDOW_SECS;
use super::{day_of, Store};
use serde::Serialize;
use std::collections::BTreeMap;

/// Who the page reads as: the person, who sees all of it. The one place
/// that says so.
pub const OWNER: Audience = Audience::Companion;

/// One topic file.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TopicRow {
    /// The file, inside the store: `person/diet.md`.
    pub path: String,
    pub name: String,
    pub scope: String,
    pub count: usize,
    pub voided: usize,
    /// The newest `updated` of its notes.
    pub changed: String,
}

/// One note, with where it is and the version of its file.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct NoteRow {
    pub id: String,
    pub summary: String,
    pub body: String,
    pub kind: &'static str,
    pub voice: &'static str,
    pub created: String,
    pub updated: String,
    pub source: String,
    /// Why it is known wrong; `None` when it is not.
    pub void: Option<String>,
    pub no_raise: bool,
    pub topic: String,
    pub scope: String,
    pub path: String,
    pub version: String,
}

impl NoteRow {
    fn of(s: Seen) -> NoteRow {
        let Seen { rel, version, scope, topic, entry, .. } = s;
        NoteRow::from_entry(entry, topic, scope.word(), rel, version)
    }

    fn from_entry(e: Entry, topic: String, scope: String, path: String, version: String) -> NoteRow {
        NoteRow {
            id: e.id,
            summary: e.summary,
            body: e.body,
            kind: e.kind.word(),
            voice: e.voice.word(),
            created: e.created,
            updated: e.updated,
            source: e.source,
            void: e.void,
            no_raise: e.no_raise,
            topic,
            scope,
            path,
            version,
        }
    }
}

/// Why a note is in the review list.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Why {
    /// Marked wrong.
    Void,
    /// Not read for the index's window, and not changed in it.
    Unused,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ReviewRow {
    #[serde(flatten)]
    pub note: NoteRow,
    pub why: Why,
}

/// The whole store at a glance.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Overview {
    pub topics: Vec<TopicRow>,
    /// Files that could not be read, with why.
    pub unreadable: Vec<String>,
}

pub fn overview(store: &Store) -> Overview {
    let (seen, unreadable) = fence::seen(store, OWNER);
    let mut by_file: BTreeMap<String, TopicRow> = BTreeMap::new();
    for s in seen {
        let row = by_file.entry(s.rel.clone()).or_insert_with(|| TopicRow {
            path: s.rel.clone(),
            name: s.topic.clone(),
            scope: s.scope.word(),
            count: 0,
            voided: 0,
            changed: String::new(),
        });
        row.count += 1;
        row.voided += usize::from(s.entry.void.is_some());
        row.changed = row.changed.clone().max(s.entry.updated);
    }
    Overview { topics: by_file.into_values().collect(), unreadable }
}

/// A topic file's notes, as they are in the file.
pub fn notes_of(store: &Store, path: &str) -> Result<Vec<NoteRow>, OpenError> {
    let file = fence::open(store, OWNER, path)?;
    let (rel, version, scope, topic) = (file.rel, file.version, file.scope.word(), file.topic);
    Ok(file
        .entries
        .into_iter()
        .map(|e| NoteRow::from_entry(e, topic.clone(), scope.clone(), rel.clone(), version.clone()))
        .collect())
}

/// One note by its id.
pub fn note(store: &Store, id: &str) -> Option<NoteRow> {
    fence::find(store, OWNER, id).map(NoteRow::of)
}

/// Notes to look at again: the ones marked wrong, then the ones not read in
/// the index's window (90 days) and not changed in it either, oldest first.
/// The chronicle's lines are listed only when marked wrong.
pub fn review(store: &Store, now: i64) -> Vec<ReviewRow> {
    let since = now - USE_WINDOW_SECS;
    let cutoff = day_of(since);
    let used = store.uses_since(since);
    let mut rows: Vec<ReviewRow> = fence::seen(store, OWNER)
        .0
        .into_iter()
        .filter_map(|s| {
            // The chronicle is long and found by search, never read into an
            // index, so "not read" says nothing of it: only a void line is listed.
            let unused = s.entry.updated < cutoff && !used.contains_key(&s.entry.id) && s.scope != Scope::Chronicle;
            let why = match (&s.entry.void, unused) {
                (Some(_), _) => Why::Void,
                (None, true) => Why::Unused,
                _ => return None,
            };
            Some(ReviewRow { note: NoteRow::of(s), why })
        })
        .collect();
    rows.sort_by(|a, b| {
        (a.why == Why::Unused, &a.note.updated, &a.note.id).cmp(&(b.why == Why::Unused, &b.note.updated, &b.note.id))
    });
    rows
}
