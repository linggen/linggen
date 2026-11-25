//! The engine's MCP catalogue: memory's branches (from the memory crate)
//! composed with the engine's own — browser control, delegation, and the
//! two upkeep steps only the engine can run. One entry per branch decides
//! its name, schema, timeout, dispatcher and who can see and call it
//! (`ling_mem::catalog`); the door's handlers read this table and keep no
//! list of names of their own.
//!
//! `/mcp` publishes it as fifteen tools — memory 5, agents 4, browser 3,
//! apps 1, other agents' memory 1, Linggen's own notes 1 — with a view per caller: this machine sees all fifteen, an approved
//! computer the five memory tools without `dream_run`. The names published
//! before the tools were folded are [`RETIRED`] (memory's are memory's):
//! a call by one is told to update the plugin.

use crate::engine::tools::browser_def;
use crate::own_memory;
use ling_mem::catalog::{Branch, Catalog, Retired, ToolDoc, Views};
use serde_json::json;
use std::sync::LazyLock;

// The browser tools are defined once, in the engine (`browser_def`): their
// time limits and answer shapes are read from there.
#[cfg(test)]
pub(super) use crate::engine::tools::browser_def::{CALL_TIMEOUT_MS, GATED_TIMEOUT_MS, NAVIGATE_TIMEOUT_MS};
pub(crate) use crate::engine::tools::browser_def::Render;

/// What runs a branch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Dispatch {
    /// One op over the browser bridge, its answer rendered as `render`.
    Bridge {
        module: &'static str,
        op: &'static str,
        render: Render,
    },
    Agent(AgentOp),
    /// Composed read: memory's days rollup + the engine's run state.
    DreamStatus,
    /// The dream mission through `trigger_mission_core` — the same guarded
    /// path the HTTP trigger and the calendar use.
    DreamRun,
    /// A memory verb, through the memory this process hosts.
    Memory(&'static str),
    /// An app's actions, for an agent outside Linggen (`mcp::app`).
    App(AppOp),
    /// What other agents remember, read-only (`mcp::outside`).
    Outside(OutsideOp),
    /// Linggen's own notes (`own_memory`), one op of `memory_notes`
    /// (`mcp::notes`): `index`, `read`, `search` or `write`.
    Notes(&'static str),
}

/// The delegation calls (`mcp_agent`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AgentOp {
    Run,
    Status,
    /// Every model with whether it can be used now, and why not.
    Readiness,
    Permit,
    Stop,
    Close,
}

/// The `app` tool's two ops.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AppOp {
    /// The apps with actions an outside agent may use, or one app's.
    List,
    /// One of an app's actions, run.
    Call,
}

/// The `memory_agents` tool's four ops.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum OutsideOp {
    /// How this Linggen serves outside agents: `v1` or `v2`.
    Mode,
    /// The other agents' index lines for a project, for a calling host.
    Index,
    /// One note, by path, from inside a registered source.
    Read,
    /// Words, across every registered source.
    Search,
}

/// A host's permission modes as `agent_run`'s description names them, made
/// from `mcp_agent::HostMode`: "plan = read-only, default/acceptEdits = edit, …".
fn host_modes_in_brief() -> String {
    grant_phrases(|words, grant| format!("{words} = {grant}"), |words| format!("{words} = read-only"))
}

/// The `permission_mode` schema's description, made from `HostMode`.
fn host_mode_schema_text() -> String {
    let modes = grant_phrases(|words, grant| format!("{words} grant {grant}"), |words| format!("{words} is read-only"));
    format!("Your current permission mode; {modes} in write_dir only. New sessions only.")
}

/// The `permission_mode` schema's enum: `HostMode`'s words.
fn host_mode_words() -> Vec<&'static str> {
    crate::server::mcp_agent::HostMode::ALL.iter().map(|m| m.word()).collect()
}

