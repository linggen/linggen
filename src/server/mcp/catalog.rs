//! The engine's MCP catalogue: memory's branches (from the memory crate)
//! composed with the engine's own — browser control, delegation, and the
//! two upkeep steps only the engine can run. One entry per branch decides
//! its name, schema, timeout, dispatcher and who can see and call it
//! (`ling_mem::catalog`); the door's handlers read this table and keep no
//! list of names of their own.
//!
//! Two surfaces come from it. The legacy one is what `/mcp` publishes
//! today: the engine's branches, each as the tool it has been. The folded
//! one is the twelve-tool door — memory 5, agents 4, browser 3 — with a
//! view per caller: this machine sees all twelve, an approved computer the
//! five memory tools without `dream_run`.

use ling_mem::catalog::{Branch, Catalog, ToolDoc, Views};
use serde_json::json;
use std::sync::LazyLock;

pub(super) const CALL_TIMEOUT_MS: u64 = 20_000;
pub(super) const NAVIGATE_TIMEOUT_MS: u64 = 45_000;
/// Mutating ops wait on the extension's permission prompt (a human, up to
/// 120s) before acting.
pub(super) const GATED_TIMEOUT_MS: u64 = 150_000;
/// Session reads open a hidden tab and wait for the site's own responses.
pub(super) const READ_MODULE_TIMEOUT_MS: u64 = 60_000;

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
    /// The legacy list: configured models and the default.
    Models,
    /// Every model with whether it can be used now, and why not.
    Readiness,
    Permit,
    Stop,
    Close,
}

/// The engine's folded tools, after memory's five.
const TOOLS: &[ToolDoc] = &[
    ToolDoc {
        name: "agent_run",
        description: "Delegate to a local Linggen agent; only its final reply returns, its steps show in the web UI. For code changes create a git worktree and pass its absolute path as write_dir with your permission_mode (plan = read-only, default/acceptEdits = edit, auto/bypassPermissions = admin there; other paths stay read-only). grants adds read/write folders, screened per folder. Continue with session_id, omitting write_dir, permission_mode and model. model_fallback defaults false: a failing model ends the run naming the model and reason. Watch the returned ws://…/api/agent/wait URL: permission_request (answer with agent_permit) and finished (reply, followup, error, model).",
        implicit_op: None,
    },
    ToolDoc {
        name: "agent_status",
        description: "op=session: one session's running work, pending permission requests, status, error, the model that answered, reply, followup, denied actions and grants — once when a watch ends without finished; never poll. op=models: every model and whether it can be used now.",
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
        description: "Read the controlled tab: op=page (accessibility tree with refs like n42 for browser_act; refs go stale after the page changes), op=screenshot (an image, for what the tree cannot show), op=console (recent messages), op=tabs (its state).",
        implicit_op: None,
    },
    ToolDoc {
        name: "browser_act",
        description: "Act in the controlled tab: click (ref or coordinate), type, key, scroll, wait (load, selector, or ms), tab_switch, tab_close. Read the page again after it changes. Chrome may ask the user to allow this site; they approve or decline there.",
        implicit_op: None,
    },
];

