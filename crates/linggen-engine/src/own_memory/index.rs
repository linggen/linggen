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

use super::fence::{self, Audience, Seen};
use super::format::Scope;
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
