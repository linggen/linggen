//! The engine's MCP catalogue: memory's branches (from the memory crate)
//! composed with the engine's own — browser control, delegation, and the
//! two upkeep steps only the engine can run. One entry per branch decides
//! its name, schema, timeout, dispatcher and who can see and call it
//! (`ling_mem::catalog`); the door's handlers read this table and keep no
//! list of names of their own.
//!
//! `/mcp` publishes it as thirteen tools — memory 5, agents 4, browser 3,
//! apps 1 — with a view per caller: this machine sees all thirteen, an approved
//! computer the five memory tools without `dream_run`. The names published
//! before the tools were folded are [`RETIRED`] (memory's are memory's):
//! a call by one is told to update the plugin.

use ling_mem::catalog::{Branch, Catalog, Retired, ToolDoc, Views};
use serde_json::json;
use std::sync::LazyLock;

pub(super) const CALL_TIMEOUT_MS: u64 = 20_000;
pub(super) const NAVIGATE_TIMEOUT_MS: u64 = 45_000;
/// Mutating ops wait on the extension's permission prompt (a human, up to
/// 120s) before acting.
pub(super) const GATED_TIMEOUT_MS: u64 = 150_000;

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
}

/// How a bridge answer reaches the caller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Render {
    Text,
    /// An accessibility tree, headed by the page's URL and title.
    Page,
    /// A PNG image content block.
    Image,
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

/// The engine's tools, after memory's five; `{host_modes}` is filled in
/// when published ([`published_tools`]).
const TOOLS: &[ToolDoc] = &[
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
    ToolDoc {
        name: "browser_open",
        description: "Load a page in the one visible Chrome tab Linggen controls: op=navigate (an absolute URL, or back/forward) or op=tab_open (an absolute URL, creating the tab if needed). Chrome may ask the user to allow this site; they approve or decline there.",
        implicit_op: None,
    },
    ToolDoc {
        name: "browser_read",
        description: "Read the controlled tab: op=page (accessibility tree with refs like n42 for browser_act; refs go stale after the page changes, so read again), op=screenshot (an image, for what the tree cannot show), op=console (recent messages), op=tabs (its state).",
        implicit_op: None,
    },
    ToolDoc {
        name: "browser_act",
        description: "Act in the controlled tab: click (ref or coordinate), type, key, scroll, wait (load, selector, or ms), tab_switch, tab_close. Read the page again after it changes. Chrome may ask the user to allow this site; they approve or decline there.",
        implicit_op: None,
    },
    ToolDoc {
        name: "app",
        description: "Use what the person's apps do on this Mac, such as playing music: op=list shows each app with what it is for and its actions; with app, that app's actions with their arguments. op=call runs one (app, action, args) and returns its result. A destructive action is refused: the person does it in the app. A private app answers only once the owner opens it to outside agents in Linggen's Settings.",
        implicit_op: None,
    },
];

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