/// The engine's branches, in the order the legacy door lists its tools;
/// a branch only the folded door has sits beside the one it replaces.
const BRANCHES: &[Branch<Dispatch>] = &[
    Branch {
        tool: Some("browser_open"),
        op: "navigate",
        legacy: Some("browser_navigate"),
        description: "Load a URL (or go \"back\"/\"forward\") in the controlled browser tab. The tab is visible to the user. Follow with browser_read_page to see the result.",
        schema: || json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Absolute URL to load, or \"back\" / \"forward\""}
            },
            "required": ["url"]
        }),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: Some(rules::navigate),
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "navigate", render: Render::Text },
    },
    Branch {
        tool: Some("browser_read"),
        op: "page",
        legacy: Some("browser_read_page"),
        description: "Read the controlled tab as an accessibility tree. Actionable nodes carry a ref like [n42] — pass that ref to browser_click / browser_type. Re-read after any action that changes the page; old refs go stale.",
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
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "read_page", render: Render::Page },
    },
    Branch {
        tool: Some("browser_act"),
        op: "click",
        legacy: Some("browser_click"),
        description: "Click a node by ref (from browser_read_page) or a viewport coordinate. Prefer refs — coordinates are the screenshot fallback.",
        schema: || json!({
            "type": "object",
            "properties": {
                "ref": {"type": "string", "description": "Node ref from browser_read_page, e.g. \"n42\""},
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
        fixed: &[],
        check: Some(rules::click),
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "click", render: Render::Text },
    },
    Branch {
        tool: Some("browser_act"),
        op: "type",
        legacy: Some("browser_type"),
        description: "Type text into a field: pass ref to focus it first (clear:true to empty it), or omit ref to type into the currently focused element.",
        schema: || json!({
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "Text to type"},
                "ref": {"type": "string", "description": "Field ref from browser_read_page; omit to use current focus"},
                "clear": {"type": "boolean", "description": "Clear the field before typing"}
            },
            "required": ["text"]
        }),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "type", render: Render::Text },
    },
    Branch {
        tool: Some("browser_act"),
        op: "key",
        legacy: Some("browser_key"),
        description: "Press a key or chord in the controlled tab, e.g. \"Enter\", \"Escape\", \"Ctrl+a\", \"Meta+Enter\".",
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
        fixed: &[],
        check: Some(rules::key),
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "key", render: Render::Text },
    },
    Branch {
        tool: Some("browser_act"),
        op: "scroll",
        legacy: Some("browser_scroll"),
        description: "Scroll the page (or the element under ref) in the controlled tab.",
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
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "scroll", render: Render::Text },
    },
    Branch {
        tool: Some("browser_read"),
        op: "screenshot",
        legacy: Some("browser_screenshot"),
        description: "Capture the controlled tab as an image. Fallback for visual/canvas content the accessibility tree can't express — prefer browser_read_page for normal pages.",
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
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "screenshot", render: Render::Image },
    },
    Branch {
        tool: Some("browser_act"),
        op: "wait",
        legacy: Some("browser_wait"),
        description: "Wait for the controlled tab to settle before the next read: for \"load\" (page load), \"selector\" (a CSS selector appears, value required), or \"ms\" (fixed delay, max 10000).",
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
        fixed: &[],
        check: Some(rules::wait),
        timeout_ms: NAVIGATE_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "wait", render: Render::Text },
    },
    Branch {
        tool: None,
        op: "tabs",
        legacy: Some("browser_tabs"),
        description: "Manage the controlled tab: list (current state), open (a URL, creating the tab if needed), switch (bring it to front), close.",
        schema: || json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "open", "switch", "close"]},
                "url": {"type": "string", "description": "URL for action=open"}
            },
            "required": ["action"]
        }),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "tabs", render: Render::Text },
    },
    Branch {
        tool: Some("browser_open"),
        op: "tab_open",
        legacy: None,
        description: "",
        schema: || json!({"type": "object", "properties": {"url": {"type": "string", "description": "Absolute URL to open in the controlled tab, creating it if needed"}}, "required": ["url"]}),
        extra: None,
        hidden: &[],
        fixed: &[("action", "open")],
        check: Some(rules::tab_open),
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "tabs", render: Render::Text },
    },
    Branch {
        tool: Some("browser_read"),
        op: "tabs",
        legacy: None,
        description: "",
        schema: || json!({"type": "object", "properties": {}}),
        extra: None,
        hidden: &[],
        fixed: &[("action", "list")],
        check: None,
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "tabs", render: Render::Text },
    },
    Branch {
        tool: Some("browser_act"),
        op: "tab_switch",
        legacy: None,
        description: "",
        schema: || json!({"type": "object", "properties": {}}),
        extra: None,
        hidden: &[],
        fixed: &[("action", "switch")],
        check: None,
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "tabs", render: Render::Text },
    },
    Branch {
        tool: Some("browser_act"),
        op: "tab_close",
        legacy: None,
        description: "",
        schema: || json!({"type": "object", "properties": {}}),
        extra: None,
        hidden: &[],
        fixed: &[("action", "close")],
        check: None,
        timeout_ms: GATED_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "tabs", render: Render::Text },
    },
    Branch {
        tool: Some("browser_read"),
        op: "console",
        legacy: Some("browser_read_console"),
        description: "Read recent console messages from the controlled tab (debugging).",
        schema: || json!({
            "type": "object",
            "properties": {
                "limit": {"type": "integer", "description": "Max messages (default 50)"}
            }
        }),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "control", op: "read_console", render: Render::Text },
    },
    Branch {
        tool: None,
        op: "search",
        legacy: Some("x_search"),
        description: "Search x.com (Twitter) through the user's logged-in session — returns structured posts (author, text, engagement) as JSON, no API keys. Requires being signed in to x.com in the browser.",
        schema: || json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "X search query (operators like from:, -filter:replies work)"},
                "max": {"type": "integer", "description": "Max posts (default 15, max 100)"}
            },
            "required": ["query"]
        }),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: READ_MODULE_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "x", op: "search", render: Render::Text },
    },
    Branch {
        tool: None,
        op: "targets",
        legacy: Some("x_targets"),
        description: "Latest original posts from a set of x.com handles (capped per author for diversity), via the user's logged-in session.",
        schema: || json!({
            "type": "object",
            "properties": {
                "handles": {"type": "array", "items": {"type": "string"}, "description": "Handles to read (max 25), with or without @"},
                "per_author": {"type": "integer", "description": "Max posts per author (default 3)"},
                "max": {"type": "integer", "description": "Max posts overall (default 25, max 100)"}
            },
            "required": ["handles"]
        }),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: READ_MODULE_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "x", op: "targets", render: Render::Text },
    },
    Branch {
        tool: None,
        op: "following",
        legacy: Some("x_following"),
        description: "List the accounts an x.com handle follows (handle, name, followers, bio), via the user's logged-in session.",
        schema: || json!({
            "type": "object",
            "properties": {
                "handle": {"type": "string", "description": "Whose following list to read"},
                "self": {"type": "string", "description": "The user's own handle, excluded from results"},
                "max": {"type": "integer", "description": "Max accounts (default 400, max 1000)"}
            },
            "required": ["handle"]
        }),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: READ_MODULE_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "x", op: "following", render: Render::Text },
    },
    Branch {
        tool: None,
        op: "whotofollow",
        legacy: Some("x_whotofollow"),
        description: "X's personalized \"Who to follow\" recommendations for the signed-in user.",
        schema: || json!({
            "type": "object",
            "properties": {
                "exclude": {"type": "array", "items": {"type": "string"}, "description": "Handles to drop from results"},
                "self": {"type": "string", "description": "The user's own handle, excluded from results"},
                "max": {"type": "integer", "description": "Max accounts (default 60, max 100)"}
            }
        }),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: READ_MODULE_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "x", op: "whotofollow", render: Render::Text },
    },
    Branch {
        tool: None,
        op: "own",
        legacy: Some("x_own"),
        description: "The signed-in user's own recent x.com posts with engagement (views, likes), plus the parent posts they already replied to.",
        schema: || json!({
            "type": "object",
            "properties": {
                "handle": {"type": "string", "description": "The user's x.com handle"},
                "max": {"type": "integer", "description": "Max posts (default 10, max 100)"}
            },
            "required": ["handle"]
        }),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: READ_MODULE_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "x", op: "own", render: Render::Text },
    },
    Branch {
        tool: None,
        op: "mentions",
        legacy: Some("x_mentions"),
        description: "Recent mentions of, and replies to, the signed-in user on x.com — read from their Mentions notifications tab. Each item says whether it is a reply to one of their posts or a bare mention, and carries the post it answers when x.com sent it along.",
        schema: || json!({
            "type": "object",
            "properties": {
                "username": {"type": "string", "description": "The user's x.com handle — tells a reply to them apart from a bare mention"},
                "max": {"type": "integer", "description": "Max items (default 15, max 100)"}
            },
            "required": ["username"]
        }),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: READ_MODULE_TIMEOUT_MS,
        views: Views::LOCAL,
        dispatch: Dispatch::Bridge { module: "x", op: "mentions", render: Render::Text },
    },
    Branch {
        tool: Some("memory_upkeep"),
        op: "dream_status",
        legacy: Some("memory_dream_status"),
        description: "Dream-pipeline status — is memory upkeep due? Returns undreamed days (oldest first, each awaiting a dream pass) with first_undreamed / first_unscanned, past-day summary counts (total_days / scanned_days / dreamed_days), the open review-item count, whether a dream run is in flight, and the last run's outcome (with any tool calls that failed in it: failure_count, failures). If last_run_error is set, surface it to the user verbatim — the engine side may need attention (model not configured, sign-in required, quota). When days await a dream, offer to run it: /linggen:dream runs the pass with YOUR model (no Linggen model needed); memory_dream_run offloads it to the local Linggen engine. When open_issues > 0, offer /linggen:solve.",
        schema: || json!({
            "type": "object",
            "properties": {}
        }),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::ALL,
        dispatch: Dispatch::DreamStatus,
    },
    Branch {
        tool: Some("memory_upkeep"),
        op: "dream_run",
        legacy: Some("memory_dream_run"),
        description: "Run the nightly memory dream on the local Linggen engine (its mission executor and configured model — prefer /linggen:dream to run the pass with YOUR model instead). Optional day (YYYY-MM-DD) scopes the run to one day; omitted runs the full nightly protocol: undreamed days oldest-first, then sweep, then audit. Returns immediately — dream runs take minutes; poll memory_dream_status, and if it reports a failed run surface last_run_error to the user. An in-flight run returns {in_flight: true} — that's state, not an error.",
        schema: || json!({
            "type": "object",
            "properties": {
                "day": {"type": "string", "description": "Optional target day, YYYY-MM-DD — one-day scoped run (the calendar shape)"}
            }
        }),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::DreamRun,
    },
    Branch {
        tool: Some("agent_run"),
        op: "run",
        legacy: Some("agent_run"),
        description: "Delegate to a local Linggen agent; only its final reply returns, while working steps stay visible in the web UI. A long run returns its session: start a new background watcher for every agent_run, including continued sessions; if it closes or expires without finished, call agent_status once, and if that still says running, start a new watch; never poll. Watch on the ws://…/api/agent/wait URL it gives, which wakes you on {\"type\":\"permission_request\"} (answer with agent_permit) and {\"type\":\"finished\"} (carries reply and followup, null if none; status failed carries the error and model).",
        schema: || json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string", "description": "Self-contained brief: files, cause, tests to run, and ask for a short reply"},
                "session_id": {"type": "string", "description": "Continue an existing session with its context; omit write_dir, permission_mode and model"},
                "model": {"type": "string", "description": "Model for a new session; use agent_models to list choices"},
                "agent": {"type": "string", "description": "Agent to run (default: ling). Unknown names return the available list."},
                "model_fallback": {"type": "boolean", "description": "Default false: if the model fails the run ends naming the model and the reason, rather than switching to another model; true allows the automatic switch."},
                "host": {"type": "string", "description": "Calling host, e.g. claude-code, codex or cursor (default claude-code)"},
                "permission_mode": {"type": "string", "enum": ["plan", "default", "acceptEdits", "auto", "bypassPermissions"], "description": "Your current permission mode; plan is read-only, default/acceptEdits grant edit, auto/bypassPermissions grant admin in write_dir only. New sessions only."},
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
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Agent(AgentOp::Run),
    },
    Branch {
        tool: Some("agent_status"),
        op: "session",
        legacy: Some("agent_status"),
        description: "Get running work, pending permission requests (answer with agent_permit), status (completed, failed, cancelled), error, the model that answered, reply (the agent's last message in the turn that answered your latest prompt), followup (its last message from a later turn the engine started itself, such as a background task ending) and denied actions with reasons. Use once if a watch closes or expires without finished; if running is still true, start a new watch; do not poll. Start a new watch for every background agent_run, including continued sessions.",
        schema: || json!({"type": "object", "properties": {"session_id": {"type": "string", "description": "Linggen session ID"}}, "required": ["session_id"]}),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Agent(AgentOp::Status),
    },
    Branch {
        tool: Some("agent_permit"),
        op: "permit",
        legacy: Some("agent_permit"),
        description: "Answer a permission request from an agent_run session (a permission_request stream event, or agent_status pending). A Bash request whose summary is cut carries the whole text as command: read it before allowing. allow_once runs that one action; allow_session grants its path or folder at the needed level for the rest of that session only; a request marked outward (it publishes, operates another machine, or force-pushes or pushes main) takes allow_once or deny only; deny refuses it. Unanswered requests count as denied after 10 min.",
        schema: || json!({"type": "object", "properties": {
            "session_id": {"type": "string", "description": "Linggen session ID"},
            "request_id": {"type": "string", "description": "The request's id"},
            "decision": {"type": "string", "enum": ["allow_once", "allow_session", "deny"]},
            "scope": {"type": "string", "enum": ["path", "dir"], "description": "allow_session only: the exact target, or its folder (default dir when grantable)"}
        }, "required": ["session_id", "request_id", "decision"]}),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: Some(rules::permit),
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Agent(AgentOp::Permit),
    },
    Branch {
        tool: Some("agent_end"),
        op: "stop",
        legacy: Some("agent_stop"),
        description: "Cancel a session's running turn and subagents.",
        schema: || json!({"type": "object", "properties": {"session_id": {"type": "string", "description": "Linggen session ID"}}, "required": ["session_id"]}),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Agent(AgentOp::Stop),
    },
    Branch {
        tool: None,
        op: "models",
        legacy: Some("agent_models"),
        description: "List configured models and the default; pass a model ID to agent_run for a new session.",
        schema: || json!({"type": "object", "properties": {}}),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Agent(AgentOp::Models),
    },
    // The folded discovery: every model with whether it can be used now.
    Branch {
        tool: Some("agent_status"),
        op: "models",
        legacy: None,
        description: "",
        schema: || json!({"type": "object", "properties": {}}),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Agent(AgentOp::Readiness),
    },
    Branch {
        tool: Some("agent_end"),
        op: "close",
        legacy: Some("agent_session_close"),
        description: "Stop a session; after reviewing and merging, optionally remove its linked worktree. Removal requires a clean worktree with HEAD contained in a remote branch.",
        schema: || json!({"type": "object", "properties": {"session_id": {"type": "string", "description": "Linggen session ID"}, "remove_worktree": {"type": "boolean", "description": "Remove the clean, remote-contained linked worktree after work stops (default false)"}}, "required": ["session_id"]}),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: 0,
        views: Views::LOCAL,
        dispatch: Dispatch::Agent(AgentOp::Close),
    },];

