//! Tool use, restored. When a session's history is rebuilt from the store
//! (`restore_chat_history_if_empty`: a fresh engine after a model change or a
//! fallback), its tool calls come back in compact form — the call, and a short
//! stub of its result — instead of being dropped. A history of plain replies
//! taught a fallback model that this agent answers without tools: it stopped
//! calling them, and told a story of its own while the app moved on.
//!
//! Only the most recent calls are kept (`KEEP_CALLS`), their results cut short
//! (`RESULT_CHARS`) and long argument strings too (`ARG_CHARS`), so the size
//! stays in check. A model with native tool calling gets them as it wrote them
//! (an assistant message with `tool_calls`, then one `tool` message per call);
//! one without gets the text form it uses live (the call's JSON, then
//! `Tool <name>: <result>`).

use crate::message::{ChatMessage, ToolCallFunction, ToolCallMessage};
use crate::state_fs::sessions::ChatMsg;
use serde_json::Value;

/// The most recent tool calls kept on restore; older ones are dropped.
pub(super) const KEEP_CALLS: usize = 8;
const RESULT_CHARS: usize = 300;
const ARG_CHARS: usize = 160;
const NO_RESULT: &str = "(result not kept)";

/// An observation row, as the restore reads it.
#[derive(Debug, PartialEq)]
pub(super) enum ToolRow {
    /// This agent called a tool (`{"type":"tool","tool":…,"args":…}`).
    Call { name: String, args: Value },
    /// A tool's result, handed to this agent (`Tool <name>: <rendered>`).
    Result { name: String, text: String },
}

/// What an observation row is: this agent's call, a result for it, or neither
/// (a loop marker, another agent's tool use).
pub(super) fn tool_row(m: &ChatMsg, own_agent: &str) -> Option<ToolRow> {
    if !m.is_observation {
        return None;
    }
    if m.from_id == own_agent {
        let v: Value = serde_json::from_str(&m.content).ok()?;
        if v.get("type")?.as_str()? != "tool" {
            return None;
        }
        let name = v.get("tool")?.as_str()?.to_string();
        let args = v.get("args").cloned().unwrap_or(Value::Null);
        return Some(ToolRow::Call { name, args });
    }
    if m.from_id == "system" && m.to_id == own_agent {
        let (name, text) = m.content.strip_prefix("Tool ")?.split_once(": ")?;
        return Some(ToolRow::Result {
            name: name.to_string(),
            text: text.to_string(),
        });
    }
    None
}

/// The index of the first call row kept: the `keep` most recent calls of this
/// agent's; every call before it is dropped.
pub(super) fn first_kept_call(msgs: &[ChatMsg], own_agent: &str, keep: usize) -> usize {
    if keep == 0 {
        return msgs.len();
    }
    let calls: Vec<usize> = msgs
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(tool_row(m, own_agent), Some(ToolRow::Call { .. })))
        .map(|(i, _)| i)
        .collect();
    calls.len().checked_sub(keep).map_or(0, |skip| calls[skip])
}

/// A run's calls in flight, and their results as they come in; written into
/// the history as one block when the run's next spoken row (or the end) comes.
pub(super) struct ToolReplay {
    native: bool,
    calls: Vec<(String, String, Value)>,
    results: Vec<Option<String>>,
    next_id: usize,
}

impl ToolReplay {
    pub(super) fn new(native: bool) -> Self {
        Self {
            native,
            calls: Vec::new(),
            results: Vec::new(),
            next_id: 0,
        }
    }

    /// A call. One made after results came in starts the run's next step.
    pub(super) fn call(&mut self, name: String, args: Value, out: &mut Vec<ChatMessage>) {
        if self.results.iter().any(Option::is_some) {
            self.flush(out);
        }
        self.next_id += 1;
        let id = format!("restored_{}", self.next_id);
        self.calls.push((id, name, cut_args(args)));
        self.results.push(None);
    }

    /// A result: it answers the first call of that name still waiting.
    pub(super) fn result(&mut self, name: &str, text: &str) {
        let waiting = self
            .calls
            .iter()
            .zip(&self.results)
            .position(|((_, n, _), r)| n == name && r.is_none());
        if let Some(i) = waiting {
            self.results[i] = Some(cut(text, RESULT_CHARS));
        }
    }

