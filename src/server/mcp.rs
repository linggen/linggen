//! MCP front door — the v2 surface of `doc/browser-control-spec.md`.
//!
//! A minimal streamable-HTTP MCP server at `POST /mcp` so third-party agents
//! (Claude Code, Cursor, Codex…) can drive the user's browser through the
//! same bridge the native `Browser_*` tools use. Stateless JSON-RPC 2.0:
//! `initialize`, `tools/list`, and `tools/call` are the whole protocol here —
//! each `tools/call` brokers one `control` op over `BridgeHub`, exactly like
//! `POST /api/bridge/call` but speaking MCP.
//!
//! Localhost-only by deployment (the daemon binds loopback); the Origin check
//! below rejects browser pages to prevent DNS-rebinding reaching the loop.

use std::sync::Arc;

use axum::extract::State;
use axum::http::header::{CONNECTION, ORIGIN};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::server::bridge::BridgeHub;
use crate::server::ServerState;

const PROTOCOL_VERSION: &str = "2025-06-18";
const CALL_TIMEOUT_MS: u64 = 20_000;
const NAVIGATE_TIMEOUT_MS: u64 = 45_000;
/// Mutating ops wait on the extension's permission prompt (a human, up to
/// 120s) before acting.
const GATED_TIMEOUT_MS: u64 = 150_000;
/// Session reads open a hidden tab and wait for the site's own responses.
const READ_MODULE_TIMEOUT_MS: u64 = 60_000;

/// Where a tool's call goes: over the browser bridge, to the ling-mem
/// daemon (the memory engine) via the engine's HTTP client path, or to
/// the engine's own mission machinery (the dream tools).
enum Backend {
    Bridge { module: &'static str, op: &'static str },
    Agent,
    /// Composed read: daemon days rollup + engine in-flight/run state.
    DreamStatus,
    /// Trigger the dream mission through `trigger_mission_core` — the
    /// same guarded path the HTTP trigger and the calendar use.
    DreamRun,
}


/// One MCP tool: its wire name, the backend it brokers to, its schema.
/// `timeout_ms` applies to bridge calls; memory calls carry their own.
struct McpTool {
    name: &'static str,
    backend: Backend,
    description: &'static str,
    schema: fn() -> Value,
    timeout_ms: u64,
}

