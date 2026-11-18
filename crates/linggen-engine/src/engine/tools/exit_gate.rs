//! The one gate for tools that carry words out of the place a turn stands
//! in. Every tool says, as a property of its own, whether a call keeps what
//! is said in the turn's place or carries it out to another
//! ([`PlaceReach`]): a built-in by `Tool::place_reach`, a call of the
//! memory server by its operation ([`memory_reach`]). From a private app's turn
//! (`TurnPlace::private_app`) a call that carries words out is refused with
//! a reason the model can read. The person's own outside lookups (web
//! search and fetch, the browser), Bash and other configured servers are
//! the skill's own `allowed-tools`' to govern, not this gate's. Every tool
//! call goes through `Tools::execute`, and this is asked there first.

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
    /// It reaches the app its call names in this field (`AppTool`'s `app`,
    /// `RunApp`'s `skill`): it stays when that is the turn's own app, and
    /// carries words out into any other.
    IntoApp(&'static str),
}

/// The fields of a memory write that move a row out of the scope the
/// engine stamps for the app (`memory_mcp::augment`).
const WIDER_SCOPE: &[&str] = &["cwd", "scope", "root", "scope_root"];

/// Memory's operations, each classed once by the verb its branch runs
/// (`ling_mem::catalog::memory`): a call by the branch its `op` names. An
/// operation that is not here carries words out until it is looked at
/// (`tests`).
const MEMORY_REACH: &[(&str, PlaceReach)] = &[
    ("search", PlaceReach::Stays),
    ("list", PlaceReach::Stays),
    ("get", PlaceReach::Stays),
    ("session_start", PlaceReach::Stays),
    ("days", PlaceReach::Stays),
    ("issues", PlaceReach::Stays),
    ("chains", PlaceReach::Stays),
    ("role", PlaceReach::Stays),
    ("delete", PlaceReach::Stays),
    ("add", PlaceReach::ScopedWrite),
    ("update", PlaceReach::ScopedWrite),
    ("issue_add", PlaceReach::CarriesOut),
    ("issue_resolve", PlaceReach::CarriesOut),
    ("remember_day", PlaceReach::CarriesOut),
    ("harvest_day", PlaceReach::CarriesOut),
    ("dream_lease", PlaceReach::CarriesOut),
    ("sweep", PlaceReach::CarriesOut),
];

/// The class of a call of the memory server's tool `name` (its own name,
/// unqualified) with `args`: its branch's. A call that names no branch
/// (the server refuses it) is its tool's class when every branch of the
/// tool has the same one, else it fails closed; a name that is no tool (a
/// retired one included) fails closed.
fn memory_reach(name: &str, args: &Value) -> PlaceReach {
    use ling_mem::catalog::View;
    let cat = ling_mem::catalog::memory::catalog();
    if !cat.tools().iter().any(|t| t.name == name) {
        return PlaceReach::CarriesOut;
    }
    if let Some(branch) = cat.branch_of(View::Local, name, args) {
        return verb_reach(branch.dispatch);
    }
    let reaches: Vec<PlaceReach> = cat.visible(View::Local, name).iter().map(|b| verb_reach(b.dispatch)).collect();
    match reaches.split_first() {
        Some((first, rest)) if rest.iter().all(|r| r == first) => *first,
        _ => PlaceReach::CarriesOut,
    }
}

fn verb_reach(verb: &str) -> PlaceReach {
    MEMORY_REACH
        .iter()
        .find(|(v, _)| *v == verb)
        .map_or(PlaceReach::CarriesOut, |(_, reach)| *reach)
}

impl Tools {
    /// Why `call` may not run here, when it would carry a private app's
    /// words out of it.
    pub(crate) fn exit_refusal(&self, call: &ToolCall) -> Option<String> {
        let app = self.private_app.as_deref()?;
        leaves(reach_of(&call.tool, &call.args), &call.args, app).then(|| refusal(app, &call.tool))
    }
}

/// The class of the tool a call names. A tool of another configured
/// server is the person's own and stays.
fn reach_of(tool: &str, args: &Value) -> PlaceReach {
    if let Some(builtin) = super::builtin::lookup(tool) {
        return builtin.place_reach();
    }
    if memory_mcp::is_memory_tool(tool) {
        return memory_reach(tool.rsplit("__").next().unwrap_or(tool), args);
    }
    PlaceReach::Stays
}