/// The engine's branches.
const BRANCHES: &[Branch<Dispatch>] = &[
    Branch {
        tool: "browser_open",
        op: "navigate",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Absolute URL to load, or \"back\" / \"forward\""}
            },
            "required": ["url"]
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: Some(rules::navigate),
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "navigate", render: Render::Text },
    },
    Branch {
        tool: "browser_read",
        op: "page",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "filter": {
                    "type": "string",
                    "enum": ["all", "interactive"],
                    "description": "\"interactive\" returns only actionable nodes (smaller); default \"all\""
                }
            }
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "read_page", render: Render::Page },
    },
    Branch {
        tool: "browser_act",
        op: "click",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "ref": {"type": "string", "description": "Node ref from browser_read op=page, e.g. \"n42\""},
                "coordinate": {
                    "type": "array", "items": {"type": "number"},
                    "minItems": 2, "maxItems": 2,
                    "description": "Viewport [x, y] — only when no ref exists for the target"
                },
                "button": {"type": "string", "enum": ["left", "middle", "right"]},
                "double": {"type": "boolean", "description": "Double-click"}
            }
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: Some(rules::click),
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "click", render: Render::Text },
    },
    Branch {
        tool: "browser_act",
        op: "type",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "Text to type"},
                "ref": {"type": "string", "description": "Field ref from browser_read op=page; omit to use current focus"},
                "clear": {"type": "boolean", "description": "Clear the field before typing"}
            },
            "required": ["text"]
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "type", render: Render::Text },
    },
    Branch {
        tool: "browser_act",
        op: "key",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "keys": {"type": "string", "description": "Key or chord, e.g. \"Enter\", \"Tab\", \"Ctrl+a\""},
                "repeat": {"type": "integer", "description": "Press count (default 1, max 20)"}
            },
            "required": ["keys"]
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: Some(rules::key),
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "key", render: Render::Text },
    },
    Branch {
        tool: "browser_act",
        op: "scroll",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "direction": {"type": "string", "enum": ["up", "down", "left", "right"]},
                "amount": {"type": "integer", "description": "Pixels (default 600)"},
                "ref": {"type": "string", "description": "Scroll at this node instead of page center"}
            },
            "required": ["direction"]
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "scroll", render: Render::Text },
    },
    Branch {
        tool: "browser_read",
        op: "screenshot",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "region": {
                    "type": "object",
                    "properties": {
                        "x": {"type": "number"}, "y": {"type": "number"},
                        "width": {"type": "number"}, "height": {"type": "number"}
                    },
                    "description": "Optional viewport region to capture; omit for the full viewport"
                }
            }
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "screenshot", render: Render::Image },
    },
    Branch {
        tool: "browser_act",
        op: "wait",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "for": {"type": "string", "enum": ["load", "selector", "ms"]},
                "value": {"type": "string", "description": "CSS selector (for=selector) or milliseconds (for=ms)"}
            },
            "required": ["for"]
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: Some(rules::wait),
        timeout_ms: NAVIGATE_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "wait", render: Render::Text },
    },
    Branch {
        tool: "browser_open",
        op: "tab_open",
        note: "",
        schema: || json!({"type": "object", "properties": {"url": {"type": "string", "description": "Absolute URL to open in the controlled tab, creating it if needed"}}, "required": ["url"]}),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[("action", "open")],
        check: Some(rules::tab_open),
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "tabs", render: Render::Text },
    },
    Branch {
        tool: "browser_read",
        op: "tabs",
        note: "",
        schema: || json!({"type": "object", "properties": {}}),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[("action", "list")],
        check: None,
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "tabs", render: Render::Text },
    },
    Branch {
        tool: "browser_act",
        op: "tab_switch",
        note: "",
        schema: || json!({"type": "object", "properties": {}}),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[("action", "switch")],
        check: None,
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "tabs", render: Render::Text },
    },
    Branch {
        tool: "browser_act",
        op: "tab_close",
        note: "",
        schema: || json!({"type": "object", "properties": {}}),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[("action", "close")],
        check: None,
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "tabs", render: Render::Text },
    },
    Branch {
        tool: "browser_read",
        op: "console",
        note: "",
        schema: || json!({
            "type": "object",
            "properties": {
                "limit": {"type": "integer", "description": "Max messages (default 50)"}
            }
        }),
        extra: None,
        hidden: &[],
        stamps: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "read_console", render: Render::Text },
    },
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
        views: Views::ALL,
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
        views: Views::LOCAL,
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
    },];

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
        branches: BRANCHES,
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
/// The cross-field rules the folded browser and agent branches check
/// before anything runs.
mod rules {
    use serde_json::{Map, Value};

    fn text<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
        obj.get(key).and_then(Value::as_str)
    }

    /// `https://…`, `file:…`, `about:blank`: a scheme, then the rest.
    fn absolute(url: &str) -> bool {
        let Some((scheme, rest)) = url.split_once(':') else {
            return false;
        };
        let mut chars = scheme.chars();
        let starts = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
        let fits = chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
        starts && fits && !rest.is_empty()
    }

    pub(super) fn navigate(obj: &Map<String, Value>) -> Result<(), String> {
        match text(obj, "url") {
            Some("back" | "forward") => Ok(()),
            Some(url) if absolute(url) => Ok(()),
            _ => Err("`url` must be an absolute URL, or \"back\" / \"forward\"".into()),
        }
    }

    pub(super) fn tab_open(obj: &Map<String, Value>) -> Result<(), String> {
        match text(obj, "url") {
            Some(url) if absolute(url) => Ok(()),
            _ => Err("`url` must be an absolute URL".into()),
        }
    }

    pub(super) fn click(obj: &Map<String, Value>) -> Result<(), String> {
        match (obj.contains_key("ref"), obj.contains_key("coordinate")) {
            (true, false) | (false, true) => Ok(()),
            (true, true) => Err("pass `ref` or `coordinate`, not both".into()),
            (false, false) => Err("needs `ref` (from browser_read op=page) or `coordinate`".into()),
        }
    }

    pub(super) fn key(obj: &Map<String, Value>) -> Result<(), String> {
        match obj.get("repeat").and_then(Value::as_i64) {
            Some(n) if !(1..=20).contains(&n) => Err("`repeat` must be 1–20".into()),
            _ => Ok(()),
        }
    }

    pub(super) fn wait(obj: &Map<String, Value>) -> Result<(), String> {
        let value = text(obj, "value");
        match (text(obj, "for"), value) {
            (Some("load"), None) => Ok(()),
            (Some("load"), Some(_)) => Err("for=load takes no `value`".into()),
            (Some("selector"), Some(_)) => Ok(()),
            (Some("selector"), None) => Err("for=selector needs `value`, a CSS selector".into()),
            (Some("ms"), Some(ms)) => match ms.trim().parse::<u64>() {
                Ok(n) if n <= 10_000 => Ok(()),
                _ => Err("for=ms needs `value`, milliseconds 0–10000 as a string".into()),
            },
            (Some("ms"), None) => Err("for=ms needs `value`, milliseconds 0–10000".into()),
            _ => Ok(()),
        }
    }

    pub(super) fn permit(obj: &Map<String, Value>) -> Result<(), String> {
        let session_wide = text(obj, "decision") == Some("allow_session");
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

    /// The door, from the one table: memory 5, agents 4, browser 3, apps 1.
    #[test]
    fn the_door_is_thirteen_tools() {
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