/// The tool table — control ops one-to-one, the x session-read ops, and the
/// memory_* group proxying ling-mem.
const TOOLS: &[McpTool] = &[
    McpTool {
        name: "browser_navigate",
        backend: Backend::Bridge { module: "control", op: "navigate" },
        description: "Load a URL (or go \"back\"/\"forward\") in the controlled browser tab. The tab is visible to the user. Follow with browser_read_page to see the result.",
        schema: || json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Absolute URL to load, or \"back\" / \"forward\""}
            },
            "required": ["url"]
        }),
        timeout_ms: GATED_TIMEOUT_MS,
    },
    McpTool {
        name: "browser_read_page",
        backend: Backend::Bridge { module: "control", op: "read_page" },
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
        timeout_ms: CALL_TIMEOUT_MS,
    },
    McpTool {
        name: "browser_click",
        backend: Backend::Bridge { module: "control", op: "click" },
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
        timeout_ms: GATED_TIMEOUT_MS,
    },
    McpTool {
        name: "browser_type",
        backend: Backend::Bridge { module: "control", op: "type" },
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
        timeout_ms: GATED_TIMEOUT_MS,
    },
    McpTool {
        name: "browser_key",
        backend: Backend::Bridge { module: "control", op: "key" },
        description: "Press a key or chord in the controlled tab, e.g. \"Enter\", \"Escape\", \"Ctrl+a\", \"Meta+Enter\".",
        schema: || json!({
            "type": "object",
            "properties": {
                "keys": {"type": "string", "description": "Key or chord, e.g. \"Enter\", \"Tab\", \"Ctrl+a\""},
                "repeat": {"type": "integer", "description": "Press count (default 1, max 20)"}
            },
            "required": ["keys"]
        }),
        timeout_ms: GATED_TIMEOUT_MS,
    },
    McpTool {
        name: "browser_scroll",
        backend: Backend::Bridge { module: "control", op: "scroll" },
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
        timeout_ms: CALL_TIMEOUT_MS,
    },
    McpTool {
        name: "browser_screenshot",
        backend: Backend::Bridge { module: "control", op: "screenshot" },
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
        timeout_ms: CALL_TIMEOUT_MS,
    },
    McpTool {
        name: "browser_wait",
        backend: Backend::Bridge { module: "control", op: "wait" },
        description: "Wait for the controlled tab to settle before the next read: for \"load\" (page load), \"selector\" (a CSS selector appears, value required), or \"ms\" (fixed delay, max 10000).",
        schema: || json!({
            "type": "object",
            "properties": {
                "for": {"type": "string", "enum": ["load", "selector", "ms"]},
                "value": {"type": "string", "description": "CSS selector (for=selector) or milliseconds (for=ms)"}
            },
            "required": ["for"]
        }),
        timeout_ms: NAVIGATE_TIMEOUT_MS,
    },
    McpTool {
        name: "browser_tabs",
        backend: Backend::Bridge { module: "control", op: "tabs" },
        description: "Manage the controlled tab: list (current state), open (a URL, creating the tab if needed), switch (bring it to front), close.",
        schema: || json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "open", "switch", "close"]},
                "url": {"type": "string", "description": "URL for action=open"}
            },
            "required": ["action"]
        }),
        timeout_ms: GATED_TIMEOUT_MS,
    },
    McpTool {
        name: "browser_read_console",
        backend: Backend::Bridge { module: "control", op: "read_console" },
        description: "Read recent console messages from the controlled tab (debugging).",
        schema: || json!({
            "type": "object",
            "properties": {
                "limit": {"type": "integer", "description": "Max messages (default 50)"}
            }
        }),
        timeout_ms: CALL_TIMEOUT_MS,
    },
    // --- x session reads: structured data from the user's logged-in x.com ---
    // Each opens a hidden tab and captures X's own responses; no paid API.
    McpTool {
        name: "x_search",
        backend: Backend::Bridge { module: "x", op: "search" },
        description: "Search x.com (Twitter) through the user's logged-in session — returns structured posts (author, text, engagement) as JSON, no API keys. Requires being signed in to x.com in the browser.",
        schema: || json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "X search query (operators like from:, -filter:replies work)"},
                "max": {"type": "integer", "description": "Max posts (default 15, max 100)"}
            },
            "required": ["query"]
        }),
        timeout_ms: READ_MODULE_TIMEOUT_MS,
    },
    McpTool {
        name: "x_targets",
        backend: Backend::Bridge { module: "x", op: "targets" },
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
        timeout_ms: READ_MODULE_TIMEOUT_MS,
    },
    McpTool {
        name: "x_following",
        backend: Backend::Bridge { module: "x", op: "following" },
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
        timeout_ms: READ_MODULE_TIMEOUT_MS,
    },
    McpTool {
        name: "x_whotofollow",
        backend: Backend::Bridge { module: "x", op: "whotofollow" },
        description: "X's personalized \"Who to follow\" recommendations for the signed-in user.",
        schema: || json!({
            "type": "object",
            "properties": {
                "exclude": {"type": "array", "items": {"type": "string"}, "description": "Handles to drop from results"},
                "self": {"type": "string", "description": "The user's own handle, excluded from results"},
                "max": {"type": "integer", "description": "Max accounts (default 60, max 100)"}
            }
        }),
        timeout_ms: READ_MODULE_TIMEOUT_MS,
    },
    McpTool {
        name: "x_own",
        backend: Backend::Bridge { module: "x", op: "own" },
        description: "The signed-in user's own recent x.com posts with engagement (views, likes), plus the parent posts they already replied to.",
        schema: || json!({
            "type": "object",
            "properties": {
                "handle": {"type": "string", "description": "The user's x.com handle"},
                "max": {"type": "integer", "description": "Max posts (default 10, max 100)"}
            },
            "required": ["handle"]
        }),
        timeout_ms: READ_MODULE_TIMEOUT_MS,
    },
    // --- memory: served by ling-mem, NOT here -------------------------------
    //
    // The `memory_*` proxies lived here from 1.4.0 (2026-07-10), when
    // `mcp-spec.md` chose one front door for every channel. Phase 2 of
    // `mcp-client-spec.md` reversed that and they were cut on 2026-07-30:
    // the tools come from ling-mem, so ling-mem is where they are served,
    // and a host that wants memory adds that server (the plugin ships both).
    //
    // Cut rather than left through a longer window because the plugin was the
    // only channel that wired this door for memory, and it migrates its users
    // in the same release. Leaving both would put two `memory_search` tools
    // with identical schemas in front of one model — the duplication this
    // whole arc exists to remove, self-inflicted.
    //
    // `memory_dream_status` / `memory_dream_run` below are NOT memory-server
    // tools and stay: dream_status composes ling-mem's rollup with the
    // engine's in-flight run state, and dream_run drives the engine's mission
    // executor. ling-mem can serve neither.
    // --- dream: the nightly memory pipeline + its review queue --------------
    // Status/run wrap the engine's single mission executor (one set of
    // guards: in-flight, snapshot, run record). Issues proxy the daemon's
    // review-queue sidecar — the audit stage queues what it can't solve
    // with confidence; a host agent solves items with ITS model.
    McpTool {
        name: "memory_dream_status",
        backend: Backend::DreamStatus,
        description: "Dream-pipeline status — is memory upkeep due? Returns undreamed days (oldest first, each awaiting a dream pass) with first_undreamed / first_unscanned, past-day summary counts (total_days / scanned_days / dreamed_days), the open review-item count, whether a dream run is in flight, and the last run's outcome. If last_run_error is set, surface it to the user verbatim — the engine side may need attention (model not configured, sign-in required, quota). When days await a dream, offer to run it: /linggen:dream runs the pass with YOUR model (no Linggen model needed); memory_dream_run offloads it to the local Linggen engine. When open_issues > 0, offer /linggen:solve.",
        schema: || json!({
            "type": "object",
            "properties": {}
        }),
        timeout_ms: 0,
    },
    McpTool {
        name: "memory_dream_run",
        backend: Backend::DreamRun,
        description: "Run the nightly memory dream on the local Linggen engine (its mission executor and configured model — prefer /linggen:dream to run the pass with YOUR model instead). Optional day (YYYY-MM-DD) scopes the run to one day; omitted runs the full nightly protocol: undreamed days oldest-first, then sweep, then audit. Returns immediately — dream runs take minutes; poll memory_dream_status, and if it reports a failed run surface last_run_error to the user. An in-flight run returns {in_flight: true} — that's state, not an error.",
        schema: || json!({
            "type": "object",
            "properties": {
                "day": {"type": "string", "description": "Optional target day, YYYY-MM-DD — one-day scoped run (the calendar shape)"}
            }
        }),
        timeout_ms: 0,
    },
    McpTool {
        name: "agent_run",
        backend: Backend::Agent,
        description: "Delegate a task to a local Linggen agent — one with this machine's skills, memory, and configured models — and get its final answer back. Runs headless in a fresh session, safe-by-default (read-only on the workspace, non-interactive; it can read/search/analyze, use the user's memory, and drive the browser, but not silently write files). Use for research, analysis, memory questions, or browser tasks you want handled by the user's own agent.",
        schema: || json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string", "description": "The task or question for the agent"},
                "agent": {"type": "string", "description": "Agent to run (default: ling). Unknown names return the available list."}
            },
            "required": ["prompt"]
        }),
        timeout_ms: 0,
    },
];

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "linggen",
            "title": "Linggen",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": "Linggen's capability front door. browser_* tools operate the \
            user's own Chrome through the linggen-browser extension: one visible \
            controlled tab — work a loop of browser_read_page (returns [nN] refs) then \
            browser_click / browser_type by ref; mutating actions may pause for the \
            user's permission prompt in the browser. x_* tools return structured data \
            from the user's logged-in x.com session. A no_bridge error means the \
            extension is not connected. Memory is NOT served here — the memory_* \
            proxies were cut on 2026-07-30, and ling-mem serves those tools on its \
            own MCP server (default 127.0.0.1:9528/mcp). Add that server alongside \
            this one for memory_search, memory_add and the rest; it carries the \
            memory protocol in its own instructions. What stays here is upkeep, \
            because it is the engine's rather than the store's: \
            memory_dream_status says whether nightly judgment passes are due \
            (first_undreamed / undreamed_days; first_unscanned flags a day whose \
            session logs were never scanned) and whether review items await the \
            user (open_issues). When days await a dream, offer to run it — /linggen:dream uses YOUR \
            model, memory_dream_run offloads to the Linggen engine; if a run failed, \
            show last_run_error to the user. When open_issues > 0, offer \
            /linggen:solve: read memory_issues, verify each item against the world \
            (git, files), fix via memory_add + replace_ids, close via \
            memory_issue_resolve — ask the user one item at a time when their call \
            is needed."
    })
}

