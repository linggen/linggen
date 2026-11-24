//! The one writer: add, update by id, void, un-void, no-raise, yearly, delete.
//! Ling, Yinyue, the phone and an outside host with no memory of its own all write through
//! [`write`], and nothing else saves into the store.
//!
//! What it enforces, for every caller:
//! - Only a rule, a decision, a gotcha, a fact about the person, or (Yinyue's)
//!   a life moment. Status, progress and "what was done" are refused with one
//!   fixed reason ([`STATUS_REFUSAL`]).
//! - An entry in the person's own voice changes only with `user_directed`.
//! - The caller says which version of the topic file it saw. A different
//!   version is merged by entry: another entry changed — fine; the same
//!   entry changed since — a [`Refusal::Conflict`] with the current text.
//! - What the audience may not see or write ([`super::fence`]) cannot be
//!   written, nor found by id.
//! - A chronicle line (kind chronicle) goes only in the chronicle, into the
//!   file of its month or year, and is not held to the status rule — what
//!   the person did is what it is for.
//! - The old file is kept under `.state/history` (the last 20 versions of
//!   each file); a topic past the limits is split by [`Store::save`].
//! - Un-void and delete are the person's, from Settings: no tool op names
//!   them (`call::write_of`), so an agent cannot bring back a note the
//!   person marked wrong, nor remove one.

use super::fence::{self, Audience};
use super::format::{safe_body, one_line, meta_text, yearly_day, Entry, Kind, Scope, Topic, Voice, BODY_CHARS, SUMMARY_CHARS};
use super::store::{slug, Store};
use regex::Regex;
use std::sync::{LazyLock, Mutex};

/// The reason given for every refused status line. One text, so a caller can
/// pass it on word for word.
pub const STATUS_REFUSAL: &str = "Not memory: status, progress and what was done. Git, the docs and the \
activity log hold those. Memory keeps a rule, a decision with its why, a gotcha, or a fact about the person.";

