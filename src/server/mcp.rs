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
use axum::http::{header::ORIGIN, HeaderMap, StatusCode};
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

/// Where a tool's call goes: over the browser bridge, or to the ling-mem
/// daemon (the memory engine) via the engine's HTTP client path.
enum Backend {
    Bridge { module: &'static str, op: &'static str },
    Memory { verb: &'static str },
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
    // --- memory_*: the user's durable cross-host memory (ling-mem) ---------
    // Thin proxy to the ling-mem daemon; names and shapes mirror ling-mem's
    // own MCP so migrating users keep muscle memory. Dream-pipeline verbs
    // (harvest/remember/sweep/chains/days) stay engine-internal.
    McpTool {
        name: "memory_search",
        backend: Backend::Memory { verb: "search" },
        description: "Semantic search over the user's durable cross-session memory (identity, preferences, decisions, gotchas). Search before answering questions that could connect to past preferences or decisions; cite rows you use (\"From memory: …\").",
        schema: || json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Natural-language description of what you're looking for"},
                "limit": {"type": "integer", "description": "Max rows (default 10)"},
                "tier": {"type": "string", "enum": ["core", "semantic", "episodic"], "description": "Omit to span all tiers"},
                "contexts": {"type": "array", "items": {"type": "string"}, "description": "Filter to these scope tags"}
            },
            "required": ["query"]
        }),
        timeout_ms: 0,
    },
    McpTool {
        name: "memory_add",
        backend: Backend::Memory { verb: "add" },
        description: "Insert a new memory row. Durable, cross-session signal only — identity facts, behavioural preferences, decisions with their reasoning. Uncertain-durability signal goes to tier=episodic (per-turn staging; a nightly pass promotes what lasts). Search first for core/semantic writes.",
        schema: || json!({
            "type": "object",
            "properties": {
                "content": {"type": "string", "description": "The fact text the model will see when this row is recalled"},
                "tier": {"type": "string", "enum": ["core", "semantic", "episodic"], "description": "episodic = per-turn capture staging (default lane); semantic = curated durable pool (search first); core = tiny always-injected universals about the person"},
                "type": {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"]},
                "contexts": {"type": "array", "items": {"type": "string"}, "description": "Scope tags, e.g. a project name"},
                "host": {"type": "string", "description": "Identify the calling host (e.g. claude-code, codex, cursor) for cross-host attribution"},
                "source_session": {"type": "string", "description": "Session id that authored this content"},
                "occurred_at": {"type": "string", "description": "RFC-3339 user-event timestamp; defaults to now"},
                "user_directed": {"type": "boolean", "description": "Assert the user's CURRENT message states this change as SETTLED (a command, declaration, or commitment). Required when replace_ids targets from=user rows — the daemon blocks such writes otherwise. Never assert from your own inference."},
                "replace_ids": {"type": "array", "items": {"type": "string"}, "description": "Row ids this new row replaces — inserted and deleted atomically. Use for conflict resolution; never separate add + delete calls."}
            },
            "required": ["content"]
        }),
        timeout_ms: 0,
    },
    McpTool {
        name: "memory_get",
        backend: Backend::Memory { verb: "get" },
        description: "Fetch one memory row by id.",
        schema: || json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "The row UUID"}
            },
            "required": ["id"]
        }),
        timeout_ms: 0,
    },
    McpTool {
        name: "memory_update",
        backend: Backend::Memory { verb: "update" },
        description: "Edit fields of an existing memory row by id. Reserve for mechanical rephrasing of the same fact; contradictions are resolved with memory_add + replace_ids after asking the user.",
        schema: || json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "The row UUID"},
                "content": {"type": "string"},
                "type": {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"]},
                "tier": {"type": "string", "enum": ["core", "semantic", "episodic"]},
                "contexts": {"type": "array", "items": {"type": "string"}},
                "user_directed": {"type": "boolean", "description": "Required when rewriting a from=user row: the user's current message directs the change"}
            },
            "required": ["id"]
        }),
        timeout_ms: 0,
    },
    McpTool {
        name: "memory_delete",
        backend: Backend::Memory { verb: "delete" },
        description: "Hard-delete ONE memory row by id. Only for explicit user requests to forget (or post-ask conflict resolution). There is no bulk delete on this surface — deleting by type or context is not offered by design.",
        schema: || json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "The row UUID"}
            },
            "required": ["id"]
        }),
        timeout_ms: 0,
    },
    McpTool {
        name: "memory_list",
        backend: Backend::Memory { verb: "list" },
        description: "Filter-only browse of memory rows (no ranking) — for audits and reviews. All filters optional and AND-combined; over-filtering is the #1 cause of empty results.",
        schema: || json!({
            "type": "object",
            "properties": {
                "tier": {"type": "string", "enum": ["core", "semantic", "episodic"], "description": "Omit to span all tiers"},
                "contexts": {"type": "array", "items": {"type": "string"}},
                "type": {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"], "description": "Omit to return all types"},
                "limit": {"type": "integer", "description": "Max rows (default 50)"},
                "offset": {"type": "integer"},
                "sort": {"type": "string", "enum": ["newest", "oldest"]}
            }
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
            extension is not connected. memory_* tools read and write the user's \
            durable cross-host memory (three tiers: core = always-on identity \
            universals, semantic = curated long-term facts, episodic = per-turn \
            staging judged nightly). Search memory before answering questions that \
            could connect to past preferences or decisions, and cite rows you use. On \
            writes, pass source_session and host; capture uncertain-durability signal \
            to tier=episodic. Replacing or rewriting a row the user authored requires \
            user_directed:true grounded in their current message (with replace_ids \
            for an atomic swap) — the daemon blocks it otherwise. Delete only by id."
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
        Backend::Memory { .. } => "",
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

async fn call_tool(
    hub: &BridgeHub,
    ling_mem_url: &str,
    name: &str,
    args: Value,
) -> Result<Value, String> {
    let Some(tool) = TOOLS.iter().find(|t| t.name == name) else {
        return Err(format!("unknown tool: {name}"));
    };
    match tool.backend {
        Backend::Bridge { module, op } => {
            let res = hub.call_value(module, op, args, tool.timeout_ms).await;
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
        Backend::Memory { verb } => {
            let mut args = args;
            if let Some(obj) = args.as_object_mut() {
                obj.insert("verb".to_string(), json!(verb));
                // Attribute writes to some host even when the caller forgot —
                // "mcp" beats a misleading default and never masks a real one.
                if verb == "add" && !obj.contains_key("host") {
                    obj.insert("host".to_string(), json!("mcp"));
                }
            }
            // The engine's ling-mem client path: episodic wire translation,
            // soft-empty cleanup, and first-use autostart all come with it.
            // The daemon itself enforces the user-voice merge floor.
            match crate::engine::tools::memory_tool::call_memory_http(ling_mem_url, tool.name, args)
                .await
            {
                Ok(value) => Ok(tool_content(value.to_string(), false)),
                Err(e) => Ok(tool_content(format!("{e:#}"), true)),
            }
        }
    }
}

/// Handle one JSON-RPC message. `None` means a notification (no response).
async fn handle_rpc(hub: &BridgeHub, ling_mem_url: &str, msg: &Value) -> Option<Value> {
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
            match call_tool(hub, ling_mem_url, name, args).await {
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

/// `POST /mcp` — the streamable-HTTP MCP endpoint (stateless, JSON responses).
pub(crate) async fn post_handler(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !origin_allowed(&headers) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    let ling_mem_url = state.manager.get_config_snapshot().await.agent.ling_mem_url;
    match handle_rpc(&state.bridge, &ling_mem_url, &body).await {
        Some(response) => Json(response).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// `GET /mcp` — this server never pushes; clients poll nothing.
pub(crate) async fn get_handler() -> Response {
    StatusCode::METHOD_NOT_ALLOWED.into_response()
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

    #[tokio::test]
    async fn initialize_reports_tools_capability() {
        let msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} });
        let res = handle_rpc(&hub(), TEST_MEM_URL, &msg).await.unwrap();
        assert_eq!(res["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(res["result"]["serverInfo"]["name"], "linggen");
        assert!(res["result"]["capabilities"]["tools"].is_object());
    }

    #[tokio::test]
    async fn tools_list_mirrors_control_x_and_memory_ops() {
        let msg = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let res = handle_rpc(&hub(), TEST_MEM_URL, &msg).await.unwrap();
        let tools = res["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 21);
        assert!(tools.iter().any(|t| t["name"] == "browser_navigate"));
        assert!(tools.iter().any(|t| t["name"] == "x_search"));
        assert!(tools.iter().any(|t| t["name"] == "memory_search"));
        assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
        // Delete is by-id only — no bulk filters on the destructive surface.
        let del = tools.iter().find(|t| t["name"] == "memory_delete").unwrap();
        assert_eq!(del["inputSchema"]["required"], json!(["id"]));
        assert!(del["inputSchema"]["properties"]["type"].is_null());
        assert!(del["inputSchema"]["properties"]["contexts"].is_null());
    }

    #[tokio::test]
    async fn notifications_get_no_response() {
        let msg = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle_rpc(&hub(), TEST_MEM_URL, &msg).await.is_none());
    }

    #[tokio::test]
    async fn unknown_method_is_rpc_error() {
        let msg = json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/list" });
        let res = handle_rpc(&hub(), TEST_MEM_URL, &msg).await.unwrap();
        assert_eq!(res["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn unknown_tool_is_invalid_params() {
        let msg = json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "browser_fly", "arguments": {} }
        });
        let res = handle_rpc(&hub(), TEST_MEM_URL, &msg).await.unwrap();
        assert_eq!(res["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn call_without_bridge_is_tool_error_not_rpc_error() {
        let msg = json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": { "name": "browser_tabs", "arguments": { "action": "list" } }
        });
        let res = handle_rpc(&hub(), TEST_MEM_URL, &msg).await.unwrap();
        assert_eq!(res["result"]["isError"], true);
        let text = res["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("not connected"));
    }

    #[test]
    fn web_origins_must_be_loopback() {
        let mut headers = HeaderMap::new();
        assert!(origin_allowed(&headers));
        headers.insert(ORIGIN, "http://127.0.0.1:9898".parse().unwrap());
        assert!(origin_allowed(&headers));
        headers.insert(ORIGIN, "https://evil.example".parse().unwrap());
        assert!(!origin_allowed(&headers));
    }
}