fn tools_list_result() -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": (t.schema)(),
            })
        })
        .collect();
    json!({ "tools": tools })
}

/// A tool result: plain text, an image, or a tool-level error (`isError`).
fn tool_content(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

/// Render a successful payload for the calling agent.
fn render_data(tool: &McpTool, data: &Value) -> Value {
    let op = match tool.backend {
        Backend::Bridge { op, .. } => op,
        Backend::Agent | Backend::DreamStatus | Backend::DreamRun => "",
    };
    match op {
        "screenshot" => {
            let base64 = data.get("base64").and_then(Value::as_str).unwrap_or_default();
            json!({
                "content": [{ "type": "image", "data": base64, "mimeType": "image/png" }],
                "isError": false
            })
        }
        "read_page" => {
            let url = data.get("url").and_then(Value::as_str).unwrap_or_default();
            let title = data.get("title").and_then(Value::as_str).unwrap_or_default();
            let tree = data.get("tree").and_then(Value::as_str).unwrap_or_default();
            tool_content(format!("{url} — \"{title}\"\n\n{tree}"), false)
        }
        _ => tool_content(data.to_string(), false),
    }
}

/// What a tool dispatch can reach. `state` is present in production
/// (post_handler) and None in unit tests that only exercise bridge/memory
/// shape — agent_run needs it and errors cleanly without it.
struct McpDeps<'a> {
    bridge: &'a BridgeHub,
    ling_mem_url: &'a str,
    state: Option<&'a Arc<ServerState>>,
}

