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
//! The same notes are also forwarded to a coding host at its session start
//! ([`index_for_forwarding`], put together with the other agents' lines in
//! [`super::forward`]): a smaller budget, the nearest project first, then the
//! person's standing rules and decisions.

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
    /// The person's standing rules, decisions and gotchas, in a forwarded
    /// slice: the person's own words (`false`) before an agent's, then the
    /// most read.
    Standing(bool, std::cmp::Reverse<usize>),
    Used(std::cmp::Reverse<usize>),
    Rest,
}

/// The index for a session of `audience` standing in `cwd`, at `now`.
pub fn index(store: &super::Store, audience: Audience, cwd: Option<&Path>, now: i64) -> Index {
    index_in(store, audience, cwd, now, Reach::Everything, Order::Session, HEADER, INDEX_BYTES)
}

/// What a coding host is forwarded of these notes at its session start, in
/// at most `bytes`: the same lines as the index, for [`Audience::Work`] (the
/// fence keeps body, mind, people and moments back), ordered by
/// [`Order::Forwarded`].
pub fn index_for_forwarding(store: &super::Store, cwd: Option<&Path>, now: i64, bytes: usize) -> Index {
    index_in(store, Audience::Work, cwd, now, Reach::Everything, Order::Forwarded, FORWARD_HEADER, bytes)
}

/// An app's own index: the notes filed at its folder or above it, and the
/// person's when `person` — nothing of any other project, app or area.
pub fn index_for_app(store: &super::Store, cwd: Option<&Path>, person: bool, now: i64) -> Index {
    index_in(store, Audience::Work, cwd, now, Reach::PlaceAnd { person }, Order::Session, HEADER, INDEX_BYTES)
}

/// What comes first in an index.
#[derive(Clone, Copy)]
enum Order {
    /// A session's own index: the person's notes, then the session's scope.
    Session,
    /// A coding host's forwarded slice: the session's projects, then the
    /// person's standing rules, decisions and gotchas — what the person has
    /// decided matters more there than who they are.
    Forwarded,
}

/// How much of the store an index may draw on.
#[derive(Clone, Copy)]
enum Reach {
    Everything,
    /// The session's own place, and the person's notes if `person`.
    PlaceAnd { person: bool },
}

#[allow(clippy::too_many_arguments)]
fn index_in(store: &super::Store, audience: Audience, cwd: Option<&Path>, now: i64, reach: Reach, order: Order, header: &str, bytes: usize) -> Index {
    let (all, _) = fence::seen(store, audience);
    let uses = store.uses_since(now - USE_WINDOW_SECS);
    let chain = cwd.map(|c| project_chain(store.env(), c)).unwrap_or_default();
    let mut ranked: Vec<(Rank, Seen)> = all
        .into_iter()
        .filter(|s| s.entry.void.is_none() && s.scope != Scope::Chronicle && reaches(reach, &s.scope, &chain))
        .map(|s| (rank(&s, audience, order, &chain, &uses), s))
        .collect();
    ranked.sort_by(|(ra, a), (rb, b)| {
        ra.cmp(rb)
            .then(b.entry.updated.cmp(&a.entry.updated))
            .then(a.entry.id.cmp(&b.entry.id))
    });
    let reserved = match order {
        Order::Session => 0,
        Order::Forwarded => bytes / STANDING_SHARE,
    };
    fit(ranked, header, bytes, reserved)
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

fn rank(s: &Seen, audience: Audience, order: Order, chain: &[std::path::PathBuf], uses: &std::collections::HashMap<String, usize>) -> Rank {
    match &s.scope {
        Scope::Person => match order {
            Order::Session => return Rank::Person,
            Order::Forwarded if standing(s.entry.kind) => {
                let read = uses.get(&s.entry.id).copied().unwrap_or(0);
                return Rank::Standing(s.entry.voice != Voice::User, std::cmp::Reverse(read));
            }
            Order::Forwarded => {}
        },
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

/// A forwarded slice keeps this share (1 in N) of its bytes for the person's
/// standing rules and decisions.
const STANDING_SHARE: usize = 2;

/// A kind that stays true until the person says otherwise.
fn standing(kind: Kind) -> bool {
    matches!(kind, Kind::Rule | Kind::Decision | Kind::Gotcha)
}

const HEADER: &str = "# Linggen memory — what Linggen keeps about the person and the work";
const FORWARD_HEADER: &str = "Linggen's own notes — what the person decided, ruled or warned about, this project first. \
Summaries only: memory_notes op=read id=<id> opens one.";

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

/// Keep the lines that fit, in order. Lines of [`Rank::Standing`] are
/// guaranteed `reserved` bytes of the budget, so a long project index cannot
/// push the person's rules out; they stay where the order puts them.
fn fit(ordered: Vec<(Rank, Seen)>, header: &str, bytes: usize, reserved: usize) -> Index {
    if ordered.is_empty() {
        return Index { text: String::new(), shown: 0, left_out: 0 };
    }
    let total = ordered.len();
    // The header and a footer line are kept free, so a full index still says so.
    let base = header.len() + 1 + footer(total).len() + 1;
    let lines_of: Vec<String> = ordered.iter().map(|(_, s)| line_of(s)).collect();
    let is_standing: Vec<bool> = ordered.iter().map(|(r, _)| matches!(r, Rank::Standing(..))).collect();
    let mut guaranteed = vec![false; total];
    let mut spent = 0;
    for i in (0..total).filter(|i| is_standing[*i]) {
        if spent + lines_of[i].len() + 1 > reserved {
            break;
        }
        spent += lines_of[i].len() + 1;
        guaranteed[i] = true;
    }
    let (mut count, mut used, mut pending, mut closed) = (0, base, spent, false);
    let mut body = Vec::new();
    for i in 0..total {
        let need = lines_of[i].len() + 1;
        let place = match guaranteed[i] {
            true => {
                pending -= need;
                true
            }
            false => !closed && 2 + count < INDEX_LINES && used + pending + need <= bytes,
        };
        closed |= !place && !guaranteed[i];
        if place {
            count += 1;
            used += need;
            body.push(lines_of[i].clone());
        }
    }
    let left_out = total - body.len();
    if body.is_empty() {
        return Index { text: String::new(), shown: 0, left_out: total };
    }
    let mut text = format!("{header}\n{}\n", body.join("\n"));
    if left_out > 0 {
        text.push_str(&footer(left_out));
        text.push('\n');
    }
    Index { text, shown: body.len(), left_out }
}