    /// The block, written into the history: calls, then one result each.
    pub(super) fn flush(&mut self, out: &mut Vec<ChatMessage>) {
        if self.calls.is_empty() {
            return;
        }
        let calls = std::mem::take(&mut self.calls);
        let results = std::mem::take(&mut self.results);
        let results = results
            .into_iter()
            .map(|r| r.unwrap_or_else(|| NO_RESULT.to_string()));
        if self.native {
            write_native(calls, results, out);
        } else {
            write_text(calls, results, out);
        }
    }
}

fn write_native(
    calls: Vec<(String, String, Value)>,
    results: impl Iterator<Item = String>,
    out: &mut Vec<ChatMessage>,
) {
    let answers: Vec<ChatMessage> = calls
        .iter()
        .zip(results)
        .map(|((id, name, _), text)| ChatMessage::tool_result_named(id.clone(), name.clone(), text))
        .collect();
    let calls = calls
        .into_iter()
        .map(|(id, name, arguments)| ToolCallMessage {
            id,
            call_type: "function".to_string(),
            function: ToolCallFunction { name, arguments },
            thought_signature: None,
        })
        .collect();
    out.push(ChatMessage::assistant_with_tool_calls(calls));
    out.extend(answers);
}

fn write_text(
    calls: Vec<(String, String, Value)>,
    results: impl Iterator<Item = String>,
    out: &mut Vec<ChatMessage>,
) {
    let said: Vec<String> = calls
        .iter()
        .map(|(_, name, args)| {
            serde_json::json!({"type": "tool", "tool": name, "args": args}).to_string()
        })
        .collect();
    let heard: Vec<String> = calls
        .iter()
        .zip(results)
        .map(|((_, name, _), text)| format!("Tool {name}: {text}"))
        .collect();
    out.push(ChatMessage::new("assistant", said.join("\n")));
    out.push(ChatMessage::new("user", heard.join("\n")));
}

/// A trimmed history never opens on a tool result whose call was cut away.
pub(super) fn drop_leading_results(history: &mut Vec<ChatMessage>) {
    let orphans = history.iter().take_while(|m| m.role == "tool").count();
    history.drain(..orphans);
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max).collect();
    s.push('…');
    s
}