async fn call_tool(deps: &McpDeps<'_>, name: &str, args: Value) -> Result<Value, String> {
    let Some(tool) = TOOLS.iter().find(|t| t.name == name) else {
        return Err(format!("unknown tool: {name}"));
    };
    match tool.backend {
        Backend::Bridge { module, op } => {
            let res = deps.bridge.call_value(module, op, args, tool.timeout_ms).await;
            if res.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                let data = res.get("data").cloned().unwrap_or(Value::Null);
                return Ok(render_data(tool, &data));
            }
            let code = res.get("code").and_then(Value::as_str).unwrap_or("upstream_error");
            let message = res.get("message").and_then(Value::as_str).unwrap_or("");
            let text = match code {
                "no_bridge" | "module_unavailable" => "browser not connected — the \
                    linggen-browser extension must be running and connected in Chrome. \
                    Ask the user to install or enable it, then retry."
                    .to_string(),
                "not_permitted" => format!(
                    "not_permitted: {message} — the user declined this action in the \
                    browser's permission prompt."
                ),
                _ => format!("{code}: {message}"),
            };
            Ok(tool_content(text, true))
        }
        Backend::Agent => {
            let Some(state) = deps.state else {
                return Ok(tool_content(
                    "agent_run is unavailable in this context (no daemon state)".to_string(),
                    true,
                ));
            };
            let prompt = args.get("prompt").and_then(Value::as_str).unwrap_or_default();
            let agent = args.get("agent").and_then(Value::as_str);
            match super::mcp_agent::run(state, agent, prompt).await {
                Ok(text) => Ok(tool_content(text, false)),
                Err(msg) => Ok(tool_content(msg, true)),
            }
        }
        Backend::DreamStatus => {
            let Some(state) = deps.state else {
                return Ok(tool_content(
                    "memory_dream_status is unavailable in this context (no daemon state)"
                        .to_string(),
                    true,
                ));
            };
            match compose_dream_status(state, deps.ling_mem_url).await {
                Ok(status) => Ok(tool_content(status.to_string(), false)),
                Err(e) => Ok(tool_content(
                    format!("dream status unavailable — ling-mem daemon unreachable: {e:#}"),
                    true,
                )),
            }
        }
        Backend::DreamRun => {
            let Some(state) = deps.state else {
                return Ok(tool_content(
                    "memory_dream_run is unavailable in this context (no daemon state)".to_string(),
                    true,
                ));
            };
            let day = args.get("day").and_then(Value::as_str).map(str::to_string);
            use crate::server::api::missions::{trigger_mission_core, TriggerOutcome};
            match trigger_mission_core(state, "dream", None, day.clone(), false).await {
                TriggerOutcome::Started { session_id } => Ok(tool_content(
                    json!({
                        "started": true,
                        "session_id": session_id,
                        "day": day,
                        "note": "dream runs take minutes — poll memory_dream_status; if it later reports a failed run, show last_run_error to the user",
                    })
                    .to_string(),
                    false,
                )),
                TriggerOutcome::InFlight => Ok(tool_content(
                    json!({
                        "started": false,
                        "in_flight": true,
                        "note": "a dream run is already in flight — poll memory_dream_status",
                    })
                    .to_string(),
                    false,
                )),
                TriggerOutcome::NotFound => Ok(tool_content(
                    "dream mission not found — the engine's built-in dream mission is missing \
                     (check ~/.linggen/missions/dream or reinstall Linggen)"
                        .to_string(),
                    true,
                )),
                TriggerOutcome::BadDay(d) => Ok(tool_content(
                    format!("invalid day '{d}' — expected YYYY-MM-DD"),
                    true,
                )),
                TriggerOutcome::Internal(e) => {
                    Ok(tool_content(format!("dream trigger failed: {e}"), true))
                }
            }
        }
    }
}

