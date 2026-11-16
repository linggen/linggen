//! The world block in a turn: who is given it, and the note it rides in.
//!
//! One block, the same in every session a person is in — Ling's chats, an
//! app's chat, Yinyue's thread, Yinyue at an app's table — whatever the
//! session's tool list says (`doc/session-spec.md` § Who is given the world).
//! It goes beside the turn, never into the system prompt, so a changed reading
//! never touches the cached prefix.

use super::PromptPurpose;
use crate::engine::AgentEngine;
use std::collections::HashSet;

/// Whether a session's turns are given the world.
///
/// Sessions a person is in: made by `user` (a chat they opened), `skill` (an
/// app's chat) or `agent` (Yinyue's daily thread, a hand-off). Not `mission`
/// or `external`: nobody is there to speak to, and a delegated worker should
/// not pay for it. Not an engine no stored session owns (`None`: a `Task`
/// child). Never a proxy-room consumer: the readings are the owner's.
pub fn world_rides(creator: Option<&str>, consumer: bool) -> bool {
    !consumer && matches!(creator, Some("user" | "skill" | "agent"))
}

impl AgentEngine {
    /// This turn's world block, rendered, or `None` when the session is not
    /// given it or there is nothing to say.
    pub(super) fn world_note(
        &self,
        allowed_tools: Option<&HashSet<String>>,
        purpose: PromptPurpose,
    ) -> Option<String> {
        if !world_rides(
            self.session_creator.as_deref(),
            self.prompt_profile.consumer_frame,
        ) {
            return None;
        }
        let now = crate::util::now_ts_secs();
        let look = crate::perception::world::Look {
            session_id: self.tools.builtins.session_id.as_deref(),
            mark: purpose == PromptPurpose::Turn,
            door: allowed_tools.is_none_or(|s| s.contains("recent_activity"))
                && !self.is_withheld("recent_activity"),
            front: self
                .tools
                .builtins
                .get_manager()
                .map(|m| m.presence_front(now)),
            now,
        };
        let lines = crate::perception::world::block(&look)?;
        let note = self.prompt_store.render_or_fallback(
            crate::prompts::keys::SYSTEM_WORLD_BLOCK,
            &[("lines", &lines)],
        );
        Some(note.trim().to_string())
    }
}

#[cfg(test)]
mod tests;
