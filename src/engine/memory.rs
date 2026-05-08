//! Mid-session self-review nudge.
//!
//! Built-in core memory (identity + style) lives in `core_memory.rs`.
//! Skill memory (facts, activity, semantic retrieval) is dispatched
//! through `capability_tools::dispatch` after being registered from the
//! memory skill's `SKILL.md` `tools:` block — there is no memory-specific
//! dispatch code in the engine anymore.
//!
//! What lives here: the periodic nudge that asks the model whether the
//! recent exchange produced anything worth saving.

use crate::message::ChatMessage;

// ── Mid-session self-review nudge ────────────────────────────────────────────

/// Returns `true` when the mid-session memory-check nudge should fire for
/// this turn. Fires every `interval` user messages; `interval == 0` disables.
pub(crate) fn should_fire_nudge(chat_history: &[ChatMessage], interval: usize) -> bool {
    if interval == 0 {
        return false;
    }
    let user_count = chat_history.iter().filter(|m| m.role == "user").count();
    user_count > 0 && user_count % interval == 0
}

/// The synthetic user message that nudges the model to check whether the
/// recent exchange produced anything worth saving to, or contradicts
/// something already in, memory.
pub(crate) fn nudge_message() -> ChatMessage {
    ChatMessage::new(
        "user",
        "[MEMORY CHECK — hidden reminder, not from the user] \
         Did the last few exchanges produce anything durable — an \
         identity fact, a cross-project preference, or a fact only the \
         user can supply? Or did the user contradict something already \
         in memory? If yes, act now: universals about the person → \
         Edit `~/.linggen/memory/identity.md` or `style.md` (tiny, \
         high-bar); cross-project user intent / decision / preference / \
         learning → `Memory_write({verb: \"add\", ...})` when a memory \
         provider is installed. Append, don't overwrite — if a row \
         contradicts a new fact, add the new one and let live retrieval \
         reconcile next time. **Do NOT write to project files** \
         (`<project>/AGENTS.md`, `CLAUDE.md`, source, docs); those are \
         user-curated. Project-internal implementation detail is not \
         memory — drop it; the agent reads the source next time. If \
         nothing durable, reply briefly with `(no memory changes)` and \
         continue with the user's current request."
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nudge_disabled_when_interval_is_zero() {
        let history = vec![
            ChatMessage::new("user", "one"),
            ChatMessage::new("user", "two"),
        ];
        assert!(!should_fire_nudge(&history, 0));
    }

    #[test]
    fn nudge_fires_at_multiples_of_interval() {
        let mut history = Vec::new();
        for i in 1..=12 {
            history.push(ChatMessage::new("user", format!("msg {i}")));
            let expected = i % 6 == 0;
            assert_eq!(
                should_fire_nudge(&history, 6),
                expected,
                "user_count={i} interval=6"
            );
        }
    }

    #[test]
    fn nudge_ignores_non_user_roles() {
        let history = vec![
            ChatMessage::new("assistant", "a"),
            ChatMessage::new("system", "s"),
            ChatMessage::new("user", "u1"),
            ChatMessage::new("tool", "t"),
        ];
        // 1 user message → does not hit interval=6.
        assert!(!should_fire_nudge(&history, 6));
    }
}
