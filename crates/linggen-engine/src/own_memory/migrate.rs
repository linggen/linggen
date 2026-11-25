//! The move from the v1 store (ling-mem rows) to these notes: what is kept,
//! what is not, and when it runs by itself.
//!
//! What a row becomes is decided here and nowhere else ([`plan`]); what is
//! written is [`super::write::carry`], the writer, so every note passes its
//! checks and no file is made around it. Nothing here reads or changes the
//! v1 store: the rows come in as data ([`V1Row`]), from `ling mem
//! migrate-v2` or from the engine's start ([`upgrade_once`]).
//!
//! Rule-based, no model: a person fact, a preference or rule, a decision and
//! a learned gotcha are kept; what was done (built, fixed, tried), episodic
//! staging and what reads as status are not. A judged clean-up of what the
//! rules let through is the dream's job, later. The id of a note is the id of
//! its row, so a second run adds nothing (`write::carry` leaves a known id).

use super::fence::Audience;
use super::format::{one_line, Kind, Scope, Voice, SUMMARY_CHARS};
use super::store::{slug, Store};
use super::write::{carry, Carried, Carry, Refusal};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::LazyLock;

/// A row of the v1 store, as its `list` and `export` give it. Only what the
/// move reads.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct V1Row {
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub tier: String,
    #[serde(default)]
    pub from: String,
    #[serde(default, alias = "cwd")]
    pub scope: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub content: String,
    pub created_at: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub expired_at: Option<String>,
}

impl V1Row {
    /// The rows in a `list` answer (a JSON array), and how many entries were
    /// not rows.
    pub fn from_list(data: &serde_json::Value) -> (Vec<V1Row>, usize) {
        let items = data.as_array().map(Vec::as_slice).unwrap_or_default();
        let rows: Vec<V1Row> = items.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect();
        let unreadable = items.len() - rows.len();
        (rows, unreadable)
    }

    /// The rows of an export file (one JSON row a line); blank lines pass.
    pub fn from_ndjson(text: &str) -> (Vec<V1Row>, usize) {
        let (mut rows, mut unreadable) = (Vec::new(), 0);
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str(line) {
                Ok(row) => rows.push(row),
                Err(_) => unreadable += 1,
            }
        }
        (rows, unreadable)
    }
}

/// Why a row is not carried over. One sentence each, shown in the counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Why {
    NotTheOwners,
    Expired,
    Episodic,
    WhatWasDone,
    ProjectFact,
    UnknownType,
    Unplaced,
    BadDate,
    Secret,
}

impl Why {
    fn words(self) -> &'static str {
        match self {
            Why::NotTheOwners => "another person's row",
            Why::Expired => "already replaced in v1",
            Why::Episodic => "episodic staging",
            Why::WhatWasDone => "what was done (built, fixed, tried)",
            Why::ProjectFact => "a project fact (state or spec)",
            Why::UnknownType => "a type with no kind here",
            Why::Unplaced => "a scope that is not a folder path",
            Why::BadDate => "a date that cannot be read",
            Why::Secret => "holds a key-shaped secret",
        }
    }
}

/// What a row is, if it is kept: the note to carry over.
fn plan(row: &V1Row) -> Result<Carried, Why> {
    if row.account_id.is_some() {
        return Err(Why::NotTheOwners);
    }
    if row.expired_at.is_some() {
        return Err(Why::Expired);
    }
    if row.tier == "episodic" {
        return Err(Why::Episodic);
    }
    let place = place_of(row.scope.as_deref())?;
    let kind = match (row.kind.as_str(), &place) {
        ("decision", _) => Kind::Decision,
        ("preference", _) => Kind::Rule,
        ("learned", _) => Kind::Gotcha,
        ("fact", Scope::Person) => Kind::Person,
        ("fact", _) => return Err(Why::ProjectFact),
        ("built" | "fixed" | "tried", _) => return Err(Why::WhatWasDone),
        _ => return Err(Why::UnknownType),
    };
    if holds_a_secret(&row.content) || row.summary.as_deref().is_some_and(holds_a_secret) {
        return Err(Why::Secret);
    }
    let created = day_of_stamp(&row.created_at).ok_or(Why::BadDate)?;
    let updated = row.updated_at.as_deref().and_then(day_of_stamp).unwrap_or_else(|| created.clone());
    let summary = summary_of(row);
    Ok(Carried {
        id: row.id.clone(),
        scope: place,
        topic: topic_of(&summary, kind),
        summary,
        kind,
        voice: if row.from == "user" { Voice::User } else { Voice::Agent },
        created,
        updated,
        source: source_of(row.host.as_deref()).to_string(),
        body: row.content.clone(),
    })
}

