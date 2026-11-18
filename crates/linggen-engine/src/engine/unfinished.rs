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

use super::AgentEngine;
use crate::message::ChatMessage;

/// Where a run's own messages begin: after its system prompt, its thread
/// and its task.
pub(crate) struct Opening {
    len: usize,
    last: Option<String>,
}

impl Opening {
    pub(crate) fn of(messages: &[ChatMessage]) -> Self {
        Self {
            len: messages.len(),
            last: messages.last().map(|m| m.content.clone()),
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
        let work = unfinished_work(messages, opening);
        if work.is_empty() {
            return;
        }
        self.chat_history.extend(work);
        manager.run_store.note_work_kept(&run_id);
    }
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
    let start = work.iter().position(|m| m.role != "tool").unwrap_or(work.len());
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
        let opening = Opening::of(&messages);
        (messages, opening)
    }

    /// The work after the opening is kept whole; the opening is not kept
    /// twice; a call without all its results, and what follows it, goes.
    #[test]
    fn a_failed_runs_reading_is_kept_after_its_opening() {
        let (mut messages, opening) = opened();
        messages.extend([calls(&["a"]), result("a", "the file's words"), calls(&["b"])]);
        let kept = unfinished_work(&messages, &opening);
        let roles: Vec<&str> = kept.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["assistant", "tool"]);
        assert_eq!(kept[1].content, "the file's words");

        let (mut batch, opening) = opened();
        batch.extend([calls(&["a"]), result("a", "x"), calls(&["b", "c"]), result("b", "y")]);
        let kept = unfinished_work(&batch, &opening);
        assert_eq!(kept.len(), 2, "a batch cut before all its results is left out");
        let (bare, opening) = opened();
        assert!(unfinished_work(&bare, &opening).is_empty(), "nothing done, nothing kept");
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
        assert!(unfinished_work(&compacted[..1], &opening).is_empty(), "shorter than the opening");
    }
}
