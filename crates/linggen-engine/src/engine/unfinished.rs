//! A run that cannot go on keeps its work in the engine's thread.
//!
//! The loop keeps its tool calls and their results in its own messages and
//! leaves the thread only its last words — which a run whose model call
//! failed (refused, a usage limit, a stream that timed out) never wrote. Its
//! session's file keeps each call, but a result there is what the chat shows
//! (a read's content is not in it). So such a run's own messages after its
//! opening — every call and the result the model saw — go into the thread,
//! and the turn that continues the session (a host's
//! `agent_run(session_id)`, a person's retry) starts from what the run had
//! read. The thread lives in the session's engine: a restart or a settings
//! change rebuilds it from the file, without the results.
//!
//! Kept only for a run whose failure was noted as a reason it cannot go on
//! (`AgentEngine::stream_with_fallback`) — never a Stop or another error —
//! and only when the thread can take it: the run did not compact, and every
//! call kept has its results.
//!
//! What is kept is exactly what the thread does not have yet: a run that
//! went on past a reply (a kickoff's next item) already wrote that reply
//! and the next item into the thread, so those rows take their place among
//! the run's own, in the order they happened, and appear once.

use super::AgentEngine;
use crate::message::ChatMessage;

/// Where a run's own messages begin: after its system prompt, its thread
/// and its task; and how long the thread was then.
pub(crate) struct Opening {
    len: usize,
    last: Option<String>,
    thread: usize,
}

impl Opening {
    pub(crate) fn of(messages: &[ChatMessage], thread: usize) -> Self {
        Self {
            len: messages.len(),
            last: messages.last().map(|m| m.content.clone()),
            thread,
        }
    }
}

impl AgentEngine {
    /// Keep a run's work after `opening` in the thread, when the run cannot
    /// go on, and note on the run that it is kept.
    pub(crate) fn keep_unfinished_work(&mut self, messages: &[ChatMessage], opening: &Opening) {
        let (Some(manager), Some(run_id)) = (self.tools.get_manager(), self.run_id.clone()) else {
            return;
        };
        if manager.run_store.cannot_run(&run_id).is_none() || self.reload_thread {
            return;
        }
        if self.chat_history.len() < opening.thread {
            return;
        }
        let work = unfinished_work(messages, opening);
        if work.is_empty() {
            return;
        }
        let written = self.chat_history.split_off(opening.thread);
        self.chat_history.extend(woven(work, written));
        manager.run_store.note_work_kept(&run_id);
    }
}

/// The run's work with the rows the run already wrote into the thread
/// (`written`, in order) taken as the same rows of the work, so each is
/// there once; a written row the work does not carry follows it.
fn woven(work: Vec<ChatMessage>, written: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let mut from = 0;
    let mut extra = Vec::new();
    for row in written {
        match work[from..].iter().position(|w| same_row(w, &row)) {
            Some(at) => from += at + 1,
            None => extra.push(row),
        }
    }
    let mut kept = work;
    kept.extend(extra);
    kept
}

/// Two rows that say the same thing in the same voice, neither a call.
fn same_row(a: &ChatMessage, b: &ChatMessage) -> bool {
    a.role == b.role && a.content == b.content && a.tool_calls.is_empty() && b.tool_calls.is_empty()
}

/// The run's messages after `opening`, cut before the first call whose
/// results are not all there. Nothing when the opening is no longer where
/// it was (the run compacted).
fn unfinished_work(messages: &[ChatMessage], opening: &Opening) -> Vec<ChatMessage> {
    let intact = opening
        .len
        .checked_sub(1)
        .and_then(|i| messages.get(i))
        .map(|m| Some(&m.content) == opening.last.as_ref())
        .unwrap_or(opening.len == 0);
    let Some(work) = messages.get(opening.len..).filter(|_| intact) else {
        return Vec::new();
    };
    let start = work
        .iter()
        .position(|m| m.role != "tool")
        .unwrap_or(work.len());
    let work = &work[start..];
    let end = work
        .iter()
        .enumerate()
        .find(|(i, m)| !answered(m, &work[i + 1..]))
        .map_or(work.len(), |(i, _)| i);
    work[..end].to_vec()
}