/// No scope is about the person; a folder path is that project.
fn place_of(scope: Option<&str>) -> Result<Scope, Why> {
    let Some(path) = scope.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(Scope::Person);
    };
    let path = PathBuf::from(path.trim_end_matches('/'));
    match path.is_absolute() && path.components().count() > 1 {
        true => Ok(Scope::Project(path)),
        false => Err(Why::Unplaced),
    }
}

/// The writing host as the `source` of a note.
fn source_of(host: Option<&str>) -> &'static str {
    match host {
        Some("claude-code") => "claude-code",
        Some("codex") => "codex",
        Some("linggen-mobile") => "phone",
        _ => "linggen",
    }
}

/// The local day of an RFC 3339 stamp.
fn day_of_stamp(stamp: &str) -> Option<String> {
    let at = chrono::DateTime::parse_from_rfc3339(stamp.trim()).ok()?;
    Some(super::day_of(at.timestamp()))
}

static SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"AKIA[A-Z0-9]{12,}|\bsk-[A-Za-z0-9_\-]{16,}|\bgh[pousr]_[A-Za-z0-9]{20,}|\bxox[baprs]-[A-Za-z0-9\-]{10,}|AIza[0-9A-Za-z_\-]{30,}|-----BEGIN [A-Z ]*PRIVATE KEY-----")
        .expect("a secret pattern")
});

fn holds_a_secret(text: &str) -> bool {
    SECRET.is_match(text)
}

/// The row's own summary, else the first clause of what it says.
fn summary_of(row: &V1Row) -> String {
    let own = row.summary.as_deref().map(|s| one_line(s, SUMMARY_CHARS)).unwrap_or_default();
    match own.is_empty() {
        false => own,
        true => first_clause(&row.content),
    }
}

/// The first line up to its first full stop, within a summary's length.
fn first_clause(content: &str) -> String {
    let line = content.lines().map(|l| l.trim().trim_start_matches('#').replace("**", "")).find(|l| !l.trim().is_empty());
    let line = line.unwrap_or_default();
    let end = [". ", "。", "; ", "；"].iter().filter_map(|stop| line.find(stop)).min().unwrap_or(line.len());
    one_line(&line[..end], SUMMARY_CHARS)
}

/// Words of a leading label that is not the subject: who said it and when,
/// and a bare "GOTCHA".
static LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:(?:hanli|liang|user|he)\b[\s,，:：]*)?(?:\(?\d{4}-\d{2}-\d{2}\)?|\d{1,2}-\d{1,2})?[\s,，:：—–-]*(?:(?:gotcha|rule|decision|lesson|note|learned)\b[\s:：—–-]*)?")
        .expect("a label pattern")
});
/// A word shorter than this is not a subject by itself ("cp", "the"): the next word goes with it.
const SUBJECT_MIN_CHARS: usize = 4;
const SUBJECT_CJK_CHARS: usize = 12;

/// A stable topic name from the subject of a note: its summary without the
/// label, up to the first mark that opens a detail, its first word (two when
/// the first is short). Measured on a real store of 2,354 rows, three words
/// made a file for almost every note (1,173 files for 1,416 notes) and one
/// word about half that. The same summary is always the same topic, so a
/// second run lands in the same file. A summary with no subject goes under
/// its kind.
fn topic_of(summary: &str, kind: Kind) -> String {
    let rest = LABEL.replace(summary.trim(), "");
    let head = rest.split(['.', ':', '：', '—', '–', '(', '（', ',', '，', ';', '；', '「', '。']).next().unwrap_or_default();
    let head = head.split(" - ").next().unwrap_or_default().trim();
    let name = match head.split_whitespace().count() {
        0 => String::new(),
        1 if head.chars().count() > SUBJECT_CJK_CHARS => head.chars().take(SUBJECT_CJK_CHARS).collect(),
        _ => subject_words(head),
    };
    match slug(&name) == "notes" {
        true => format!("{}s", kind.word()),
        false => name,
    }
}

/// The first word of `head`, with the second when the first is short.
fn subject_words(head: &str) -> String {
    let mut words = head.split_whitespace();
    let first = words.next().unwrap_or_default();
    match (first.chars().count() < SUBJECT_MIN_CHARS, words.next()) {
        (true, Some(second)) => format!("{first} {second}"),
        _ => first.to_string(),
    }
}

/// What a move did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub read: usize,
    /// Entries in the file that were not rows.
    pub unreadable: usize,
    /// Notes written (or that would be, in a dry run).
    pub landed: usize,
    /// Rows whose id is already a note.
    pub already: usize,
    pub by_kind: BTreeMap<&'static str, usize>,
    /// Topic files the written notes are in.
    pub topics: usize,
    /// Rows not carried over, by reason.
    pub dropped: BTreeMap<String, usize>,
}

impl Report {
    pub fn dropped_total(&self) -> usize {
        self.dropped.values().sum()
    }

    fn drop_one(&mut self, reason: impl Into<String>) {
        *self.dropped.entry(reason.into()).or_default() += 1;
    }

