//! The index: what a session sees of Linggen's own notes at its start.
//! Made by the engine from the entries' headers, never kept by hand, so no
//! note falls out by accident — what does not fit is "forgotten": still
//! found by search.
//!
//! Order (design § 4a): the person's notes (all) → the session's own scope
//! (the nearest project first; for the companion, her areas) → notes read in
//! the last 90 days, most read first → the rest, newest first. Budget: 200
//! lines and 25 KB, Claude Code's own limit for its index. The chronicle is
//! not in it: it is long, and found by search when the past is asked about.
//!
//! What a coding host is forwarded of the same notes at its session start is
//! [`notes_for_forwarding`]: grouped by topic, the person's rules one by one,
//! and only the person's notes and the projects in the session's chain
//! (nothing is promoted from one to the other). How much of it fits is
//! decided in [`super::forward`].

use super::fence::{self, Audience, Seen};
use super::format::{Kind, Scope, Voice};
use super::TOOL;
use crate::outside_memory::project_chain;
use std::path::Path;

pub const INDEX_LINES: usize = 200;
pub const INDEX_BYTES: usize = 25 * 1024;
pub(super) const USE_WINDOW_SECS: i64 = 90 * 24 * 3600;

/// The index text and what it left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Index {
    /// Empty when there are no notes at all.
    pub text: String,
    pub shown: usize,
    pub left_out: usize,
}

/// Where an entry stands in the order: smaller is earlier.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    Person,
    Here(usize),
    Used(std::cmp::Reverse<usize>),
    Rest,
}

/// The index for a session of `audience` standing in `cwd`, at `now`.
pub fn index(store: &super::Store, audience: Audience, cwd: Option<&Path>, now: i64) -> Index {
    index_in(store, audience, cwd, now, Reach::Everything)
}

/// An app's own index: the notes filed at its folder or above it, and the
/// person's when `person` — nothing of any other project, app or area.
pub fn index_for_app(store: &super::Store, cwd: Option<&Path>, person: bool, now: i64) -> Index {
    index_in(store, Audience::Work, cwd, now, Reach::PlaceAnd { person })
}

/// How much of the store an index may draw on.
#[derive(Clone, Copy)]
enum Reach {
    Everything,
    /// The session's own place, and the person's notes if `person`.
    PlaceAnd { person: bool },
}

fn index_in(store: &super::Store, audience: Audience, cwd: Option<&Path>, now: i64, reach: Reach) -> Index {
    let (all, _) = fence::seen(store, audience);
    let uses = store.uses_since(now - USE_WINDOW_SECS);
    let chain = cwd.map(|c| project_chain(store.env(), c)).unwrap_or_default();
    let mut ranked: Vec<(Rank, Seen)> = all
        .into_iter()
        .filter(|s| s.entry.void.is_none() && s.scope != Scope::Chronicle && reaches(reach, &s.scope, &chain))
        .map(|s| (rank(&s, audience, &chain, &uses), s))
        .collect();
    ranked.sort_by(|(ra, a), (rb, b)| {
        ra.cmp(rb)
            .then(b.entry.updated.cmp(&a.entry.updated))
            .then(a.entry.id.cmp(&b.entry.id))
    });
    fit(ranked.into_iter().map(|(_, s)| s).collect())
}

/// Whether an index of this `reach` may list a note of `scope`.
fn reaches(reach: Reach, scope: &Scope, chain: &[std::path::PathBuf]) -> bool {
    match (reach, scope) {
        (Reach::Everything, _) => true,
        (Reach::PlaceAnd { person }, Scope::Person) => person,
        (Reach::PlaceAnd { .. }, Scope::Project(p)) => chain.contains(p),
        (Reach::PlaceAnd { .. }, _) => false,
    }
}

fn rank(s: &Seen, audience: Audience, chain: &[std::path::PathBuf], uses: &std::collections::HashMap<String, usize>) -> Rank {
    match &s.scope {
        Scope::Person => return Rank::Person,
        Scope::Project(p) => {
            if let Some(distance) = chain.iter().position(|c| c == p) {
                return Rank::Here(distance);
            }
        }
        Scope::Area(_) if audience == Audience::Companion => return Rank::Here(0),
        _ => {}
    }
    match uses.get(&s.entry.id) {
        Some(n) => Rank::Used(std::cmp::Reverse(*n)),
        None => Rank::Rest,
    }
}

const HEADER: &str = "# Linggen memory — what Linggen keeps about the person and the work";

fn line_of(s: &Seen) -> String {
    let raise = if s.entry.no_raise { " · don't raise" } else { "" };
    format!(
        "- {} ({} · {} · id {}{raise})",
        s.entry.summary,
        s.entry.kind.word(),
        s.topic,
        s.entry.id
    )
}

fn footer(left_out: usize) -> String {
    format!("({left_out} more notes not shown; {TOOL} op=search finds them.)")
}