/// One status object answering "is memory upkeep due, and did the last
/// run succeed?" — daemon days rollup (undreamed worklist + open issues)
/// merged with the engine's own run state. Engine-side failures are
/// carried as `last_run_error` so MCP callers can show the user WHY a
/// dream failed (model not configured, sign-in expired, quota) instead
/// of a silent dead pipeline.
async fn compose_dream_status(
    state: &Arc<ServerState>,
    ling_mem_url: &str,
) -> anyhow::Result<Value> {
    let rollup = crate::engine::tools::memory_http::call_memory_http(
        ling_mem_url,
        "memory_dream_status",
        json!({ "verb": "days", "undreamed_only": true }),
    )
    .await?;

    let undreamed: Vec<Value> = rollup
        .get("days")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let first_undreamed = rollup.get("first_undreamed").cloned().unwrap_or(Value::Null);
    let first_unscanned = rollup.get("first_unscanned").cloned().unwrap_or(Value::Null);
    let open_issues = rollup.get("open_issues").cloned().unwrap_or(json!(0));

    let in_flight = crate::extensions::missions::scheduler::mission_in_flight("dream");
    let last_run = state
        .manager
        .missions
        .list_mission_runs_paginated("dream", Some(1), None)
        .ok()
        .and_then(|runs| runs.into_iter().next());

    // For a failed run, pull the session's last line — that's where the
    // engine said what broke ("Consolidation failed: …", provider errors).
    let last_run_error = match &last_run {
        Some(run) if run.status == "failed" => run.session_id.as_deref().and_then(|sid| {
            let messages = state.manager.global_sessions.get_chat_history(sid).ok()?;
            let tail = messages
                .iter()
                .rev()
                .find(|m| !m.is_observation && !m.content.trim().is_empty())?;
            let text: String = tail.content.chars().take(400).collect();
            Some(Value::String(text))
        }),
        _ => None,
    };

    Ok(json!({
        "in_flight": in_flight,
        "undreamed_days": undreamed,
        "first_undreamed": first_undreamed,
        "first_unscanned": first_unscanned,
        "total_days": rollup.get("total_days").cloned().unwrap_or(json!(0)),
        "scanned_days": rollup.get("scanned_days").cloned().unwrap_or(json!(0)),
        "dreamed_days": rollup.get("dreamed_days").cloned().unwrap_or(json!(0)),
        "open_issues": open_issues,
        "today": rollup.get("today").cloned().unwrap_or(Value::Null),
        "last_run": last_run.as_ref().map(|r| json!({
            "status": r.status,
            "triggered_at": r.triggered_at,
            "session_id": r.session_id,
        })).unwrap_or(Value::Null),
        "last_run_error": last_run_error.unwrap_or(Value::Null),
    }))
}

