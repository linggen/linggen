//! A run that ends on an error keeps its work in the engine's thread.
//!
//! The loop keeps its tool calls and their results in its own messages and
//! leaves the thread only its last words — which a run that failed (its
//! model refused, its stream timed out) never wrote. Its session's file
//! keeps each call, but a result there is what the chat shows (a read's
//! content is not in it). So the run's own messages after its opening —
//! every call and the result the model saw — go into the thread, and the
//! turn that continues the session (a person's retry, a host's
//! `agent_run(session_id)`) starts from what the run had read.

use super::AgentEngine;
use crate::message::ChatMessage;

impl AgentEngine {
    /// Keep `messages` after `opening` (the run's system prompt, thread and
    /// task) in the thread. Nothing is kept when the run compacted below its
    /// opening; a last call whose results never came back is left out — a
    /// provider refuses a call with no result.
    pub(crate) fn keep_unfinished_work(&mut self, messages: &[ChatMessage], opening: usize) {
        self.chat_history.extend(unfinished_work(messages, opening));
    }
}

fn unfinished_work(messages: &[ChatMessage], opening: usize) -> Vec<ChatMessage> {
    let Some(work) = messages.get(opening..) else {
        return Vec::new();
    };
    let mut work = work.to_vec();
    while work.last().is_some_and(|m| !m.tool_calls.is_empty()) {
        work.pop();
    }
    work
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ToolCallFunction, ToolCallMessage};

    fn call(id: &str) -> ChatMessage {
        ChatMessage::assistant_with_tool_calls(vec![ToolCallMessage {
            id: id.into(),
            call_type: "function".into(),
            function: ToolCallFunction {
                name: "Read".into(),
                arguments: serde_json::json!({}),
            },
            thought_signature: None,
        }])
    }

    fn result(id: &str, text: &str) -> ChatMessage {
        let mut m = ChatMessage::new("tool", text.to_string());
        m.tool_call_id = Some(id.into());
        m
    }

    /// The work after the opening is kept whole; the opening is not kept
    /// twice; a dangling last call goes; a compacted run keeps nothing.
    #[test]
    fn a_failed_runs_reading_is_kept_after_its_opening() {
        let opening = vec![ChatMessage::new("system", "s".to_string()), ChatMessage::new("user", "the task".to_string())];
        let mut messages = opening.clone();
        messages.extend([call("a"), result("a", "the file's words"), call("b")]);
        let kept = unfinished_work(&messages, opening.len());
        let roles: Vec<&str> = kept.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["assistant", "tool"]);
        assert_eq!(kept[1].content, "the file's words");
        assert!(unfinished_work(&opening[..1], opening.len()).is_empty(), "compacted below the opening");
        assert!(unfinished_work(&opening, opening.len()).is_empty(), "nothing done, nothing kept");
    }
}
