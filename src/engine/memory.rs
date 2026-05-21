//! Memory-related per-turn cadence helpers.
//!
//! Built-in core memory (the `tier=core` rows the engine inlines into
//! every owner session) lives in `core_memory.rs`. The rest of the
//! memory store is dispatched through `capability_tools::dispatch`
//! after being registered from the memory skill's `SKILL.md` `tools:`
//! block.
//!
//! The canonical Memory protocol (read-before-write, AskUser on
//! contradiction, tier selection by confidence) is a single TOML block
//! `[memory_protocol]` in `prompts/system-prompt.toml`, injected into
//! every memory-enabled session by `prompt.rs`. There is no separate
//! per-turn "nudge message" any more — the system prompt + the N-turn
//! encoder subagent cover the same ground without a third redundant
//! layer.
//!
//! What lives here: the `should_consolidate` cadence helper used by
//! `server::chat::consolidation` to decide when to spawn the encoder.

use crate::message::ChatMessage;

/// Returns `true` when the every-N-turns memory consolidation subagent
/// should fire after this turn. Cadence is derived from the
/// user-message count over the session's chat history (restart-safe,
/// per-session by construction). `interval == 0` disables (config
/// rejects 0, but the guard keeps tests and any future opt-out honest).
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

#[cfg(test)]
mod tests {
    use super::*;

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