/// `HostMode`'s words grouped by grant, each group phrased by `granted` or
/// `read_only`, joined by ", ".
fn grant_phrases(
    granted: impl Fn(&str, crate::engine::permission::PermissionMode) -> String,
    read_only: impl Fn(&str) -> String,
) -> String {
    crate::server::mcp_agent::HostMode::by_grant()
        .iter()
        .map(|(words, grant)| match grant {
            Some(mode) => granted(words, *mode),
            None => read_only(words),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The engine's tools as published: [`TOOLS`] with `{host_modes}` filled in
/// from `HostMode` (`host_modes_in_brief`).
fn published_tools() -> &'static [ToolDoc] {
    static PUBLISHED: LazyLock<Vec<ToolDoc>> = LazyLock::new(|| {
        let brief = host_modes_in_brief();
        TOOLS
            .iter()
            .map(|t| ToolDoc {
                description: match t.description.contains("{host_modes}") {
                    true => Box::leak(t.description.replace("{host_modes}", &brief).into_boxed_str()),
                    false => t.description,
                },
                ..*t
            })
            .collect()
    });
    &PUBLISHED
}

/// The engine's tools but the browser's, in the order the door lists them:
/// the agents' four before the browser's three, the app tool after; the
/// `{host_modes}` in them is filled in when published ([`published_tools`]).
const AGENT_TOOLS: &[ToolDoc] = &[
    ToolDoc {
        name: "agent_run",
        description: "Delegate to a local Linggen agent; only its final reply returns, its steps show in the web UI. For code changes create a git worktree and pass its absolute path as write_dir with your permission_mode ({host_modes} there; other paths stay read-only). grants adds read/write folders, screened per folder. Continue with session_id, omitting write_dir, permission_mode and model. model_fallback defaults false: a failing model ends the run naming the model and reason. Watch the returned ws://…/api/agent/wait URL: permission_request (answer with agent_permit) and finished (reply, followup, error, model, and cannot_run when the run could not go on: reason, model, resets_at, line, work_kept).",
        implicit_op: None,
    },
    ToolDoc {
        name: "agent_status",
        description: "op=session: one session's running work, pending permission requests, status, error, the model that answered, reply, followup, denied actions, grants and cannot_run (why it stopped: a usage limit carries resets_at) — once when a watch ends without finished; never poll. It also carries usage, what the session's model calls used and cost: per-model tokens, cost_usd only for a model with a known price, the total, and unpriced (models whose tokens the total leaves out). op=models: every model and whether it can be used now (ready, or why not and what to do); pick a ready one for agent_run's model.",
        implicit_op: None,
    },
    ToolDoc {
        name: "agent_permit",
        description: "Answer a session's permission request (a permission_request event, or agent_status pending). A cut Bash summary carries the whole command: read it first. allow_once runs that action; allow_session grants its path (scope=path) or folder (scope=dir) for the session; an outward request takes allow_once or deny only; deny refuses. Unanswered requests are denied after 10 minutes.",
        implicit_op: None,
    },
    ToolDoc {
        name: "agent_end",
        description: "op=stop cancels a session's running turn and subagents. op=close stops it and, with remove_worktree=true after you reviewed and merged, removes its linked worktree — only a clean one whose HEAD is on a remote branch.",
        implicit_op: None,
    },
];

const APP_TOOLS: &[ToolDoc] = &[
    ToolDoc {
        name: "app",
        description: "Use what the person's apps do on this Mac, such as playing music: op=list shows each app with what it is for and its actions; with app, that app's actions with their arguments. op=call runs one (app, action, args) and returns its result. A destructive action is refused: the person does it in the app. A private app answers only once the owner opens it to outside agents in Linggen's Settings.",
        implicit_op: None,
    },
];

const OUTSIDE_TOOLS: &[ToolDoc] = &[
    ToolDoc {
        name: "memory_agents",
        description: "What other agents (Claude Code, Codex) remember, read from their own files. Linggen only reads: it never edits, moves or copies them, and each agent keeps its memory its own way. op=index: what a session starts with for a project (cwd) — Linggen's own notes by topic, then their index lines, never the calling host's own (host); op=read: one note by path, only inside those memories; op=search: words across them, any language. For this Mac's owner at this Mac.",
        implicit_op: None,
    },
];

const NOTES_TOOLS: &[ToolDoc] = &[
    ToolDoc {
        name: "memory_notes",
        description: own_memory::spec::DESCRIPTION,
        implicit_op: None,
    },
];

/// Every tool the engine adds to the door: the agents', the browser's
/// (defined in `browser_def`), the app tool and what other agents remember.
static TOOLS: LazyLock<Vec<ToolDoc>> = LazyLock::new(|| {
    AGENT_TOOLS
        .iter()
        .chain(browser_def::TOOLS)
        .chain(APP_TOOLS)
        .chain(OUTSIDE_TOOLS)
        .chain(NOTES_TOOLS)
        .copied()
        .collect()
});

/// The engine's tool names published before the tools were folded, and
/// what does their work now. The X reads left the door (the browser
/// bridge's X module stays, for Linggen's own apps).
pub(super) const RETIRED: &[Retired] = &[
    Retired {
        name: "browser_navigate",
        now: "browser_open op=navigate",
    },
    Retired {
        name: "browser_read_page",
        now: "browser_read op=page",
    },
    Retired {
        name: "browser_click",
        now: "browser_act op=click",
    },
    Retired {
        name: "browser_type",
        now: "browser_act op=type",
    },
    Retired {
        name: "browser_key",
        now: "browser_act op=key",
    },
    Retired {
        name: "browser_scroll",
        now: "browser_act op=scroll",
    },
    Retired {
        name: "browser_screenshot",
        now: "browser_read op=screenshot",
    },
    Retired {
        name: "browser_wait",
        now: "browser_act op=wait",
    },
    Retired {
        name: "browser_tabs",
        now:
            "browser_open op=tab_open, browser_read op=tabs, browser_act op=tab_switch / tab_close",
    },
    Retired {
        name: "browser_read_console",
        now: "browser_read op=console",
    },
    Retired {
        name: "x_search",
        now: "nothing: the X reads left Linggen's MCP door",
    },
    Retired {
        name: "x_targets",
        now: "nothing: the X reads left Linggen's MCP door",
    },
    Retired {
        name: "x_following",
        now: "nothing: the X reads left Linggen's MCP door",
    },
    Retired {
        name: "x_whotofollow",
        now: "nothing: the X reads left Linggen's MCP door",
    },
    Retired {
        name: "x_own",
        now: "nothing: the X reads left Linggen's MCP door",
    },
    Retired {
        name: "x_mentions",
        now: "nothing: the X reads left Linggen's MCP door",
    },
    Retired {
        name: "memory_dream_status",
        now: "memory_upkeep op=dream_status",
    },
    Retired {
        name: "memory_dream_run",
        now: "memory_upkeep op=dream_run",
    },
    Retired {
        name: "agent_stop",
        now: "agent_end op=stop",
    },
    Retired {
        name: "agent_models",
        now: "agent_status op=models",
    },
    Retired {
        name: "agent_session_close",
        now: "agent_end op=close",
    },
];

/// The engine's branches but the browser's.
const OWN_BRANCHES: &[Branch<Dispatch>] = &[
    Branch {
        tool: "memory_upkeep",
        op: "dream_status",
        note: "dream_status: undreamed days, first_unscanned, open_issues, in_flight, the last run and its failures; show last_run_error, and last_run_cannot_run says why a run could not go on (reason, model, resets_at, line). Offer the dream with your own model (the dream runbook) when days await it.",
        schema: || json!({
            "type": "object",
            "properties": {}
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::MEMORY_V1,
        dispatch: Dispatch::DreamStatus,
    },
    Branch {
        tool: "memory_upkeep",
        op: "dream_run",
        note: "dream_run runs the dream on this machine's Linggen model and returns at once; read dream_status afterwards.",
        schema: || json!({
            "type": "object",
            "properties": {
                "day": {"type": "string", "description": "Optional target day, YYYY-MM-DD — one-day scoped run (the calendar shape)"}
            }
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL_V1,
        dispatch: Dispatch::DreamRun,
    },
    Branch {
        tool: "agent_run",
        op: "run",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string", "description": "Self-contained brief: files, cause, tests to run, and ask for a short reply"},
                "session_id": {"type": "string", "description": "Continue an existing session with its context; omit write_dir, permission_mode and model"},
                "model": {"type": "string", "description": "Model for a new session; agent_status op=models lists which are ready"},
                "agent": {"type": "string", "description": "Agent to run (default: ling). Unknown names return the available list."},
                "model_fallback": {"type": "boolean", "description": "Default false: if the model fails the run ends naming the model and the reason, rather than switching to another model; true allows the automatic switch."},
                "host": {"type": "string", "description": "Calling host, e.g. claude-code, codex or cursor (default claude-code)"},
                "permission_mode": {"type": "string", "enum": host_mode_words(), "description": host_mode_schema_text()},
                "work": {"type": "string", "enum": ["code", "none"], "description": "New sessions: code adds coding guidance and lets the run run programs where it may edit without asking (outward commands still ask you); defaults to code with write_dir, otherwise none; ignored on continuation."},
                "write_dir": {"type": "string", "description": "For code changes, create a git worktree first and pass its existing absolute path. The grant follows permission_mode; other paths stay read-only, and an action beyond that waits for your agent_permit. Omit for read-only; new sessions only (a continued session keeps its grant)."},
                "grants": {"type": "object", "description": "New sessions only: folders the run may use without asking. read: folders to read. write: more folders at write_dir's level. Folders your level does not allow are refused; the run still starts, and the reply and agent_status (grants) list what was granted and refused, with reasons. Programs need no grant: a run with write_dir runs them where it may edit.", "properties": {
                    "read": {"type": "array", "items": {"type": "string"}},
                    "write": {"type": "array", "items": {"type": "string"}}
                }}
            },
            "required": ["prompt"]
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Agent(AgentOp::Run),
    },
    Branch {
        tool: "agent_status",
        op: "session",
        note: "",
        schema: || json!({"type": "object", "properties": {"session_id": {"type": "string", "description": "Linggen session ID"}}, "required": ["session_id"]}),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Agent(AgentOp::Status),
    },
    Branch {
        tool: "agent_permit",
        op: "permit",
        note: "",
        schema: || json!({"type": "object", "properties": {
            "session_id": {"type": "string", "description": "Linggen session ID"},
            "request_id": {"type": "string", "description": "The request's id"},
            "decision": {"type": "string", "enum": ["allow_once", "allow_session", "deny"]},
            "scope": {"type": "string", "enum": ["path", "dir"], "description": "allow_session only: the exact target, or its folder (default dir when grantable)"}
        }, "required": ["session_id", "request_id", "decision"]}),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: Some(rules::permit),
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Agent(AgentOp::Permit),
    },
    Branch {
        tool: "agent_end",
        op: "stop",
        note: "",
        schema: || json!({"type": "object", "properties": {"session_id": {"type": "string", "description": "Linggen session ID"}}, "required": ["session_id"]}),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Agent(AgentOp::Stop),
    },
    Branch {
        tool: "agent_status",
        op: "models",
        note: "",
        schema: || json!({"type": "object", "properties": {}}),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Agent(AgentOp::Readiness),
    },
    Branch {
        tool: "agent_end",
        op: "close",
        note: "",
        schema: || json!({"type": "object", "properties": {"session_id": {"type": "string", "description": "Linggen session ID"}, "remove_worktree": {"type": "boolean", "description": "Remove the clean, remote-contained linked worktree after work stops (default false)"}}, "required": ["session_id"]}),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Agent(AgentOp::Close),
    },
    Branch {
        tool: "app",
        op: "list",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "app": {"type": "string", "description": "An app's name; omit to list every app that offers an action"}
            }
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::App(AppOp::List),
    },
    Branch {
        tool: "app",
        op: "call",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "app": {"type": "string", "description": "The app's name"},
                "action": {"type": "string", "description": "The action to run, as op=list names it"},
                "args": {"type": "object", "description": "The action's arguments, as op=list shows them"}
            },
            "required": ["app", "action"]
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::App(AppOp::Call),
    },
    Branch {
        tool: "memory_agents",
        op: "mode",
        note: "",
        schema: || json!({"type": "object", "properties": {}}),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Outside(OutsideOp::Mode),
    },
    Branch {
        tool: "memory_agents",
        op: "index",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "cwd": {"type": "string", "description": "The project folder; the nearest project's lines come first"},
                "host": {"type": "string", "description": "The calling host, e.g. claude-code or codex: its own memory is left out"}
            }
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Outside(OutsideOp::Index),
    },
    Branch {
        tool: "memory_agents",
        op: "read",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "A note's absolute path (or ~/…), as the index names its folder and file"}
            },
            "required": ["path"]
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Outside(OutsideOp::Read),
    },
    Branch {
        tool: "memory_agents",
        op: "search",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "words": {"type": "string", "description": "Words to find (any language, any case); a line with more of them ranks higher"},
                "limit": {"type": "integer", "description": "Most lines to return (default 10, at most 30)"}
            },
            "required": ["words"]
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Outside(OutsideOp::Search),
    },
    Branch {
        tool: "memory_notes",
        op: "index",
        note: "",
        schema: || notes_schema(own_memory::spec::Op::Index),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Notes("index"),
    },
    Branch {
        tool: "memory_notes",
        op: "read",
        note: "",
        schema: || notes_schema(own_memory::spec::Op::Read),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Notes("read"),
    },
    Branch {
        tool: "memory_notes",
        op: "search",
        note: "",
        schema: || notes_schema(own_memory::spec::Op::Search),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Notes("search"),
    },
    Branch {
        tool: "memory_notes",
        op: "write",
        note: "",
        schema: || notes_schema(own_memory::spec::Op::Write),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Notes("write"),
    },
];