/// Memory's branches and the engine's as one catalogue, or why they cannot
/// be: a name that would decide two things is refused here.
pub(super) fn compose(
    memory_tools: &[ToolDoc],
    memory_branches: &[Branch<&'static str>],
    tools: &[ToolDoc],
    branches: &[Branch<Dispatch>],
) -> Result<Catalog<Dispatch>, String> {
    let all_tools = memory_tools.iter().chain(tools).copied().collect();
    let all_branches = memory_branches
        .iter()
        .map(|b| b.with(Dispatch::Memory(b.dispatch)))
        .chain(branches.iter().copied())
        .collect();
    Catalog::new(all_tools, all_branches)
}

/// The door's catalogue.
pub(crate) fn catalog() -> &'static Catalog<Dispatch> {
    static CATALOG: LazyLock<Catalog<Dispatch>> = LazyLock::new(|| {
        use ling_mem::catalog::memory;
        compose(memory::TOOLS, memory::BRANCHES, TOOLS, BRANCHES)
            .expect("the MCP catalogue is consistent")
    });
    &CATALOG
}

/// A branch the engine's legacy door publishes (memory's legacy tools are
/// memory's own server's).
pub(super) fn on_legacy_door(b: &Branch<Dispatch>) -> bool {
    !matches!(b.dispatch, Dispatch::Memory(_))
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
    use ling_mem::catalog::{memory, View};

    fn names(view: View) -> Vec<String> {
        catalog()
            .tools_list(view)
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    }

    /// The folded door, from the one table: memory 5, agents 4, browser 3.
    #[test]
    fn the_folded_door_is_twelve_tools() {
        assert_eq!(
            names(View::Local),
            [
                "memory_search", "memory_add", "memory_update", "memory_delete", "memory_upkeep",
                "agent_run", "agent_status", "agent_permit", "agent_end",
                "browser_open", "browser_read", "browser_act",
            ]
        );
    }

    /// The legacy door is the engine's own branches; memory's legacy tools
    /// stay memory's server's.
    #[test]
    fn the_legacy_door_is_the_engines_branches() {
        let legacy: Vec<&str> = catalog()
            .legacy()
            .filter(|b| on_legacy_door(b))
            .filter_map(|b| b.legacy)
            .collect();
        assert_eq!(legacy.len(), 24);
        assert!(!legacy.iter().any(|n| memory::catalog().legacy_branch(n).is_some()));
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
        let why = compose(memory::TOOLS, memory::BRANCHES, &tools, BRANCHES)
            .err()
            .expect("a second memory_add is refused");
        assert!(why.contains("memory_add"), "{why}");

        let mut branches = BRANCHES.to_vec();
        let mut legacy = branches[0];
        legacy.tool = None;
        legacy.legacy = Some("memory_search");
        branches.push(legacy);
        let why = compose(memory::TOOLS, memory::BRANCHES, TOOLS, &branches)
            .err()
            .expect("a second legacy memory_search is refused");
        assert!(why.contains("memory_search"), "{why}");

        let mut upkeep = BRANCHES.to_vec();
        let mut again = *upkeep.iter().find(|b| b.op == "dream_run").unwrap();
        again.op = "sweep";
        again.legacy = None;
        upkeep.push(again);
        let why = compose(memory::TOOLS, memory::BRANCHES, TOOLS, &upkeep)
            .err()
            .expect("a second upkeep sweep is refused");
        assert!(why.contains("sweep"), "{why}");
    }
}