/// Handle one JSON-RPC message. `None` means a notification (no response).
async fn handle_rpc(deps: &McpDeps<'_>, msg: &Value) -> Option<Value> {
    let method = msg.get("method").and_then(Value::as_str).unwrap_or_default();
    let id = msg.get("id").cloned();
    if id.is_none() || method.starts_with("notifications/") {
        return None;
    }
    let id = id.unwrap();
    let response = match method {
        "initialize" => rpc_result(id, initialize_result()),
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(id, tools_list_result()),
        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or_default();
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match call_tool(deps, name, args).await {
                Ok(result) => rpc_result(id, result),
                Err(message) => rpc_error(id, -32602, &message),
            }
        }
        _ => rpc_error(id, -32601, &format!("method not found: {method}")),
    };
    Some(response)
}

/// Same posture as the bridge socket: no Origin (CLI/native clients) is fine;
/// a web page's http(s) Origin must be loopback, so a random site can't drive
/// the browser through a rebound DNS name.
fn origin_allowed(headers: &HeaderMap) -> bool {
    match headers.get(ORIGIN) {
        None => true,
        Some(value) => value
            .to_str()
            .map(|o| {
                o.starts_with("http://127.0.0.1") || o.starts_with("http://localhost")
            })
            .unwrap_or(false),
    }
}

/// Every `/mcp` exchange is one-shot: `Connection: close` keeps client pools
/// from ever reusing a stale socket (observed intermittently with Claude
/// Code's MCP client; loopback reconnect cost is negligible).
fn one_shot(mut resp: Response) -> Response {
    resp.headers_mut()
        .insert(CONNECTION, HeaderValue::from_static("close"));
    resp
}

/// `POST /mcp` — the streamable-HTTP MCP endpoint (stateless, JSON responses).
pub(crate) async fn post_handler(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !origin_allowed(&headers) {
        return one_shot((StatusCode::FORBIDDEN, "origin not allowed").into_response());
    }
    let ling_mem_url = state.manager.get_config_snapshot().await.agent.ling_mem_url;
    let deps = McpDeps {
        bridge: &state.bridge,
        ling_mem_url: &ling_mem_url,
        state: Some(&state),
    };
    match handle_rpc(&deps, &body).await {
        Some(response) => one_shot(Json(response).into_response()),
        None => one_shot(StatusCode::ACCEPTED.into_response()),
    }
}

