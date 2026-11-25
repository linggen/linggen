//! What a coding host is forwarded at its session start: Linggen's own notes
//! and what the other agents remember, in ONE block with ONE budget
//! ([`BUDGET_BYTES`]). The split is decided here and nowhere else:
//!
//! - the other agents' lines get up to half of the budget, always;
//! - Linggen's notes get the other half and whatever the other agents'
//!   lines leave unused (their notes are the person's own decisions, so
//!   they come first in the block and take the slack, not the other way
//!   round).
//!
//! Inside Linggen's share, half is kept for the person's standing rules and
//! decisions (`index::STANDING_SHARE`), the rest goes to the session's
//! projects first, so neither can push the other out.
//!
//! Which Linggen notes, in what order, and what the fence keeps back is
//! [`super::index::index_for_forwarding`] (the fence is [`super::fence`]);
//! the other agents' lines are [`crate::outside_memory::slice`]. This file
//! only decides the shares and joins the two. Summaries and ids only, never
//! a body, and nothing leaves unredacted.

use super::index::{index_for_forwarding, Index};
use super::Store;
use crate::outside_memory::{self as outside, Agent, Env, Slice, BUDGET_BYTES};
use std::path::Path;

/// The block, and the parts it was made from.
pub struct SessionStart {
    /// Empty when there is nothing to forward.
    pub block: String,
    /// Linggen's own notes in it (empty text when there are none).
    pub own: Index,
    /// The other agents' lines in it.
    pub others: Slice,
}

/// What a session at `cwd` is forwarded. `own` is Linggen's own store when
/// its notes are served (`outside_agents = "v2"`), `None` when they are not.
pub fn session_start(env: &Env, own: Option<&Store>, host: Option<&Agent>, cwd: Option<&Path>, now: i64) -> SessionStart {
    let others_room = if own.is_some() { BUDGET_BYTES / 2 } else { BUDGET_BYTES };
    let others = outside::slice(env, host, cwd, others_room);
    let others_text = others.block();
    // The blocks are joined by a blank line and end in a newline: 3 bytes.
    let own_room = BUDGET_BYTES.saturating_sub(others_text.as_str().len() + 3);
    let own = match own {
        Some(store) => index_for_forwarding(store, cwd, now, own_room),
        None => Index { text: String::new(), shown: 0, left_out: 0 },
    };
    let block = [outside::clean(&own.text), others_text]
        .iter()
        .map(|part| part.as_str().trim_end())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    SessionStart { block: if block.is_empty() { block } else { format!("{block}\n") }, own, others }
}