fn fit(ordered: Vec<Seen>) -> Index {
    if ordered.is_empty() {
        return Index { text: String::new(), shown: 0, left_out: 0 };
    }
    let total = ordered.len();
    // The header and a footer line are kept free, so a full index still says so.
    let mut lines = 1 + 1;
    let mut bytes = HEADER.len() + 1 + footer(total).len() + 1;
    let mut body = Vec::new();
    for s in &ordered {
        let line = line_of(s);
        if lines + 1 > INDEX_LINES || bytes + line.len() + 1 > INDEX_BYTES {
            break;
        }
        lines += 1;
        bytes += line.len() + 1;
        body.push(line);
    }
    let left_out = total - body.len();
    let mut text = format!("{HEADER}\n{}\n", body.join("\n"));
    if left_out > 0 {
        text.push_str(&footer(left_out));
        text.push('\n');
    }
    Index { text, shown: body.len(), left_out }
}

/// Linggen's own notes as a coding host is forwarded them, before anything is
/// cut: the lines in the order they should be kept.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForwardedNotes {
    /// The person's rules, one line each: the person's own words first, then
    /// the most read, then newest.
    pub rules: Vec<String>,
    /// One line per topic file: its name, how many notes, a gist (the newest
    /// note's summary) and the path that opens it. The nearest project
    /// first, then the person's topics; newest first inside each.
    pub topics: Vec<String>,
}

/// The notes a coding host may see ([`Audience::Work`]: the fence keeps back
/// body, mind, people, moments and the chronicle), for a session at `cwd`:
/// person-scope notes, and project notes of the folders in its chain. A note
/// of any other project, and of any area, is found by search, not listed.
pub fn notes_for_forwarding(store: &super::Store, cwd: Option<&Path>, now: i64) -> ForwardedNotes {
    let (all, _) = fence::seen(store, Audience::Work);
    let uses = store.uses_since(now - USE_WINDOW_SECS);
    let chain = cwd.map(|c| project_chain(store.env(), c)).unwrap_or_default();
    let mut rules: Vec<&Seen> = Vec::new();
    // One group per topic file: where it stands (the person's, or a chain
    // distance) and its notes.
    let mut files: Vec<(Where, Vec<&Seen>)> = Vec::new();
    for note in all.iter().filter(|s| s.entry.void.is_none()) {
        let at = match &note.scope {
            Scope::Person if note.entry.kind == Kind::Rule => {
                rules.push(note);
                continue;
            }
            Scope::Person => Where::Person,
            Scope::Project(p) => match chain.iter().position(|c| c == p) {
                Some(distance) => Where::Project(distance),
                None => continue,
            },
            _ => continue,
        };
        match files.iter_mut().find(|(_, notes)| notes[0].rel == note.rel) {
            Some((_, notes)) => notes.push(note),
            None => files.push((at, vec![note])),
        }
    }
    rules.sort_by(|a, b| {
        let reads = |s: &Seen| uses.get(&s.entry.id).copied().unwrap_or(0);
        (a.entry.voice != Voice::User)
            .cmp(&(b.entry.voice != Voice::User))
            .then(reads(b).cmp(&reads(a)))
            .then(b.entry.updated.cmp(&a.entry.updated))
            .then(a.entry.id.cmp(&b.entry.id))
    });
    files.sort_by(|(wa, na), (wb, nb)| wa.cmp(wb).then(newest(nb).cmp(newest(na))).then(na[0].rel.cmp(&nb[0].rel)));
    ForwardedNotes {
        rules: rules.into_iter().map(line_of).collect(),
        topics: files.iter().map(|(at, notes)| topic_line(store, at, &chain, notes)).collect(),
    }
}

/// Where a topic file stands for a session: the person's come after the
/// session's own projects, nearest first.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Where {
    Project(usize),
    Person,
}

fn newest<'a>(notes: &[&'a Seen]) -> &'a str {
    notes.iter().map(|n| n.entry.updated.as_str()).max().unwrap_or("")
}

/// `- Build (3 notes, project ~/work/app): the gist — memory_notes op=read path=…`
fn topic_line(store: &super::Store, at: &Where, chain: &[std::path::PathBuf], notes: &[&Seen]) -> String {
    let first = notes[0];
    let gist = notes
        .iter()
        .max_by(|a, b| a.entry.updated.cmp(&b.entry.updated).then(b.entry.id.cmp(&a.entry.id)))
        .map_or("", |n| n.entry.summary.as_str());
    let whose = match at {
        Where::Person => "the person".to_string(),
        Where::Project(d) => format!("project {}", store.env().tilde(&chain[*d])),
    };
    let count = notes.len();
    let plural = if count == 1 { "" } else { "s" };
    format!(
        "- {} ({count} note{plural}, {whose}): {} — {TOOL} op=read path={}",
        first.topic,
        crate::outside_memory::shorten(gist, GIST_CHARS),
        first.rel
    )
}

/// A topic's gist, in characters.
const GIST_CHARS: usize = 80;