/// One op of `memory_notes`: the fields `own_memory::spec` defines for it,
/// and the calling host, which only the door has (a session knows who it is).
fn notes_schema(op: own_memory::spec::Op) -> serde_json::Value {
    use own_memory::spec::Op;
    let mut props = own_memory::spec::properties(op);
    if matches!(op, Op::Index | Op::Write) {
        props["host"] = json!({"type": "string", "description": "The calling host, e.g. claude-code or codex: recorded as the writer"});
    }
    json!({"type": "object", "properties": props, "required": []})
}

/// The engine's branches: the browser's, from `browser_def`, then the rest.
static BRANCHES: LazyLock<Vec<Branch<Dispatch>>> = LazyLock::new(|| {
    browser_def::BRANCHES
        .iter()
        .map(|b| {
            let call = b.dispatch;
            b.with(Dispatch::Bridge {
                module: call.module,
                op: call.op,
                render: call.render,
            })
        })
        .chain(OWN_BRANCHES.iter().copied())
        .collect()
});

/// One side's part of the catalogue: its tools, branches and retired names.
#[derive(Clone, Copy)]
pub(super) struct Side<'a, D: Copy + 'static> {
    pub tools: &'a [ToolDoc],
    pub branches: &'a [Branch<D>],
    pub retired: &'a [Retired],
}

