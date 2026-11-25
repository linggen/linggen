//! The slice: what the OTHER agents remember for a session, put in the room
//! it was given and tagged with where each line came from.
//!
//! Inside one agent the person's lines come first (they are the same in every
//! folder), then the projects, the nearest first, newest first inside each.
//! When two or more agents have lines, none takes more than the share it is
//! given. Lines that do not fit are counted, never cut mid-line. How big the
//! room is, and who gets how much of it, is decided in
//! `own_memory::forward`; this file only fills what it is handed
//! ([`super::room::Room`]).

use super::lines::{place_results, Line, Skip};
use super::readers::{Agent, Registry};
use super::redact::{clean, Clean};
use super::room::Room;
use super::{Env, Scope};
use std::path::Path;

/// A source that gave nothing, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceNote {
    pub source: String,
    pub state: String,
}

/// The lines forwarded to one host for one scope.
#[derive(Clone, Debug)]
pub struct Slice {
    env: Env,
    lines: Vec<Line>,
    /// How many lines were left out: what the room could not hold, and the
    /// project lines whose heading names no folder of the session's chain.
    pub cut: usize,
    /// Sources that gave no lines.
    pub notes: Vec<SourceNote>,
}

/// The other agents' index lines for a session at `cwd`, for the calling
/// `host` (an agent never gets its own memory back; a host Linggen knows no
/// memory of gets everyone's), in `room`, no agent taking more than `each`
/// when several have lines.
pub fn slice(env: &Env, host: Option<&Agent>, cwd: Option<&Path>, room: Room, each: Room) -> Slice {
    let scope = Scope { cwd: cwd.map(Path::to_path_buf) };
    let mut by_agent: Vec<(Agent, Vec<Line>)> = Vec::new();
    let mut notes = Vec::new();
    let mut unmatched = 0;
    for result in place_results(&Registry::load(env), env, &scope).into_iter().filter(|r| Some(&r.agent) != host) {
        unmatched += result.unmatched;
        match result.outcome {
            Ok(lines) => match by_agent.iter_mut().find(|(a, _)| *a == result.agent) {
                Some((_, all)) => all.extend(lines),
                None => by_agent.push((result.agent, lines)),
            },
            Err(Skip::NoData | Skip::Older(_)) => {}
            Err(skip) => notes.push(SourceNote { source: result.source, state: skip.to_string() }),
        }
    }
    let total: usize = by_agent.iter().map(|(_, lines)| lines.len()).sum();
    let share = if by_agent.len() > 1 { each } else { room };
    let mut kept: Vec<Line> = Vec::new();
    for (_, mut lines) in by_agent {
        order(&mut lines);
        let left = room.minus(body(env, &kept).as_str());
        let mine = fit(env, lines, share.at_most(left));
        kept.extend(mine);
    }
    Slice { env: env.clone(), cut: total - kept.len() + unmatched, lines: kept, notes }
}

/// The person first, then the nearest project; newest first inside each.
fn order(lines: &mut [Line]) {
    lines.sort_by(|a, b| {
        a.distance
            .cmp(&b.distance)
            .then(b.changed.cmp(&a.changed))
            .then(a.source.cmp(&b.source))
            .then(a.text.cmp(&b.text))
    });
}

/// The leading lines that fit `room` once rendered with their headers.
fn fit(env: &Env, candidates: Vec<Line>, room: Room) -> Vec<Line> {
    let mut kept: Vec<Line> = Vec::new();
    for line in candidates {
        kept.push(line);
        if !room.holds(body(env, &kept).as_str()) {
            kept.pop();
            break;
        }
    }
    kept
}

/// The lines grouped under their headers, in the order given, with a new
/// header wherever the group changes.
fn body(env: &Env, lines: &[Line]) -> Clean {
    let mut out = String::new();
    let mut group: Option<(&str, &str)> = None;
    for line in lines {
        let key = (line.source.as_str(), line.about.as_str());
        if group != Some(key) {
            group = Some(key);
            let notes_in = match line.distance {
                Some(_) => format!(" — notes in {}", env.tilde(&line.place)),
                None => String::new(),
            };
            out.push_str(&format!("{} · {}{notes_in}\n", line.agent.label(), line.about));
        }
        out.push_str(&line.text);
        out.push('\n');
    }
    clean(&out)
}

impl Slice {
    /// Whether anything is forwarded.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The lines as data: source id, agent, group, text.
    pub fn entries(&self) -> Vec<(&str, &Agent, &str, Clean)> {
        self.lines
            .iter()
            .map(|l| (l.source.as_str(), &l.agent, l.about.as_str(), clean(&l.text)))
            .collect()
    }

    /// The lines under their headers, as the block carries them; empty when
    /// there is nothing to forward.
    pub fn body(&self) -> Clean {
        body(&self.env, &self.lines)
    }
}
