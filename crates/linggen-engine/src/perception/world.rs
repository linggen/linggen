//! The world block: every reading a person's session is given each turn —
//! this machine, the Linggen page in front, one line per app, the other
//! device, and the doorbell (`doc/perception-spec.md` §2, §4).
//!
//! Readings only. Nothing here judges, compares or concludes; what a reading
//! means is the agent's to say.

use crate::engine::agent::Front;

/// The whole block, in characters, bullets and newlines included. About 400
/// tokens of English; CJK text runs about a token a character, so a block
/// full of it costs more.
pub const BLOCK_CHARS: usize = 1600;

/// How a group of lines goes when the block is over [`BLOCK_CHARS`]: app
/// lines first, the oldest reading first; then the other device's lines, all
/// together. The machine, the page in front and the doorbell always stay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Keep {
    Always,
    Peer,
    App,
}

/// Lines that stay or go together.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub keep: Keep,
    pub lines: Vec<String>,
}

impl Group {
    fn of(keep: Keep, lines: Vec<String>) -> Option<Self> {
        (!lines.is_empty()).then_some(Self { keep, lines })
    }
}

/// One look at the world.
pub struct Look<'a> {
    /// The reader, for "since you last looked". `None`: no reader, no claim.
    pub session_id: Option<&'a str>,
    /// Whether this look tells the reader (a turn) or only peeks.
    pub mark: bool,
    /// Whether the reader holds `recent_activity`, so the doorbell may name it.
    pub door: bool,
    /// The page in front, from presence; `None` when presence cannot be read.
    pub front: Option<Option<Front>>,
    /// This turn hands the reader a mission fault as its kickoff: the doorbell
    /// does not also say the Mac recorded it.
    pub told_fault: bool,
    /// Unix seconds.
    pub now: u64,
}

/// The groups in the order they are shown.
pub fn groups(look: &Look) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    out.extend(Group::of(Keep::Always, super::state::own_lines()));
    let front = look.front.as_ref().map(|f| {
        front_line(
            f.as_ref(),
            super::stretch::in_front(look.now),
            look.now,
            local_hhmm,
        )
    });
    out.extend(Group::of(Keep::Always, front.into_iter().collect()));
    out.extend(
        super::apps::lines()
            .into_iter()
            .filter_map(|l| Group::of(Keep::App, vec![l])),
    );
    out.extend(Group::of(Keep::Peer, super::state::peer_lines()));
    out.extend(Group::of(
        Keep::Always,
        super::state::doorbell(look.session_id, look.mark, look.door, look.told_fault),
    ));
    out
}

/// Every line of one look, unformatted — what `sense` reports, so the tool
/// and the block never disagree.
pub fn read_lines(look: &Look) -> Vec<String> {
    groups(look).into_iter().flat_map(|g| g.lines).collect()
}

/// The block's bullets, within [`BLOCK_CHARS`]. `None` when there is nothing
/// to say.
pub fn block(look: &Look) -> Option<String> {
    render(fit(groups(look)))
}

/// Drop groups, lowest kept first and the last of a kind first, until the
/// block fits. A line is never cut.
pub fn fit(mut groups: Vec<Group>) -> Vec<Group> {
    while chars(&groups) > BLOCK_CHARS {
        let Some(at) = droppable(&groups) else {
            break;
        };
        groups.remove(at);
    }
    groups
}

fn droppable(groups: &[Group]) -> Option<usize> {
    let lowest = groups
        .iter()
        .map(|g| g.keep)
        .filter(|k| *k != Keep::Always)
        .max()?;
    groups.iter().rposition(|g| g.keep == lowest)
}

fn chars(groups: &[Group]) -> usize {
    render(groups.to_vec()).map_or(0, |b| b.chars().count())
}

/// One bullet a line.
pub fn render(groups: Vec<Group>) -> Option<String> {
    let lines: Vec<String> = groups
        .into_iter()
        .flat_map(|g| g.lines)
        .map(|l| format!("- {l}"))
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Which Linggen page is in front and since when — from presence and the
/// open stretch of use — or that none is. Never another program: only
/// Linggen's own pages beat.
pub fn front_line(
    front: Option<&Front>,
    open: Option<(String, u64)>,
    now: u64,
    hhmm: fn(u64) -> String,
) -> String {
    let Some(f) = front else {
        return "no Linggen page in front".to_string();
    };
    let app = f.app.as_deref().unwrap_or(super::stretch::MAIN_PAGE);
    let page = page_name(app);
    match open.filter(|(a, _)| a == app) {
        Some((_, since)) => format!(
            "in front: {page} since {} ({})",
            hhmm(since),
            minutes(now.saturating_sub(since))
        ),
        None => format!(
            "in front: {page} · no input for {}",
            minutes(now.saturating_sub(f.last_input_at))
        ),
    }
}

fn page_name(app: &str) -> String {
    if app == super::stretch::MAIN_PAGE {
        return "Linggen's main page".to_string();
    }
    format!("the {} page", super::activity::clean(app, 32))
}

/// A span to the minute, once it is one.
fn minutes(secs: u64) -> String {
    let whole = if secs >= 60 { secs - secs % 60 } else { secs };
    super::query::readable(whole)
}

fn local_hhmm(secs: u64) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