/// Memory's side as the memory crate defines it.
pub(super) fn memory_side() -> Side<'static, &'static str> {
    use ling_mem::catalog::memory;
    Side {
        tools: memory::TOOLS,
        branches: memory::BRANCHES,
        retired: memory::RETIRED,
    }
}

/// The engine's own side.
pub(super) fn engine_side() -> Side<'static, Dispatch> {
    Side {
        tools: published_tools(),
        branches: &BRANCHES,
        retired: RETIRED,
    }
}

/// Memory's branches and the engine's as one catalogue, or why they cannot
/// be: a name that would decide two things is refused here.
pub(super) fn compose(
    memory: Side<'_, &'static str>,
    engine: Side<'_, Dispatch>,
) -> Result<Catalog<Dispatch>, String> {
    let tools = memory.tools.iter().chain(engine.tools).copied().collect();
    let branches = memory
        .branches
        .iter()
        .map(|b| b.with(Dispatch::Memory(b.dispatch)))
        .chain(engine.branches.iter().copied())
        .collect();
    let retired = memory
        .retired
        .iter()
        .chain(engine.retired)
        .copied()
        .collect();
    Catalog::new(tools, branches, retired)
}

/// The door's catalogue.
pub(crate) fn catalog() -> &'static Catalog<Dispatch> {
    static CATALOG: LazyLock<Catalog<Dispatch>> = LazyLock::new(|| {
        compose(memory_side(), engine_side()).expect("the MCP catalogue is consistent")
    });
    &CATALOG
}
/// The cross-field rule the agent branches check before anything runs (the
/// browser's are in `browser_def`).
mod rules {
    use serde_json::{Map, Value};