/// Whether a call of this class with `args`, made in `own` app's turn,
/// carries words out.
fn leaves(reach: PlaceReach, args: &Value, own: &str) -> bool {
    match reach {
        PlaceReach::Stays => false,
        PlaceReach::CarriesOut => true,
        PlaceReach::ScopedWrite => widens_memory(args),
        PlaceReach::IntoApp(field) => args.get(field).and_then(Value::as_str).is_some_and(|app| app != own),
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

    /// Every operation of memory's catalogue is classed here.
    #[test]
    fn every_memory_operation_is_classed() {
        for branch in ling_mem::catalog::memory::catalog().branches() {
            assert!(
                MEMORY_REACH.iter().any(|(v, _)| *v == branch.dispatch),
                "{} has no class: is it a way out of the place?",
                branch.dispatch
            );
        }
    }

    fn upkeep_without_op() -> PlaceReach {
        memory_reach("memory_upkeep", &json!({"date": "2026-11-01"}))
    }

    /// A folded call is classed by its op: reading the day's state stays,
    /// stamping or sweeping it carries words out.
    #[test]
    fn a_folded_call_is_classed_by_its_op() {
        let upkeep = |op: &str| memory_reach("memory_upkeep", &json!({"op": op}));
        for op in ["days", "issues", "chains", "role", "session_start"] {
            assert_eq!(upkeep(op), PlaceReach::Stays, "{op}");
        }
        for op in ["sweep", "remember_day", "harvest_day", "dream_lease", "issue_add", "issue_resolve"] {
            assert_eq!(upkeep(op), PlaceReach::CarriesOut, "{op}");
        }
        assert_eq!(upkeep("dream_run"), PlaceReach::CarriesOut, "not memory's: fails closed");
        let search = |args: Value| memory_reach("memory_search", &args);
        assert_eq!(search(json!({"query": "q"})), PlaceReach::Stays);
        assert_eq!(search(json!({"op": "get", "id": "a"})), PlaceReach::Stays);
        assert_eq!(search(json!({"id": "a"})), PlaceReach::Stays, "no branch, but every search stays");
        assert_eq!(upkeep_without_op(), PlaceReach::CarriesOut, "no branch, mixed classes: fails closed");
        assert_eq!(memory_reach("memory_update", &json!({"id": "a"})), PlaceReach::ScopedWrite);
    }

    #[test]
    fn what_carries_words_out_is_refused_and_the_rest_stays() {
        let leaves = |reach, args: &Value| leaves(reach, args, "ledger");
        for tool in ["agent_chat", crate::notes::WRITE_TOOL, "answer_prompt", "StartClaudeSession"] {
            assert!(leaves(reach_of(tool, &json!({})), &json!({})), "{tool}");
        }
        for tool in ["Read", "Bash", "WebSearch", "WebFetch", crate::notes::DONE_TOOL] {
            assert!(!leaves(reach_of(tool, &json!({})), &json!({})), "{tool}");
        }
        let mem = |tool: &str, args: Value| leaves(memory_reach(tool, &args), &args);
        assert!(mem("memory_upkeep", json!({"op": "issue_add", "kind": "chain", "note": "x"})));
        assert!(mem("memory_add", json!({"content": "x", "global": true})));
        assert!(mem("memory_add", json!({"content": "x", "cwd": "/elsewhere"})));
        assert!(!mem("memory_add", json!({"content": "x"})));
        assert!(!mem("memory_search", json!({"query": "x", "scope": "/elsewhere"})));
        assert!(mem("memory_not_yet_classed", json!({})), "unknown fails closed");
    }

    /// AppTool and RunApp stay in the turn's own app and carry words out
    /// into any other.
    #[test]
    fn reaching_into_another_app_carries_words_out() {
        let into = |tool: &str, args: Value| leaves(reach_of(tool, &args), &args, "ledger");
        assert!(into("AppTool", json!({"app": "dice", "tool": "roll"})));
        assert!(!into("AppTool", json!({"app": "ledger", "tool": "read"})));
        assert!(!into("AppTool", json!({})), "the listing names no app");
        assert!(into("RunApp", json!({"skill": "dice"})));
        assert!(!into("RunApp", json!({"skill": "ledger"})));
    }
}