pub(super) fn cut_args(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(cut(&s, ARG_CHARS)),
        Value::Array(xs) => Value::Array(xs.into_iter().map(cut_args).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, v)| (k, cut_args(v))).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(from: &str, to: &str, content: &str, obs: bool) -> ChatMsg {
        ChatMsg {
            via: None,
            via_note: None,
            agent_id: "ling".into(),
            from_id: from.into(),
            to_id: to.into(),
            content: content.into(),
            timestamp: 0,
            is_observation: obs,
            client_id: None,
            failed: false,
        }
    }
    fn call(name: &str) -> ChatMsg {
        row(
            "ling",
            "user",
            &format!(r#"{{"type":"tool","tool":"{name}","args":{{"said":"hi"}}}}"#),
            true,
        )
    }
    fn result(name: &str, text: &str) -> ChatMsg {
        row("system", "ling", &format!("Tool {name}: {text}"), true)
    }

    /// The rows of a session as the restore walks them (handler.rs), minus the
    /// agent lookups: spoken rows as they are, tool rows through the replay.
    fn restore(msgs: &[ChatMsg], native: bool, keep: usize) -> Vec<ChatMessage> {
        let first = first_kept_call(msgs, "ling", keep);
        let mut out = Vec::new();
        let mut replay = ToolReplay::new(native);
        for (i, m) in msgs.iter().enumerate() {
            if m.is_observation {
                match tool_row(m, "ling") {
                    Some(ToolRow::Call { name, args }) if i >= first => {
                        replay.call(name, args, &mut out)
                    }
                    Some(ToolRow::Result { name, text }) => replay.result(&name, &text),
                    _ => {}
                }
                continue;
            }
            replay.flush(&mut out);
            let role = if m.from_id == "user" {
                "user"
            } else {
                "assistant"
            };
            out.push(ChatMessage::new(role, m.content.clone()));
        }
        replay.flush(&mut out);
        out
    }

    #[test]
    fn a_restored_history_keeps_its_tool_calls_in_compact_form() {
        let long = "x".repeat(2000);
        let msgs = vec![
            row("user", "ling", "[scene] took 收下鸡蛋", false),
            row(
                "system",
                "ling",
                "Starting autonomous loop for task: …",
                true,
            ),
            call("Look"),
            result("Look", &long),
            row("ling", "user", "天亮前，阿禾……", false),
        ];
        let h = restore(&msgs, true, KEEP_CALLS);
        let roles: Vec<&str> = h.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["user", "assistant", "tool", "assistant"]);
        assert_eq!(h[1].tool_calls[0].function.name, "Look");
        assert_eq!(h[1].tool_calls[0].id, h[2].tool_call_id.clone().unwrap());
        assert_eq!(h[2].name.as_deref(), Some("Look"));
        assert!(
            h[2].content.chars().count() <= RESULT_CHARS + 1,
            "the result is cut short"
        );
        assert!(h[2].content.ends_with('…'));
        // without native tools: the text form the model uses live
        let t = restore(&msgs, false, KEEP_CALLS);
        assert_eq!(t[1].role, "assistant");
        assert!(
            t[1].content.contains(r#""tool":"Look""#) && t[1].content.contains(r#""type":"tool""#)
        );
        assert_eq!(t[2].role, "user");
        assert!(t[2].content.starts_with("Tool Look: xxx"));
    }

    #[test]
    fn parallel_calls_answer_in_order_and_a_lost_result_gets_a_stub() {
        let msgs = vec![
            row("user", "ling", "go", false),
            call("Look"),
            call("Resolve"),
            result("Resolve", "ok"),
            result("Look", "scene"),
            call("Show"),
            row("ling", "user", "done", false),
        ];
        let h = restore(&msgs, true, KEEP_CALLS);
        let roles: Vec<&str> = h.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(
            roles,
            [
                "user",
                "assistant",
                "tool",
                "tool",
                "assistant",
                "tool",
                "assistant"
            ]
        );
        assert_eq!(h[1].tool_calls.len(), 2);
        assert_eq!(
            (h[2].content.as_str(), h[3].content.as_str()),
            ("scene", "ok")
        );
        assert_eq!(
            h[5].content, NO_RESULT,
            "a call with no result kept still answers"
        );
    }

    #[test]
    fn only_the_most_recent_calls_come_back_and_others_rows_never_do() {
        let mut msgs = vec![row("user", "ling", "hi", false)];
        for i in 0..20 {
            msgs.push(call("Look"));
            msgs.push(result("Look", &format!("r{i}")));
        }
        msgs.push(row(
            "yinyue",
            "user",
            r#"{"type":"tool","tool":"Express","args":{}}"#,
            true,
        ));
        msgs.push(row("system", "yinyue", "Tool Express: ok", true));
        let h = restore(&msgs, true, 3);
        let answers: Vec<&str> = h
            .iter()
            .filter(|m| m.role == "tool")
            .map(|m| m.content.as_str())
            .collect();
        assert_eq!(answers, ["r17", "r18", "r19"]);
        assert!(h
            .iter()
            .all(|m| m.tool_calls.iter().all(|c| c.function.name == "Look")));
        assert_eq!(first_kept_call(&msgs, "ling", 0), msgs.len());
    }

    #[test]
    fn long_argument_strings_are_cut_and_orphaned_results_are_dropped() {
        let v = cut_args(serde_json::json!({"said": "y".repeat(500), "n": 3}));
        assert_eq!(v["said"].as_str().unwrap().chars().count(), ARG_CHARS + 1);
        assert_eq!(v["n"], 3);
        let mut h = vec![
            ChatMessage::tool_result_named("a", "Look", "x"),
            ChatMessage::new("assistant", "hi"),
        ];
        drop_leading_results(&mut h);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].role, "assistant");
    }
}
