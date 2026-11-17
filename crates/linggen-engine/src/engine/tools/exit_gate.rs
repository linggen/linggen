//! The one gate for tools that carry words out of the place a turn stands
//! in. From a place whose talk is kept to its app (`turn_place::sight`), a
//! call that would put what is said there in front of another place is
//! refused with a reason the model can read: a message to another agent
//! (`agent_chat`), a note (in front of its agent everywhere), and a memory
//! write that asks for a wider scope than the app's own (`global`, or a
//! folder or scope of its own choosing). Every tool call goes through
//! `Tools::execute`, and this is asked there first.

use super::{memory_mcp, ToolCall, Tools};
use serde_json::Value;

/// The fields of a memory write that move a row out of the scope the
/// engine stamps for the app (`memory_mcp::augment`).
const WIDER_SCOPE: &[&str] = &["cwd", "scope", "root", "scope_root"];

impl Tools {
    /// Why `call` may not run here, when it would carry the turn's words out
    /// of a place whose talk is kept.
    pub(crate) fn exit_refusal(&self, call: &ToolCall) -> Option<String> {
        let app = self.kept_here()?;
        let name = super::builtin::lookup(&call.tool).map_or(call.tool.as_str(), |t| t.name());
        leaves(name, &call.args).then(|| refusal(app, name))
    }
}

/// Whether a call of `tool` with `args` carries words out of its place.
fn leaves(tool: &str, args: &Value) -> bool {
    if tool == "agent_chat" || tool == crate::notes::WRITE_TOOL {
        return true;
    }
    memory_mcp::is_memory_tool(tool) && widens_memory(args)
}

/// A memory write that asks to be kept somewhere else than the app's scope.
fn widens_memory(args: &Value) -> bool {
    let writes = args.get("content").is_some() || args.get("replace_ids").is_some();
    let global = args.get("global").and_then(Value::as_bool) == Some(true);
    let elsewhere = WIDER_SCOPE
        .iter()
        .any(|f| args.get(*f).is_some_and(|v| !v.is_null()));
    writes && (global || elsewhere)
}

fn refusal(app: &str, tool: &str) -> String {
    format!(
        "refused: {app} keeps what is said in it to itself, and {tool} would carry it \
         out of {app}. Keep it here: answer in this chat, or write memory without \
         `global`, `cwd` or `scope`."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_message_a_note_and_a_wider_memory_write_leave_the_place() {
        assert!(leaves(
            "agent_chat",
            &json!({"to": "yinyue", "message": "hi"})
        ));
        assert!(leaves(crate::notes::WRITE_TOOL, &json!({"text": "x"})));
        assert!(widens_memory(&json!({"content": "x", "global": true})));
        assert!(widens_memory(&json!({"content": "x", "cwd": "/elsewhere"})));
        assert!(widens_memory(
            &json!({"content": "x", "scope": "/elsewhere"})
        ));
        // A write in the app's own scope, and any read, stay.
        assert!(!widens_memory(&json!({"content": "x"})));
        assert!(!widens_memory(&json!({"content": "x", "global": false})));
        assert!(!widens_memory(
            &json!({"query": "x", "scope": "/elsewhere"})
        ));
        assert!(!leaves("Read", &json!({"path": "a"})));
    }
}
