//! The slice: what the OTHER agents remember for a session, trimmed to a
//! budget and tagged with where each line came from.
//!
//! Order: the nearest project first, then the person, newest first inside
//! each. The person's lines keep a quarter of the budget so a long project
//! index cannot push them out. Lines that do not fit are counted, not cut
//! mid-line.

use super::redact::{clean, Clean};
use super::sources::{Line, Skip, SOURCES};
use super::{Agent, Env, Scope};
use std::path::Path;

/// What a session start is forwarded, at most, in bytes, of every source
/// together (Linggen's own notes and the other agents' lines; how the two
/// share it is [`crate::own_memory::forward`]) — aligned to neither agent's
/// own limit: Claude Code's index is its own memory (25 KB), this is what it
/// is told about the rest.
pub const BUDGET_BYTES: usize = 4096;
/// Kept free for the "N more" line.
const FOOTER_BYTES: usize = 128;
/// The person's share of the budget.
const PERSON_SHARE: usize = 4;

/// A source that gave nothing, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceNote {
    pub source: &'static str,
    pub state: String,
}

/// The lines forwarded to one host for one scope.
#[derive(Clone, Debug)]
pub struct Slice {
    env: Env,
    lines: Vec<Line>,
    /// How many lines the budget left out.
    pub cut: usize,
    /// Sources that gave no lines.
    pub notes: Vec<SourceNote>,
}

/// The other agents' index lines for a session at `cwd`, for the calling
/// `host` (an agent never gets its own memory back; a host Linggen knows no
/// memory of gets everyone's), in at most `budget` bytes.
pub fn slice(env: &Env, host: Option<Agent>, cwd: Option<&Path>, budget: usize) -> Slice {
    let scope = Scope { cwd: cwd.map(Path::to_path_buf) };
    let mut all = Vec::new();
    let mut notes = Vec::new();
    for source in SOURCES.iter().filter(|s| Some(s.agent) != host) {
        match (source.lines)(env, &scope) {
            Ok(lines) => all.extend(lines),
            Err(Skip::NoData) => {}
            Err(skip) => notes.push(SourceNote { source: source.id, state: skip.to_string() }),
        }
    }
    let total = all.len();
    let lines = fit(env, all, budget);
    Slice { env: env.clone(), cut: total - lines.len(), lines, notes }
}

/// Newest first; a project's nearer lines before a farther one's.
fn order(lines: &mut [Line]) {
    lines.sort_by(|a, b| {
        a.distance
            .cmp(&b.distance)
            .then(b.changed.cmp(&a.changed))
    });
}

/// The lines that fit: the person's share first, the projects in the rest.
fn fit(env: &Env, all: Vec<Line>, budget: usize) -> Vec<Line> {
    let (mut person, mut projects): (Vec<Line>, Vec<Line>) =
        all.into_iter().partition(|l| l.distance.is_none());
    order(&mut person);
    order(&mut projects);
    let room = budget.saturating_sub(FOOTER_BYTES + INTRO.len());
    let person = take_while_fits(env, person, Vec::new(), room / PERSON_SHARE);
    let used = body(env, &person).as_str().len();
    let mut kept = take_while_fits(env, projects, Vec::new(), room.saturating_sub(used));
    kept.extend(person);
    kept
}

/// Add `candidates` one by one to `kept` while the rendered whole stays
/// within `limit` bytes; stop at the first that does not.
fn take_while_fits(env: &Env, candidates: Vec<Line>, mut kept: Vec<Line>, limit: usize) -> Vec<Line> {
    for line in candidates {
        kept.push(line);
        if body(env, &kept).as_str().len() > limit {
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
        let key = (line.source, line.about.as_str());
        if group != Some(key) {
            group = Some(key);
            out.push_str(&format!(
                "{} · {} — notes in {}\n",
                line.agent.label(),
                line.about,
                env.tilde(&line.place)
            ));
        }
        out.push_str(&line.text);
        out.push('\n');
    }
    clean(&out)
}

const INTRO: &str = "What other agents remember here (read-only: Linggen forwards it and never edits it). \
Open a note with memory_agents op=read path=<folder>/<file>; find words with op=search. \
If two disagree, the newer one wins unless the person says otherwise.\n";

impl Slice {
    /// Whether anything is forwarded.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The lines as data: source id, agent, group, text.
    pub fn entries(&self) -> Vec<(&'static str, Agent, &str, Clean)> {
        self.lines
            .iter()
            .map(|l| (l.source, l.agent, l.about.as_str(), clean(&l.text)))
            .collect()
    }

    /// What a host is injected at session start; empty when there is
    /// nothing to forward.
    pub fn block(&self) -> Clean {
        if self.lines.is_empty() {
            return clean("");
        }
        let mut text = format!("{INTRO}{}", body(&self.env, &self.lines));
        if self.cut > 0 {
            text.push_str(&format!(
                "({} more lines left out; memory_agents op=search finds them.)\n",
                self.cut
            ));
        }
        clean(&text)
    }
}
