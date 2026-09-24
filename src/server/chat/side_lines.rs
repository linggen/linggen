//! Side lines — what was said in a session outside its agent's own turns.
//!
//! A session's agent (Ling, in an app's chat) keeps its thread in memory
//! between turns. When someone else speaks in that session meanwhile — the
//! companion answering an `@` addressed to her, or her line on an app moment —
//! the row lands on disk, but the live thread never sees it. These lines are
//! held here and handed to the agent at the start of its next turn, so the
//! exchange is part of what it knows. A thread rebuilt from disk already has
//! them, and drops them.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// The most lines held per session; the oldest go first.
const MAX_LINES: usize = 12;

#[derive(Clone, Debug, PartialEq)]
struct SideLine {
    /// The agent whose exchange this is — never handed back to that agent.
    thread: String,
    line: String,
}

static LINES: LazyLock<Mutex<HashMap<String, Vec<SideLine>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Hold `line` (already labeled for the model) for the session's other agents.
pub(crate) fn note(session_id: &str, thread: &str, line: String) {
    let mut all = LINES.lock().unwrap_or_else(|e| e.into_inner());
    let held = all.entry(session_id.to_string()).or_default();
    held.push(SideLine {
        thread: thread.to_string(),
        line,
    });
    if held.len() > MAX_LINES {
        held.drain(..held.len() - MAX_LINES);
    }
}

/// The lines `agent_id` has not seen, oldest first; they are handed over once.
pub(super) fn take_for(session_id: &str, agent_id: &str) -> Vec<String> {
    let mut all = LINES.lock().unwrap_or_else(|e| e.into_inner());
    let Some(held) = all.get_mut(session_id) else {
        return Vec::new();
    };
    let (theirs, keep): (Vec<SideLine>, Vec<SideLine>) =
        held.drain(..).partition(|l| l.thread != agent_id);
    *held = keep;
    if held.is_empty() {
        all.remove(session_id);
    }
    theirs.into_iter().map(|l| l.line).collect()
}

/// The one message the agent reads them as, or `None` when there are none.
pub(super) fn as_message(lines: &[String]) -> Option<String> {
    if lines.is_empty() {
        return None;
    }
    Some(format!(
        "[Said in this chat since your last turn]\n{}",
        lines.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_go_to_the_other_agent_once_and_never_back_to_their_own() {
        note("s-side-1", "yinyue", "@银月 how am I doing?".into());
        note(
            "s-side-1",
            "yinyue",
            "[Yinyue]: Steady. Rest before the pass.".into(),
        );
        assert!(
            take_for("s-side-1", "yinyue").is_empty(),
            "her own exchange"
        );
        let got = take_for("s-side-1", "ling");
        assert_eq!(got.len(), 2);
        assert!(take_for("s-side-1", "ling").is_empty(), "handed over once");
        let msg = as_message(&got).unwrap();
        assert!(msg.find("@银月").unwrap() < msg.find("[Yinyue]").unwrap());
        assert_eq!(as_message(&[]), None);
    }

    #[test]
    fn only_the_newest_lines_are_held() {
        for i in 0..(MAX_LINES + 5) {
            note("s-side-2", "yinyue", format!("line {i}"));
        }
        let got = take_for("s-side-2", "ling");
        assert_eq!(got.len(), MAX_LINES);
        assert_eq!(got.last().unwrap(), &format!("line {}", MAX_LINES + 4));
    }
}