    /// The lines `ling mem migrate-v2` prints.
    pub fn lines(&self, dry_run: bool) -> Vec<String> {
        let mut out = vec![format!("{} v1 rows read{}", self.read, if self.unreadable > 0 { format!(" ({} unreadable)", self.unreadable) } else { String::new() })];
        let verb = if dry_run { "would keep" } else { "kept" };
        let kinds: Vec<String> = self.by_kind.iter().map(|(k, n)| format!("{k} {n}")).collect();
        out.push(format!("{verb} {} in {} topic files ({})", self.landed, self.topics, kinds.join(", ")));
        out.push(format!("already there: {}", self.already));
        out.push(format!("dropped {}:", self.dropped_total()));
        let mut why: Vec<_> = self.dropped.iter().collect();
        why.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        out.extend(why.into_iter().map(|(reason, n)| format!("  {reason}: {n}")));
        if dry_run {
            out.push("dry run: nothing was written".to_string());
        }
        out
    }

    fn record(&self) -> serde_json::Value {
        serde_json::json!({
            "result": "moved",
            "read": self.read,
            "kept": self.landed,
            "already": self.already,
            "dropped": self.dropped_total(),
            "topics": self.topics,
        })
    }
}

/// The writer's refusal, as the sentence the counts show.
fn refusal_words(refusal: &Refusal) -> String {
    match refusal {
        Refusal::Status => "reads as status".to_string(),
        Refusal::Duplicate(_) => "same summary as a note in its topic".to_string(),
        Refusal::Empty(what) => format!("empty {what}"),
        Refusal::TooLong(what) => format!("{what} over its length limit"),
        other => format!("refused: {}", other.code()),
    }
}

/// Carry the kept rows over, oldest first, through the writer. `save` false:
/// the counts of a real run, nothing written. Safe to run again: a row whose
/// id is a note is counted as already there.
pub fn migrate(store: &Store, rows: &[V1Row], save: bool) -> Report {
    let mut report = Report { read: rows.len(), ..Report::default() };
    let mut kept: Vec<&V1Row> = rows.iter().collect();
    kept.sort_by(|a, b| (&a.created_at, &a.id).cmp(&(&b.created_at, &b.id)));
    let (mut batch, mut rels) = (Vec::new(), Vec::new());
    for row in kept {
        match plan(row) {
            Ok(note) => {
                rels.push(Store::rel_of(&note.scope, &slug(&one_line(&note.topic, 80))));
                batch.push(note);
            }
            Err(why) => report.drop_one(why.words()),
        }
    }
    let mut topics = BTreeSet::new();
    for ((note, rel), answer) in batch.iter().zip(rels).zip(carry(store, Audience::Companion, &batch, save)) {
        match answer {
            Ok(Carry::Landed) => {
                report.landed += 1;
                *report.by_kind.entry(note.kind.word()).or_default() += 1;
                topics.insert(rel);
            }
            Ok(Carry::Already) => report.already += 1,
            Err(refusal) => report.drop_one(refusal_words(&refusal)),
        }
    }
    report.topics = topics.len();
    report
}

/// What the start-up check did.
#[derive(Debug, PartialEq, Eq)]
pub enum Upgrade {
    /// Both switches are v1: the person chose to stay.
    NotWanted,
    /// Looked at before; the record says so.
    AlreadyLooked,
    /// These notes already exist (a Mac that was moved by hand): left alone.
    HasNotes,
    /// The v1 store has no rows to move.
    NothingToMove,
    Moved(Report),
}

/// The move, once, at the engine's start: when the person's memory is v2
/// (`wants_v2`), the check has not run before, no note exists yet, and the v1
/// store has rows, those rows are moved and the fact is recorded under
/// `.state`; a Mac that already has notes is recorded and left alone. `rows`
/// is asked only then. An `Err` (the rows could not be read, a record could
/// not be saved) records nothing, so the next start tries again. Delete
/// `.state/moved-from-v1.json` to have it looked at again.
pub fn upgrade_once(
    store: &Store,
    wants_v2: bool,
    rows: impl FnOnce() -> Result<Vec<V1Row>, String>,
) -> Result<Upgrade, String> {
    if !wants_v2 {
        return Ok(Upgrade::NotWanted);
    }
    if store.moved_record().is_some() {
        return Ok(Upgrade::AlreadyLooked);
    }
    let record = |text: serde_json::Value| store.record_moved(&text.to_string()).map_err(|e| format!("could not record the move: {e}"));
    if store.has_notes() {
        record(serde_json::json!({ "result": "left alone: notes were already here" }))?;
        return Ok(Upgrade::HasNotes);
    }
    let rows = rows()?;
    if rows.is_empty() {
        return Ok(Upgrade::NothingToMove);
    }
    let report = migrate(store, &rows, true);
    record(report.record())?;
    Ok(Upgrade::Moved(report))
}