    pub(super) fn permit(obj: &Map<String, Value>) -> Result<(), String> {
        let session_wide = obj.get("decision").and_then(Value::as_str) == Some("allow_session");
        if obj.contains_key("scope") && !session_wide {
            return Err("`scope` goes with decision=allow_session only".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ling_mem::catalog::View;

    fn names(view: View) -> Vec<String> {
        catalog()
            .tools_list(view)
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    }

    /// The door, from the one table: memory 5, agents 4, browser 3, apps 1,
    /// what other agents remember 1, Linggen's own notes 1.
    #[test]
    fn the_door_is_fifteen_tools() {
        assert_eq!(
            names(View::Local),
            [
                "memory_search",
                "memory_add",
                "memory_update",
                "memory_delete",
                "memory_upkeep",
                "agent_run",
                "agent_status",
                "agent_permit",
                "agent_end",
                "browser_open",
                "browser_read",
                "browser_act",
                "app",
                "memory_agents",
                "memory_notes",
            ]
        );
    }

    /// The forty-one tools of the two servers before the fold: seven kept
    /// their name, the other thirty-four are retired, each once, none a
    /// tool again; and every branch is published by a tool.
    #[test]
    fn every_old_name_is_kept_or_retired_once() {
        let cat = catalog();
        let kept = [
            "memory_search",
            "memory_add",
            "memory_update",
            "memory_delete",
            "agent_run",
            "agent_status",
            "agent_permit",
        ];
        let current: Vec<&str> = cat.tools().iter().map(|t| t.name).collect();
        for name in kept {
            assert!(current.contains(&name), "{name}");
        }
        assert_eq!(cat.retired().len(), 34);
        assert_eq!(kept.len() + cat.retired().len(), 41);
        for b in cat.branches() {
            assert!(current.contains(&b.tool), "{} {}", b.tool, b.op);
        }
    }

    /// The browser tools are defined once (`engine::tools::browser_def`): a
    /// Linggen session is offered the same three the door publishes, name,
    /// text and schema word for word — and no other browser tool. A second
    /// copy of any of them, here or in the session, fails this.
    #[test]
    fn a_session_is_offered_the_doors_browser_tools_word_for_word() {
        use crate::engine::tool_registry::ToolRegistry;
        use crate::engine::tools::Tools;
        let session = ToolRegistry::new(Tools::new(std::path::PathBuf::from(".")).unwrap())
            .oai_tool_definitions(None);
        let offered: Vec<(String, String, serde_json::Value)> = session
            .iter()
            .filter(|d| d["function"]["name"].as_str().is_some_and(|n| n.to_lowercase().starts_with("browser_")))
            .map(|d| {
                let f = &d["function"];
                (
                    f["name"].as_str().unwrap().to_string(),
                    f["description"].as_str().unwrap().to_string(),
                    f["parameters"].clone(),
                )
            })
            .collect();
        let door: Vec<(String, String, serde_json::Value)> = catalog()
            .tools_list(ling_mem::catalog::View::Local)
            .iter()
            .filter(|t| t["name"].as_str().is_some_and(|n| n.starts_with("browser_")))
            .map(|t| {
                (
                    t["name"].as_str().unwrap().to_string(),
                    t["description"].as_str().unwrap().to_string(),
                    t["inputSchema"].clone(),
                )
            })
            .collect();
        assert_eq!(door.len(), 3, "the door's browser tools");
        assert_eq!(offered, door);
    }

    /// `agent_run` publishes `HostMode`'s words, and its description names
    /// each of them, none left as a placeholder.
    #[test]
    fn agent_run_publishes_the_host_modes_it_takes() {
        let words: Vec<&str> = crate::server::mcp_agent::HostMode::ALL.iter().map(|m| m.word()).collect();
        let listed = catalog().tools_list(ling_mem::catalog::View::Local);
        let run = listed.iter().find(|t| t["name"] == "agent_run").expect("agent_run");
        let text = run.to_string();
        assert!(!text.contains("{host_modes}"), "{text}");
        let schema = BRANCHES.iter().find(|b| b.tool == "agent_run" && b.op == "run").unwrap();
        let mode = &(schema.schema)()["properties"]["permission_mode"];
        assert_eq!(mode["enum"], json!(words));
        for word in words {
            assert!(run["description"].as_str().unwrap().contains(word), "{word}");
            assert!(mode["description"].as_str().unwrap().contains(word), "{word}");
        }
    }

    /// Composing refuses a name memory and the engine would both decide.
    #[test]
    fn composing_refuses_a_name_both_sides_define() {
        let clash = ToolDoc {
            name: "memory_add",
            description: "",
            implicit_op: None,
        };
        let mut tools = TOOLS.to_vec();
        tools.push(clash);
        let engine = Side {
            tools: &tools,
            ..engine_side()
        };
        let why = compose(memory_side(), engine)
            .err()
            .expect("a second memory_add is refused");
        assert!(why.contains("memory_add"), "{why}");

        let mut retired = RETIRED.to_vec();
        retired.push(Retired {
            name: "memory_get",
            now: "",
        });
        let engine = Side {
            retired: &retired,
            ..engine_side()
        };
        let why = compose(memory_side(), engine)
            .err()
            .expect("a second retired memory_get is refused");
        assert!(why.contains("memory_get"), "{why}");

        let mut retired = RETIRED.to_vec();
        retired.push(Retired {
            name: "browser_act",
            now: "",
        });
        let engine = Side {
            retired: &retired,
            ..engine_side()
        };
        let why = compose(memory_side(), engine)
            .err()
            .expect("a retired name that is a tool is refused");
        assert!(why.contains("browser_act"), "{why}");

        let mut upkeep = BRANCHES.to_vec();
        let mut again = *upkeep.iter().find(|b| b.op == "dream_run").unwrap();
        again.op = "sweep";
        upkeep.push(again);
        let engine = Side {
            branches: &upkeep,
            ..engine_side()
        };
        let why = compose(memory_side(), engine)
            .err()
            .expect("a second upkeep sweep is refused");
        assert!(why.contains("sweep"), "{why}");
    }
}