/// Every call `m` makes has its result among the messages after it.
fn answered(m: &ChatMessage, after: &[ChatMessage]) -> bool {
    m.tool_calls.iter().all(|call| {
        after
            .iter()
            .any(|r| r.role == "tool" && r.tool_call_id.as_deref() == Some(call.id.as_str()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ToolCallFunction, ToolCallMessage};

    fn calls(ids: &[&str]) -> ChatMessage {
        ChatMessage::assistant_with_tool_calls(
            ids.iter()
                .map(|id| ToolCallMessage {
                    id: id.to_string(),
                    call_type: "function".into(),
                    function: ToolCallFunction {
                        name: "Read".into(),
                        arguments: serde_json::json!({}),
                    },
                    thought_signature: None,
                })
                .collect(),
        )
    }

    fn result(id: &str, text: &str) -> ChatMessage {
        let mut m = ChatMessage::new("tool", text.to_string());
        m.tool_call_id = Some(id.into());
        m
    }

    fn opened() -> (Vec<ChatMessage>, Opening) {
        let messages = vec![
            ChatMessage::new("system", "s".to_string()),
            ChatMessage::new("user", "the task".to_string()),
        ];
        let opening = Opening::of(&messages, 0);
        (messages, opening)
    }

    /// The work after the opening is kept whole; the opening is not kept
    /// twice; a call without all its results, and what follows it, goes.
    #[test]
    fn a_failed_runs_reading_is_kept_after_its_opening() {
        let (mut messages, opening) = opened();
        messages.extend([
            calls(&["a"]),
            result("a", "the file's words"),
            calls(&["b"]),
        ]);
        let kept = unfinished_work(&messages, &opening);
        let roles: Vec<&str> = kept.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["assistant", "tool"]);
        assert_eq!(kept[1].content, "the file's words");

        let (mut batch, opening) = opened();
        batch.extend([
            calls(&["a"]),
            result("a", "x"),
            calls(&["b", "c"]),
            result("b", "y"),
        ]);
        let kept = unfinished_work(&batch, &opening);
        assert_eq!(
            kept.len(),
            2,
            "a batch cut before all its results is left out"
        );
        let (bare, opening) = opened();
        assert!(
            unfinished_work(&bare, &opening).is_empty(),
            "nothing done, nothing kept"
        );
    }

    /// A run that went past a reply wrote that reply and the next item into
    /// the thread already: woven into the work, each row is there once, in
    /// the order it happened; a row the work never carried follows it.
    #[test]
    fn rows_the_thread_has_are_not_kept_twice() {
        let (mut messages, opening) = opened();
        messages.extend([
            calls(&["a"]),
            result("a", "x"),
            ChatMessage::new("assistant", "first done".to_string()),
            ChatMessage::new("user", "the next item".to_string()),
            calls(&["b"]),
            result("b", "y"),
        ]);
        let written = vec![
            ChatMessage::new("assistant", "first done".to_string()),
            ChatMessage::new("user", "the next item".to_string()),
            ChatMessage::new("user", "a note the run never saw".to_string()),
        ];
        let kept = woven(unfinished_work(&messages, &opening), written);
        let said: Vec<&str> = kept.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(
            said,
            ["", "x", "first done", "the next item", "", "y", "a note the run never saw"]
        );
    }

    /// A run that compacted rewrote its messages: the opening no longer
    /// stands where it was, and nothing is kept.
    #[test]
    fn a_compacted_run_keeps_nothing() {
        let (_, opening) = opened();
        let compacted = vec![
            ChatMessage::new("system", "s".to_string()),
            ChatMessage::new("user", "summary of what came before".to_string()),
            result("a", "an orphan result"),
            calls(&["b"]),
            result("b", "y"),
        ];
        assert!(unfinished_work(&compacted, &opening).is_empty());
        assert!(
            unfinished_work(&compacted[..1], &opening).is_empty(),
            "shorter than the opening"
        );
    }
}