/// One write.
#[derive(Clone, Debug)]
pub struct Write {
    pub action: Action,
    /// The person said so, in this conversation: lets a `Voice::User` entry change.
    pub user_directed: bool,
    /// Who is writing: `claude-code`, `codex`, `linggen`, `yinyue`, `phone`, …
    pub source: String,
    /// The version of the topic file the caller last saw.
    pub base: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Action {
    Add { scope: Scope, topic: String, summary: String, kind: Kind, voice: Voice, body: String },
    Update { id: String, summary: Option<String>, body: Option<String>, kind: Option<Kind> },
    Void { id: String, reason: String },
    /// The note is not wrong after all. Settings only.
    Unvoid { id: String },
    NoRaise { id: String, on: bool },
    /// Set the day the note repeats every year (`MM-DD` or `YYYY-MM-DD`), or
    /// clear it (`None`).
    Yearly { id: String, day: Option<String> },
    /// Take the note out of its topic; the file's last version stays in history.
    /// Settings only.
    Delete { id: String },
}

/// What a write did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    pub id: String,
    /// The file the entry is in now (a split may move it).
    pub rel: String,
    /// That file's version now.
    pub version: String,
    /// The caller's version was old and the write was merged into the newer file.
    pub merged: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Status,
    Empty(&'static str),
    TooLong(&'static str),
    NotFound,
    /// The scope is not one this audience may write.
    Fenced,
    /// A moment is Yinyue's, in `moments`, and a chronicle line is in the
    /// chronicle; a rule or a gotcha is in neither.
    KindMismatch,
    /// A chronicle file is a month (`2026-11`) or a year (`2026`).
    NotAPeriod,
    /// A yearly day is `MM-DD` or `YYYY-MM-DD`.
    NotADay,
    /// An entry in the person's own voice: show it and ask first.
    UserVoice { id: String, summary: String, updated: String },
    /// Another topic of a different scope has that file name.
    TopicTaken,
    Duplicate(String),
    /// The same entry changed since the version the caller saw.
    Conflict { current: Box<Entry>, version: String },
    /// The caller's version is gone and the file changed.
    ReadAgain,
    Disk(String),
}

impl Refusal {
    /// A short word for the refusal, for a page to put in its own words
    /// (the sentence in `Display` is written for an agent).
    pub fn code(&self) -> &'static str {
        match self {
            Refusal::Status => "status",
            Refusal::Empty(_) => "empty",
            Refusal::TooLong(_) => "too_long",
            Refusal::NotFound => "not_found",
            Refusal::Fenced => "fenced",
            Refusal::KindMismatch => "kind_mismatch",
            Refusal::NotAPeriod => "not_a_period",
            Refusal::NotADay => "not_a_day",
            Refusal::UserVoice { .. } => "user_voice",
            Refusal::TopicTaken => "topic_taken",
            Refusal::Duplicate(_) => "duplicate",
            Refusal::Conflict { .. } => "conflict",
            Refusal::ReadAgain => "read_again",
            Refusal::Disk(_) => "disk",
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Status => f.write_str(STATUS_REFUSAL),
            Refusal::Empty(what) => write!(f, "the {what} is empty"),
            Refusal::TooLong(what) => write!(f, "the {what} is too long"),
            Refusal::NotFound => f.write_str("no note with that id"),
            Refusal::Fenced => f.write_str("this reader may not write there"),
            Refusal::KindMismatch => f.write_str(
                "kind moment goes with scope moments, kind chronicle with scope chronicle, and only those",
            ),
            Refusal::NotAPeriod => f.write_str("a chronicle topic is a month (2026-11) or a year (2026)"),
            Refusal::NotADay => f.write_str("a yearly day is MM-DD, or YYYY-MM-DD when the first year is known"),
            Refusal::UserVoice { id, summary, updated } => write!(
                f,
                "note {id} (\"{summary}\", {updated}) is in the person's own voice: show it to them and ask; \
                 after their answer, write again with user_directed true"
            ),
            Refusal::TopicTaken => f.write_str("another topic of a different scope has that name; name the topic differently"),
            Refusal::Duplicate(id) => write!(f, "note {id} already says this; update it instead"),
            Refusal::Conflict { current, version } => write!(
                f,
                "note {} changed since you read it (now: \"{}\" — {}); version {version}. Look at it and write again.",
                current.id, current.summary, current.body
            ),
            Refusal::ReadAgain => f.write_str("the topic changed since you read it; read it again and write again"),
            Refusal::Disk(why) => write!(f, "could not save: {why}"),
        }
    }
}

/// Two writes never run at once.
static LOCK: Mutex<()> = Mutex::new(());

/// Apply `w` for `audience` at `now` (epoch seconds).
pub fn write(store: &Store, audience: Audience, now: i64, w: &Write) -> Result<Written, Refusal> {
    let _one_at_a_time = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let today = super::day_of(now);
    match &w.action {
        Action::Add { scope, topic, summary, kind, voice, body } => {
            add(store, audience, w, &today, scope, topic, summary, *kind, *voice, body)
        }
        Action::Update { id, summary, body, kind } => {
            let (summary, body, kind) = (summary.clone(), body.clone(), *kind);
            change(store, audience, w, id, move |e| {
                if let Some(s) = &summary {
                    let s = one_line(s, SUMMARY_CHARS);
                    e.summary = s;
                }
                if let Some(b) = &body {
                    e.body = safe_body(b);
                }
                if let Some(k) = kind {
                    e.kind = k;
                }
                checked(e)
            }, &today)
        }
        Action::Void { id, reason } => {
            let reason = meta_text(reason);
            if reason.is_empty() {
                return Err(Refusal::Empty("reason"));
            }
            change(store, audience, w, id, move |e| {
                e.void = Some(reason.clone());
                Ok(())
            }, &today)
        }
        Action::Unvoid { id } => change(store, audience, w, id, |e| {
            e.void = None;
            Ok(())
        }, &today),
        Action::NoRaise { id, on } => {
            let on = *on;
            change(store, audience, w, id, move |e| {
                e.no_raise = on;
                Ok(())
            }, &today)
        }
        Action::Yearly { id, day } => {
            let day = match day.as_deref() {
                Some(text) => Some(yearly_day(text).ok_or(Refusal::NotADay)?),
                None => None,
            };
            change(store, audience, w, id, move |e| {
                e.yearly = day.clone();
                Ok(())
            }, &today)
        }
        Action::Delete { id } => delete(store, audience, w, id),
    }
}

#[allow(clippy::too_many_arguments)]
fn add(
    store: &Store,
    audience: Audience,
    w: &Write,
    today: &str,
    scope: &Scope,
    topic: &str,
    summary: &str,
    kind: Kind,
    voice: Voice,
    body: &str,
) -> Result<Written, Refusal> {
    if !audience.can_write(scope) {
        return Err(Refusal::Fenced);
    }
    if !kind.fits(scope) {
        return Err(Refusal::KindMismatch);
    }
    let name = match scope {
        Scope::Moments => today.chars().take(7).collect(),
        Scope::Chronicle => period_of(topic).ok_or(Refusal::NotAPeriod)?,
        _ => one_line(topic, 80),
    };
    let rel = Store::rel_of(scope, &slug(&name));
    let (mut file, version) = match store.load(&rel) {
        Ok(Some(f)) if f.topic.scope == *scope => (f.topic, Some(f.version)),
        Ok(Some(_)) => return Err(Refusal::TopicTaken),
        Ok(None) => (Topic::new(&name, scope.clone()), None),
        Err(why) => return Err(Refusal::Disk(format!("{rel}: {why}"))),
    };
    let entry = Entry::new(new_id(store), summary, kind, voice, today, &w.source, body);
    checked(&entry)?;
    if let Some(same) = file.entries.iter().find(|e| e.void.is_none() && e.summary.to_lowercase() == entry.summary.to_lowercase()) {
        return Err(Refusal::Duplicate(same.id.clone()));
    }
    let id = entry.id.clone();
    let merged = matches!((&w.base, &version), (Some(b), Some(v)) if b != v);
    file.entries.push(entry);
    store.save(&rel, file).map_err(|e| Refusal::Disk(e.to_string()))?;
    landed(store, audience, &id, merged)
}

/// An entry about to change: where it is now, and whether the caller's
/// version of its file was old (the rest of the file moved: a merge).
struct Pending {
    found: fence::Seen,
    topic: Topic,
    merged: bool,
}

/// The checks every change to an existing entry passes: the audience may
/// see it, the person's own voice is changed only when they said so, and
/// the entry is as the caller last saw it.
fn prepare(store: &Store, audience: Audience, w: &Write, id: &str) -> Result<Pending, Refusal> {
    let found = fence::find(store, audience, id).ok_or(Refusal::NotFound)?;
    if !audience.can_write(&found.scope) {
        return Err(Refusal::Fenced);
    }
    if found.entry.voice == Voice::User && !w.user_directed {
        return Err(Refusal::UserVoice {
            id: found.entry.id,
            summary: found.entry.summary,
            updated: found.entry.updated,
        });
    }
    let file = store
        .load(&found.rel)
        .map_err(Refusal::Disk)?
        .ok_or(Refusal::NotFound)?;
    let merged = match &w.base {
        Some(base) if *base != file.version => {
            unchanged_since(store, &found.rel, base, &found.entry)?;
            true
        }
        _ => false,
    };
    Ok(Pending { found, topic: file.topic, merged })
}

/// Apply `edit` to the entry `id` in the current file, after the checks.
fn change(
    store: &Store,
    audience: Audience,
    w: &Write,
    id: &str,
    edit: impl FnOnce(&mut Entry) -> Result<(), Refusal>,
    today: &str,
) -> Result<Written, Refusal> {
    let Pending { found, mut topic, merged } = prepare(store, audience, w, id)?;
    let entry = topic.entries.iter_mut().find(|e| e.id == found.entry.id).ok_or(Refusal::NotFound)?;
    edit(entry)?;
    if !entry.kind.fits(&found.scope) {
        return Err(Refusal::KindMismatch);
    }
    entry.updated = today.to_string();
    entry.source = one_line(&w.source, 40);
    store.save(&found.rel, topic).map_err(|e| Refusal::Disk(e.to_string()))?;
    landed(store, audience, &found.entry.id, merged)
}

/// Take the entry out of its file. A file left with no entries goes too
/// ([`Store::remove`]); either way its last version is in the history.
fn delete(store: &Store, audience: Audience, w: &Write, id: &str) -> Result<Written, Refusal> {
    let Pending { found, mut topic, merged } = prepare(store, audience, w, id)?;
    topic.entries.retain(|e| e.id != found.entry.id);
    let saved = match topic.entries.is_empty() {
        true => store.remove(&found.rel),
        false => store.save(&found.rel, topic),
    };
    saved.map_err(|e| Refusal::Disk(e.to_string()))?;
    let version = store.load(&found.rel).ok().flatten().map(|f| f.version).unwrap_or_default();
    Ok(Written { id: found.entry.id, rel: found.rel, version, merged })
}

/// The entry is as it was in the version the caller saw — so a change to
/// the rest of the file is a merge, not a conflict.
fn unchanged_since(store: &Store, rel: &str, base: &str, current: &Entry) -> Result<(), Refusal> {
    let then = store.at_version(rel, base).ok_or(Refusal::ReadAgain)?;
    match then.entries.iter().find(|e| e.id == current.id) {
        Some(before) if before == current => Ok(()),
        _ => Err(Refusal::Conflict {
            current: Box::new(current.clone()),
            version: store.load(rel).ok().flatten().map(|f| f.version).unwrap_or_default(),
        }),
    }
}

fn landed(store: &Store, audience: Audience, id: &str, merged: bool) -> Result<Written, Refusal> {
    let now = fence::find(store, audience, id).ok_or_else(|| Refusal::Disk("the note did not land".into()))?;
    let version = store.load(&now.rel).ok().flatten().map(|f| f.version).unwrap_or_default();
    Ok(Written { id: id.to_string(), rel: now.rel, version, merged })
}

/// An entry's text is a memory: not empty, not too long, not a status line.
fn checked(e: &Entry) -> Result<(), Refusal> {
    if e.summary.is_empty() {
        return Err(Refusal::Empty("summary"));
    }
    if e.body.chars().count() > BODY_CHARS {
        return Err(Refusal::TooLong("body"));
    }
    let reads_as_status = !matches!(e.kind, Kind::Moment | Kind::Chronicle) && is_status(&e.summary, &e.body);
    match reads_as_status {
        true => Err(Refusal::Status),
        false => Ok(()),
    }
}

/// A chronicle file's name: `2026-11` or `2026`, nothing else.
fn period_of(topic: &str) -> Option<String> {
    let topic = topic.trim();
    let (year, month) = match topic.split_once('-') {
        Some((year, month)) => (year, Some(month)),
        None => (topic, None),
    };
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    let month_ok = month.map_or(true, |m| digits(m, 2) && (1..=12).contains(&m.parse::<u32>().unwrap_or(0)));
    (digits(year, 4) && month_ok).then(|| topic.to_string())
}

fn new_id(store: &Store) -> String {
    loop {
        let id: String = uuid::Uuid::new_v4().simple().to_string().chars().take(10).collect();
        if fence::find(store, Audience::Companion, &id).is_none() {
            return id;
        }
    }
}

/// Markers of a line that says where work stands or what was done. A rule, a
/// decision, a gotcha and a fact about the person do not read like these.
static STATUS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)^\s*(status|progress|state|done|todo|wip|resume|pending|in progress|next steps?)\s*[:：\-–—]",
        r"(?i)^\s*(fixed|shipped|merged|released|deployed|completed|finished|implemented|migrated|refactored)\b",
        r"(?i)\b(is|was|are|were|now|been)\s+(fixed|shipped|merged|released|deployed|done|completed|in progress|pending)\b",
        r"(?i)\bcurrently (working|in progress|being)\b",
        r"^\s*(已完成|已修复|已发布|已上线|已合并|进行中|待办|进度|状态|今天做了|刚刚|目前)",
        r"(已完成|已修复|已发布|已上线|已合并|进行中|待办)",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("a status marker"))
    .collect()
});

/// Whether the summary, or the body's first line, reads as status.
pub(super) fn is_status(summary: &str, body: &str) -> bool {
    std::iter::once(summary)
        .chain(body.lines().find(|l| !l.trim().is_empty()))
        .any(|line| STATUS.iter().any(|re| re.is_match(line)))
}
