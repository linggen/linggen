//! Mid-session self-review nudge.
//!
//! Built-in core memory (the `tier=core` rows the engine inlines into
//! every owner session) lives in `core_memory.rs`. The rest of the
//! semantic store is dispatched through `capability_tools::dispatch`
//! after being registered from the memory skill's `SKILL.md` `tools:`
//! block — there is no memory-specific dispatch code in the engine
//! anymore.
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

/// Returns `true` when the every-N-turns memory consolidation subagent
/// should fire after this turn. Mirrors [`should_fire_nudge`]'s counting
/// (user-message count over the session's chat history) so the cadence is
/// derived, not a persisted counter — restart-safe and per-session by
/// construction. `interval == 0` disables (config rejects 0, but the guard
/// keeps tests and any future opt-out honest).
///
/// `count % interval == 0` with `count > 0` means the first fire lands at
/// exactly turn N: sessions shorter than N turns are never consolidated
/// (spec §2), and since this is only ever checked *after* a turn completes
/// there is no startup trigger.
pub(crate) fn should_consolidate(chat_history: &[ChatMessage], interval: usize) -> bool {
    if interval == 0 {
        return false;
    }
    let turn_count = chat_history.iter().filter(|m| m.role == "user").count();
    turn_count > 0 && turn_count % interval == 0
}

/// The synthetic user message that nudges the model to check whether the
/// recent exchange produced anything worth saving to, or contradicts
/// something already in, memory. Carries the live (user-present) half of
/// the Reconcile contract (`memory-spec.md` §2): read-before-write +
/// reconcile-on-contradiction.
pub(crate) fn nudge_message() -> ChatMessage {
    ChatMessage::new(
        "user",
        "[MEMORY CHECK — hidden reminder, not from the user] \
         Did the last few exchanges produce anything durable — an \
         identity fact, a cross-project preference, a decision, or a \
         fact only the user can supply? Anything the user *explicitly* \
         asked you to remember, a standing instruction, or a stated \
         preference you must have written the moment it was said — this \
         check only backstops the *incidental* signal you didn't \
         capture live. \
         \
         Read before you write: `Memory_query` first. If an equivalent \
         row already exists, don't duplicate it. If an existing row \
         **contradicts** the new fact on the same subject (opposite \
         value — e.g. \"cat is male\" vs \"cat is female\", not merely \
         similar) and it matters here, **ask the user** which is right, \
         showing both rows with their dates; write their answer as a new \
         timestamped row — never silently overwrite. (Delete the wrong \
         row only if the user explicitly asks.) Unsure, immaterial, or \
         no real conflict → just append the new row; later recall \
         reconciles by recency. \
         \
         Reconcile on recall: if memory you were given this turn holds \
         rows that conflict on something the user's request depends on, \
         ask them (with dates) before relying on it. You may merge rows \
         *in your reply*; never write a synthesized/merged row to the \
         store, and never delete to \"tidy\" — that is user-only. \
         \
         Where: stable universals about the person (name, role, hard \
         work rules) → `Memory_write({verb: \"add\", tier: \"core\", \
         ...})` — `tier=core` rows are injected into every session, so \
         keep that bar high. Cross-project intent / decision / \
         preference / learning → `Memory_write({verb: \"add\", ...})` \
         (semantic tier, the default). **Never** write project files \
         (`<project>/AGENTS.md`, `CLAUDE.md`, source, docs) or store \
         file-derivable detail — the agent reads the source next time. \
         If nothing durable and nothing to reconcile, reply briefly \
         with `(no memory changes)` and continue."
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

    #[test]
    fn consolidate_disabled_when_interval_is_zero() {
        let history = vec![
            ChatMessage::new("user", "one"),
            ChatMessage::new("user", "two"),
        ];
        assert!(!should_consolidate(&history, 0));
    }

    #[test]
    fn consolidate_never_fires_for_sub_n_session() {
        // A session shorter than the interval is never consolidated.
        let mut history = Vec::new();
        for i in 1..10 {
            history.push(ChatMessage::new("user", format!("msg {i}")));
            assert!(
                !should_consolidate(&history, 10),
                "turn {i} < 10 must not fire"
            );
        }
    }

    #[test]
    fn consolidate_fires_at_multiples_of_interval() {
        let mut history = Vec::new();
        for i in 1..=30 {
            history.push(ChatMessage::new("user", format!("msg {i}")));
            assert_eq!(
                should_consolidate(&history, 10),
                i % 10 == 0,
                "turn_count={i} interval=10"
            );
        }
    }
}
