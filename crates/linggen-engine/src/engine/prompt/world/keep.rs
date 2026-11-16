//! The world block kept in the thread.
//!
//! A turn's block is built once, when the turn starts, and kept right after
//! the turn's words: every request of the turn sends it as it was, and the
//! next turn's request begins with this one's. In the session's file it is a
//! row of its own sender ([`WORLD_SENDER`]) — read back as a system note,
//! never listed as a message, never spoken, never anyone's words.
//!
//! A turn whose readings are the same as the last kept block's, but for the
//! clock (its `as of` time, ages and durations), keeps no new one: the earlier
//! block still stands, and says when it was read.

use super::PromptPurpose;
use crate::engine::AgentEngine;
use crate::message::ChatMessage;
use crate::state_fs::sessions::ChatMsg;
use regex::Regex;
use std::sync::LazyLock;

/// The pseudo-sender of a kept block's row in the session's file.
pub const WORLD_SENDER: &str = "world";
/// How every world block begins (the `world_block` template's first line).
pub const WORLD_HEADING: &str = "# World\n";

/// A world block in a thread.
pub fn is_world_note(m: &ChatMessage) -> bool {
    m.role == "system" && m.content.starts_with(WORLD_HEADING)
}

/// A kept world block's row in the session's file.
pub fn is_world_row(r: &ChatMsg) -> bool {
    r.from_id == WORLD_SENDER
}

impl AgentEngine {
    /// At a turn's start, once its words are in the thread: the world as of
    /// now, kept right after them. Returns the block when one was kept, for
    /// the caller to save as the session's row; `None` when the session is
    /// not given the world, this turn already has its block (a turn's block
    /// never changes), or the readings are the last block's.
    pub fn keep_world(&mut self) -> Option<String> {
        let note = self.new_world_block(PromptPurpose::Turn)?;
        self.chat_history
            .push(ChatMessage::new("system", note.clone()));
        Some(note)
    }

    /// The block the next turn would keep, without keeping it or stamping
    /// the doorbell — for the prompt export.
    pub(in crate::engine::prompt) fn world_preview(&self) -> Option<String> {
        self.new_world_block(PromptPurpose::Preview)
    }

    fn new_world_block(&self, purpose: PromptPurpose) -> Option<String> {
        if self.turn_has_world() {
            return None;
        }
        let allowed = self.allowed_tool_names();
        let note = self.world_note(allowed.as_ref(), purpose)?;
        let unchanged = self
            .last_world()
            .is_some_and(|last| same_readings(&last.content, &note));
        (!unchanged).then_some(note)
    }

    /// Whether a block was kept since the thread's last user message.
    fn turn_has_world(&self) -> bool {
        self.chat_history
            .iter()
            .rev()
            .take_while(|m| m.role != "user")
            .any(is_world_note)
    }

    fn last_world(&self) -> Option<&ChatMessage> {
        self.chat_history.iter().rev().find(|m| is_world_note(m))
    }
}

/// The parts of a block that move with the clock alone.
static CLOCK_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"as of \d{4}-\d{2}-\d{2} \d{2}:\d{2}",
        r"|since \d{1,2}:\d{2} \([^)\n]*\)",
        r"|no input for [^\n]*",
        r"|just now|age unknown",
        r"|\b\d+[mhd] ago\b",
    ))
    .expect("clock words")
});

/// Whether two blocks say the same, but for the clock.
pub fn same_readings(a: &str, b: &str) -> bool {
    CLOCK_WORDS.replace_all(a, "~") == CLOCK_WORDS.replace_all(b, "~")
}