/// `GET /mcp` — this server never pushes; clients poll nothing.
pub(crate) async fn get_handler() -> Response {
    one_shot(StatusCode::METHOD_NOT_ALLOWED.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unroutable per RFC 5737 — memory tests must never reach a real daemon
    // (the client path would autostart/install ling-mem on connection refusal).
    const TEST_MEM_URL: &str = "http://192.0.2.1:9";

    fn hub() -> BridgeHub {
        BridgeHub::new()
    }

    // No ServerState in unit tests — agent_run errors cleanly, everything
    // else is state-independent.
    fn deps(hub: &BridgeHub) -> McpDeps<'_> {
        McpDeps { bridge: hub, ling_mem_url: TEST_MEM_URL, state: None }
    }

    #[tokio::test]
    async fn initialize_reports_tools_capability() {
        let msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} });
        let res = handle_rpc(&deps(&hub()), &msg).await.unwrap();
        assert_eq!(res["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(res["result"]["serverInfo"]["name"], "linggen");
        assert!(res["result"]["capabilities"]["tools"].is_object());
    }

    #[tokio::test]
    async fn tools_list_is_browser_x_agent_and_the_two_dream_tools() {
        let msg = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let res = handle_rpc(&deps(&hub()), &msg).await.unwrap();
        let tools = res["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 18);
        assert!(tools.iter().any(|t| t["name"] == "browser_navigate"));
        assert!(tools.iter().any(|t| t["name"] == "x_search"));
        assert!(tools.iter().any(|t| t["name"] == "agent_run"));
        assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
    }

    /// Memory is ling-mem's server, and a host that wants it adds that server.
    /// Serving it here too would put two `memory_search` tools with identical
    /// schemas in front of one model — the duplication the MCP arc exists to
    /// remove. This is the test that notices if one comes back.
    #[tokio::test]
    async fn memory_is_not_served_here_beyond_the_two_engine_dream_tools() {
        let msg = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let res = handle_rpc(&deps(&hub()), &msg).await.unwrap();
        let tools = res["result"]["tools"].as_array().unwrap();
        let memory: Vec<&str> = tools
            .iter()
            .filter_map(|t| t["name"].as_str())
            .filter(|n| n.starts_with("memory_"))
            .collect();
        // dream_status composes ling-mem's rollup with the engine's in-flight
        // run state; dream_run drives the mission executor. Neither is
        // something ling-mem could serve, so both stay.
        assert_eq!(memory, ["memory_dream_status", "memory_dream_run"]);
    }

    #[tokio::test]
    async fn notifications_get_no_response() {
        let msg = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle_rpc(&deps(&hub()), &msg).await.is_none());
    }

    #[test]
    fn mcp_responses_disable_keep_alive() {
        let resp = one_shot((StatusCode::OK, "ok").into_response());
        assert_eq!(resp.headers().get(CONNECTION).unwrap(), "close");
    }

    #[tokio::test]
    async fn unknown_method_is_rpc_error() {
        let msg = json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/list" });
        let res = handle_rpc(&deps(&hub()), &msg).await.unwrap();
        assert_eq!(res["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn unknown_tool_is_invalid_params() {
        let msg = json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "browser_fly", "arguments": {} }
        });
        let res = handle_rpc(&deps(&hub()), &msg).await.unwrap();
        assert_eq!(res["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn call_without_bridge_is_tool_error_not_rpc_error() {
        let msg = json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": { "name": "browser_tabs", "arguments": { "action": "list" } }
        });
        let res = handle_rpc(&deps(&hub()), &msg).await.unwrap();
        assert_eq!(res["result"]["isError"], true);
        let text = res["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("not connected"));
    }

    #[tokio::test]
    async fn agent_run_without_state_is_tool_error() {
        let msg = json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": { "name": "agent_run", "arguments": { "prompt": "hi" } }
        });
        let res = handle_rpc(&deps(&hub()), &msg).await.unwrap();
        assert_eq!(res["result"]["isError"], true);
    }

    #[tokio::test]
    async fn dream_tools_without_state_are_tool_errors() {
        for (name, args) in [
            ("memory_dream_status", json!({})),
            ("memory_dream_run", json!({ "day": "2026-07-01" })),
        ] {
            let msg = json!({
                "jsonrpc": "2.0", "id": 8, "method": "tools/call",
                "params": { "name": name, "arguments": args }
            });
            let res = handle_rpc(&deps(&hub()), &msg).await.unwrap();
            assert_eq!(res["result"]["isError"], true, "{name}");
            let text = res["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("unavailable"), "{name}: {text}");
        }
    }

    #[test]
    fn web_origins_must_be_loopback() {
        let mut headers = HeaderMap::new();
        assert!(origin_allowed(&headers));
        headers.insert(ORIGIN, "http://127.0.0.1:9527".parse().unwrap());
        assert!(origin_allowed(&headers));
        headers.insert(ORIGIN, "https://evil.example".parse().unwrap());
        assert!(!origin_allowed(&headers));
    }
}
