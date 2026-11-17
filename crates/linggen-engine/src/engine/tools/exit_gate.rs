//! The one gate for tools that carry words out of the place a turn stands
//! in. Every tool says, as a property of its own, whether a call keeps what
//! is said in the turn's place or carries it out to another
//! ([`PlaceReach`]): a built-in by `Tool::place_reach`, the memory server's
//! tools by [`memory_reach`]. From a place whose talk is kept to its app
//! (`turn_place::sight`), a call that carries words out is refused with a
//! reason the model can read. The person's own outside lookups (web search
//! and fetch, the browser) stay. Every tool call goes through
//! `Tools::execute`, and this is asked there first.

use super::{memory_mcp, ToolCall, Tools};
use serde_json::Value;

/// Whether a tool's call keeps what is said in the turn's place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceReach {
    /// It stays: read, act on the machine, look outside for the person.
    Stays,
    /// It puts words in front of another place: another agent, a note in
    /// front of its agent everywhere, another turn's question, another
    /// session, a queue every host reads.
    CarriesOut,
    /// A memory write: it stays in the app's own scope, and carries words
    /// out when it asks for a wider one (`global`, a folder or scope of its
    /// own choosing).
    ScopedWrite,
}

/// The fields of a memory write that move a row out of the scope the
/// engine stamps for the app (`memory_mcp::augment`).
const WIDER_SCOPE: &[&str] = &["cwd", "scope", "root", "scope_root"];

/// The memory server's tools, each classed once. A tool it lists that is
/// not here carries words out until it is looked at (`tests`).
const MEMORY_REACH: &[(&str, PlaceReach)] = &[
    ("memory_search", PlaceReach::Stays),
    ("memory_list", PlaceReach::Stays),
    ("memory_get", PlaceReach::Stays),
    ("memory_session_start", PlaceReach::Stays),
    ("memory_days", PlaceReach::Stays),
    ("memory_issues", PlaceReach::Stays),
    ("memory_chains", PlaceReach::Stays),
    ("memory_role", PlaceReach::Stays),
    ("memory_delete", PlaceReach::Stays),
    ("memory_add", PlaceReach::ScopedWrite),
    ("memory_update", PlaceReach::ScopedWrite),
    ("memory_issue_add", PlaceReach::CarriesOut),
    ("memory_issue_resolve", PlaceReach::CarriesOut),
    ("memory_remember_day", PlaceReach::CarriesOut),
    ("memory_harvest_day", PlaceReach::CarriesOut),
    ("memory_dream_lease", PlaceReach::CarriesOut),
    ("memory_sweep", PlaceReach::CarriesOut),
];

/// The class of the memory server's tool `name` (its own name, unqualified).
fn memory_reach(name: &str) -> PlaceReach {
    MEMORY_REACH
        .iter()
        .find(|(n, _)| *n == name)
        .map_or(PlaceReach::CarriesOut, |(_, reach)| *reach)
}

impl Tools {
    /// Why `call` may not run here, when it would carry the turn's words out
    /// of a place whose talk is kept.
    pub(crate) fn exit_refusal(&self, call: &ToolCall) -> Option<String> {
        let app = self.kept_here()?;
        leaves(reach_of(&call.tool), &call.args).then(|| refusal(app, &call.tool))
    }
}

/// The class of the tool a call names. A tool of another configured
/// server is the person's own and stays.
fn reach_of(tool: &str) -> PlaceReach {
    if let Some(builtin) = super::builtin::lookup(tool) {
        return builtin.place_reach();
    }
    if memory_mcp::is_memory_tool(tool) {
        return memory_reach(tool.rsplit("__").next().unwrap_or(tool));
    }
    PlaceReach::Stays
}

/// Whether a call of this class with `args` carries words out.
fn leaves(reach: PlaceReach, args: &Value) -> bool {
    match reach {
        PlaceReach::Stays => false,
        PlaceReach::CarriesOut => true,
        PlaceReach::ScopedWrite => widens_memory(args),
    }
}

/// A memory write that asks to be kept somewhere else than the app's scope.
fn widens_memory(args: &Value) -> bool {
    let global = args.get("global").and_then(Value::as_bool) == Some(true);
    let elsewhere = WIDER_SCOPE
        .iter()
        .any(|f| args.get(*f).is_some_and(|v| !v.is_null()));
    global || elsewhere
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

    /// Every tool the memory server lists is classed here.
    #[test]
    fn every_memory_tool_is_classed() {
        for name in ling_mem::http::tool_names() {
            assert!(
                MEMORY_REACH.iter().any(|(n, _)| *n == name),
                "{name} has no class: is it a way out of the place?"
            );
        }
    }

    #[test]
    fn what_carries_words_out_is_refused_and_the_rest_stays() {
        for tool in ["agent_chat", crate::notes::WRITE_TOOL, "answer_prompt", "StartClaudeSession"] {
            assert!(leaves(reach_of(tool), &json!({})), "{tool}");
        }
        for tool in ["Read", "Bash", "WebSearch", "WebFetch", crate::notes::DONE_TOOL] {
            assert!(!leaves(reach_of(tool), &json!({})), "{tool}");
        }
        assert!(leaves(memory_reach("memory_issue_add"), &json!({"text": "x"})));
        assert!(leaves(memory_reach("memory_add"), &json!({"content": "x", "global": true})));
        assert!(leaves(memory_reach("memory_add"), &json!({"content": "x", "cwd": "/elsewhere"})));
        assert!(!leaves(memory_reach("memory_add"), &json!({"content": "x"})));
        assert!(!leaves(memory_reach("memory_search"), &json!({"query": "x", "scope": "/elsewhere"})));
        assert!(leaves(memory_reach("memory_not_yet_classed"), &json!({})), "unknown fails closed");
    }
}
