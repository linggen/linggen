//! Linggen's one MCP door: `POST /mcp`, a minimal streamable-HTTP MCP
//! server (stateless JSON-RPC 2.0: `initialize`, `ping`, `tools/list`,
//! `tools/call`) for third-party hosts — Claude Code, Codex, Cursor, … —
//! publishing fifteen tools from one catalogue (`catalog`): memory 5,
//! agents 4, browser 3, apps 1, other agents' memory 1, Linggen's own notes 1.
//!
//! The caller decides the view (`view_of`) and the config the mode
//! (`[memory] outside_agents`); together they are the `Door` (`door_of`):
//! this machine sees all fifteen while Linggen serves outside agents the v1
//! way, and ten under v2 — the five v1 memory tools are not listed and a call
//! of one is refused with where each kind of memory goes (`withheld`). An
//! approved computer sees the five memory tools (no `dream_run`) in either
//! mode, anyone else nothing. `tools/list`, `tools/call` and the server text
//! all read that one door, so what a caller cannot see it cannot call. A tool name
//! published before the tools were folded runs nothing: the caller is told
//! to update the plugin (`UPGRADE_PLUGIN`), as `/mcp/memory` tells an old
//! plugin (`memory_host::routes`).
//!
//! Localhost-only by deployment for the full view; the Origin check below
//! rejects browser pages to prevent DNS-rebinding reaching the loop.

use std::sync::Arc;

use axum::extract::State;
use axum::http::header::{CONNECTION, ORIGIN};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::config::OutsideAgents;
use crate::server::bridge::BridgeHub;
use crate::server::ServerState;

mod app;
mod catalog;
mod dispatch;
mod notes;
mod outside;

use catalog::{catalog, Render};
use ling_mem::catalog::{Door, Mode, Resolved, Retired, View};

const PROTOCOL_VERSION: &str = "2025-06-18";

// What an old plugin is told, wherever it knocks: one sentence it can act on.
use crate::memory_host::UPGRADE_PLUGIN as UPGRADE;

// The server text's pieces, written once: a macro because `concat!` takes
// literals, and the two local texts below share most of them. The locked
// fingerprints (tests) hold the v1 text word for word.
macro_rules! memory_rules {
    () => {
        "Linggen memory works without an account. Save durable facts about the person, not project internals or secrets.\nCore is tiny and always loaded: stable identity (name, role, location, languages, family). Semantic is durable and recalled on demand: preferences, decisions and why. Episodic is default staging for uncertain signal and events; no search first. Dream promotes what lasts.\nBefore saving core or semantic, search the subject. Mark the user's words from=user. Merge your own derived rows atomically with memory_add + replace_ids; never add then delete. Before replacing or rewriting a user row, show its full text and date and ask; set user_directed:true only after their answer or an explicit settled instruction. Keep tier and scope unless directed; supersede old status in the same write. Cite recalled facts you use."
    };
}
macro_rules! delegation {
    () => {
        "For each background agent_run, including a follow-up, start a new background watcher at the returned /api/agent/wait WebSocket URL. Answer permission_request with agent_permit: allow_once, allow_session (path/folder for this session), or deny. Outward requests take allow_once or deny only; the deny floor cannot be granted. No reply in 10 minutes means deny. Read reply, followup, error and cannot_run from finished. If the watch ends first, call agent_status (op=session) once; if still running, start a new watch. Do not poll. Check pending and denied. Continue with session_id without widening grants."
    };
}
macro_rules! usage_limit {
    () => {
        "On a usage limit, read its reset time (cannot_run.resets_at), then pick a ready model from agent_status (op=models) or come back after it; never retry before."
    };
}
macro_rules! browser {
    () => {
        "Browser tools control one visible Chrome tab; actions can prompt for per-site permission in Chrome. Read again after page changes."
    };
}
macro_rules! apps {
    () => {
        "The app tool uses what the person's apps do on this Mac, such as playing music: app op=list, then op=call. A private app stays closed until the owner opens it in Settings."
    };
}

/// The server text this machine's hosts read in v1: memory's rules,
/// delegation's, the usage limit, the browser, the apps. At most 2,048
/// characters — Claude Code drops the rest of a server's instructions
/// without a word.
const LOCAL_TEXT: &str = concat!(
    memory_rules!(),
    "\n",
    delegation!(),
    "\n",
    usage_limit!(),
    "\n",
    browser!(),
    " For memory upkeep use the dream, scan or solve runbook; memory_upkeep (op=dream_status) shows backlog and failures.\n",
    apps!()
);

/// The same for v2 (`[memory] outside_agents = "v2"`): no rule to save into
/// the v1 store and no upkeep — the door offers neither. What the memory
/// part says, by scope: about the person and true everywhere → `memory_notes`
/// (person); project things → the agent's own memory; and to read another
/// agent's memory, its file first and `memory_agents` when the file is out
/// of reach. The rest as in v1.
const LOCAL_TEXT_V2: &str = concat!(
    "Claude Code and Codex each keep their own memory; Linggen only reads it and never edits it. Facts about the person that hold everywhere (language, how to address them, long-standing preferences): memory_notes op=write, scope person. Project things (code conventions, decisions with their why, gotchas): your own memory, not Linggen's; a host with no memory of its own may keep them in memory_notes, scope project. To read another agent's memory, open its file yourself when you can; use memory_agents (op=index lists what they remember for a project, op=read opens a note, op=search finds words) only when you can't: in a sandbox, or for another machine. Where two disagree, the newer wins. Read Linggen's own notes with memory_notes op=index, read, search.\n",
    delegation!(),
    "\n",
    usage_limit!(),
    "\n",
    browser!(),
    "\n",
    apps!()
);

/// The server text an approved computer reads: memory only, its upkeep
/// with the host's own model.
const REMOTE_TEXT: &str = "Linggen memory works without an account. Save durable facts about the person, not project internals or secrets.
Core is tiny and always loaded: stable identity (name, role, location, languages, family). Semantic is durable and recalled on demand: preferences, decisions and why. Episodic is default staging for uncertain signal and events; no search first. Dream promotes what lasts.
Before saving core or semantic, search the subject. Mark the user's words from=user. Merge your own derived rows atomically with memory_add + replace_ids; never add then delete. Before replacing or rewriting a user row, show its full text and date and ask; set user_directed:true only after their answer or an explicit settled instruction. Keep tier and scope unless directed; supersede old status in the same write. Cite recalled facts you use.
For memory upkeep use the dream, scan or solve runbook with your own model; memory_upkeep (op=dream_status) shows backlog and failures.";

/// The door a caller meets: its view, and the way this Linggen serves
/// outside agents right now. Made here and nowhere else; `tools/list`,
/// `tools/call` and the server text all read this one value, so what the
/// door does not list it cannot call, and its text never names it.
fn door_of(view: View, outside: OutsideAgents) -> Door {
    let mode = match outside {
        OutsideAgents::V1 => Mode::V1,
        OutsideAgents::V2 => Mode::V2,
    };
    Door { view, mode }
}

/// The server text a caller reads, picked by its door. The one place.
fn server_text(door: Door) -> &'static str {
    match (door.view, door.mode) {
        (View::Local, Mode::V1) => LOCAL_TEXT,
        (View::Local, Mode::V2) => LOCAL_TEXT_V2,
        (View::Remote, _) => REMOTE_TEXT,
    }
}

/// How this Linggen serves agents outside it right now (`[memory]
/// outside_agents`): read here, for the server text and for the
/// `memory_agents` tool, and nowhere else. No daemon state (a unit test): v1.
async fn outside_mode(deps: &McpDeps<'_>) -> OutsideAgents {
    match deps.state {
        Some(state) => state.manager.get_config_snapshot().await.memory.outside_agents,
        // No engine state is a unit test's door: the old text its facts are checked against.
        None => OutsideAgents::V1,
    }
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize_result(door: Door) -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "linggen",
            "title": "Linggen",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": server_text(door),
    })
}

/// A tool result: plain text, an image, or a tool-level error (`isError`).
fn tool_content(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

/// Render a successful bridge payload for the calling agent.
fn render_data(render: Render, data: &Value) -> Value {
    match render {
        Render::Image => {
            let base64 = data
                .get("base64")
                .and_then(Value::as_str)
                .unwrap_or_default();
            json!({
                "content": [{ "type": "image", "data": base64, "mimeType": "image/png" }],
                "isError": false
            })
        }
        Render::Page => {
            let url = data.get("url").and_then(Value::as_str).unwrap_or_default();
            let title = data
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let tree = data.get("tree").and_then(Value::as_str).unwrap_or_default();
            tool_content(format!("{url} — \"{title}\"\n\n{tree}"), false)
        }
        Render::Text => tool_content(data.to_string(), false),
    }
}

/// What a tool dispatch can reach. `state` is present in production
/// (post_handler) and None in unit tests that only exercise bridge/memory
/// shape — agent_run needs it and errors cleanly without it.
struct McpDeps<'a> {
    bridge: &'a BridgeHub,
    state: Option<&'a Arc<ServerState>>,
    read_origin: crate::engine::permission::ReadOrigin,
    /// The caller is the owner at this Mac itself — not a device, not a
    /// tunnelled peer: what the `app` tool asks before it looks at an app.
    at_this_mac: bool,
}

/// The view a caller gets: this machine everything, an approved computer
/// memory only, anyone else nothing.
fn view_of(caller: &crate::memory_host::caller::Caller) -> Option<View> {
    use crate::memory_host::caller::Caller;
    match caller {
        Caller::Owner => Some(View::Local),
        Caller::Machine(_) => Some(View::Remote),
        Caller::Phone(_) | Caller::Unknown => None,
    }
}

fn tools_list_result(door: impl Into<Door>) -> Value {
    json!({ "tools": catalog().tools_list(door) })
}

/// `tools/call`: resolved against the branches `view` can call — the same
/// filter as its `tools/list` — checked, then run by the branch's
/// dispatcher. A refusal is a tool error carrying the form that works; a
/// retired name, or a call without the `op` its tool needs (an old
/// plugin's shape), is told to update; a tool the view cannot see is
/// unknown.
async fn call_tool(
    deps: &McpDeps<'_>,
    door: impl Into<Door>,
    name: &str,
    args: Value,
) -> Result<Value, String> {
    let door = door.into();
    let sent_op = args.get("op").is_some();
    match catalog().resolve(door, name, args) {
        Resolved::Unknown => match catalog().retired_name(name) {
            Some(retired) => Ok(upgrade(retired)),
            None if catalog().withheld_by_mode(door, name) => Ok(tool_content(withheld(name), true)),
            None => Err(format!("unknown tool: {name}")),
        },
        Resolved::Refused(why) if !sent_op && needs_op(door, name) => Ok(old_shape(name, &why)),
        Resolved::Refused(why) => Ok(tool_content(why, true)),
        Resolved::Call { branch, args } => Ok(dispatch::run(deps, branch, args).await),
    }
}

/// Whether tool `name` has more than one branch in `view` and no implicit
/// one: a call must name its `op`.
fn needs_op(door: Door, name: &str) -> bool {
    let implicit = catalog()
        .tools()
        .iter()
        .any(|t| t.name == name && t.implicit_op.is_some());
    !implicit && catalog().visible(door, name).len() > 1
}

/// The answer to a call of a tool this Linggen withholds while it serves
/// outside agents the v2 way: nothing ran, and the caller is told where
/// each kind of memory goes instead.
fn withheld(name: &str) -> String {
    format!(
        "{name} is not offered: this Linggen is on memory v2, where each agent keeps its own memory and Linggen only reads it. \
Facts about the person that hold everywhere (language, how to address them, standing preferences): memory_notes op=write, scope person. \
Project things (code conventions, decisions, gotchas): your own memory."
    )
}

/// The answer to a retired tool name: nothing ran; update the plugin.
fn upgrade(retired: &Retired) -> Value {
    let text = format!(
        "`{}` is a retired Linggen tool and nothing ran. {UPGRADE} (Its work is now {}.)",
        retired.name, retired.now
    );
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": true,
        "structuredContent": { "code": "UPGRADE_PLUGIN", "retired": retired.name, "now": retired.now },
    })
}

/// The answer to an old plugin's shape on a current name: nothing ran;
/// update the plugin, or send the form that works.
fn old_shape(name: &str, why: &str) -> Value {
    let text = format!(
        "{name} was called in an older shape and nothing ran. If a plugin sent this, update the Linggen plugin and reconnect; otherwise: {why}"
    );
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": true,
        "structuredContent": { "code": "UPGRADE_PLUGIN", "tool": name },
    })
}

/// One status object answering "is memory upkeep due, and did the last
/// run succeed?" — daemon days rollup (undreamed worklist + open issues)
/// merged with the engine's own run state. Engine-side failures are
/// carried as `last_run_error` so MCP callers can show the user WHY a
/// dream failed (model not configured, sign-in expired, quota) instead
/// of a silent dead pipeline.
async fn compose_dream_status(state: &Arc<ServerState>) -> anyhow::Result<Value> {
    let rollup = crate::engine::tools::memory_http::call_memory(
        ling_mem::Who::Owner,
        "memory_dream_status",
        json!({ "verb": "days", "undreamed_only": true }),
    )
    .await?;

    let undreamed: Vec<Value> = rollup
        .get("days")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let first_undreamed = rollup
        .get("first_undreamed")
        .cloned()
        .unwrap_or(Value::Null);
    let first_unscanned = rollup
        .get("first_unscanned")
        .cloned()
        .unwrap_or(Value::Null);
    let open_issues = rollup.get("open_issues").cloned().unwrap_or(json!(0));

    let in_flight = crate::extensions::missions::scheduler::mission_in_flight("dream");
    let last_run = state
        .missions
        .list_mission_runs_paginated("dream", Some(1), None)
        .ok()
        .and_then(|runs| runs.into_iter().next());

    // For a failed run, the error its record kept; a record from before
    // errors were kept falls back to the session's last line — where the
    // engine said what broke ("Consolidation failed: …", provider errors).
    let last_run_error = match &last_run {
        Some(run) if run.status == "failed" && run.error.is_some() => {
            run.error.clone().map(Value::String)
        }
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
    // Why a failed run could not go on, as its record typed it — a field
    // of its own beside the words of `last_run_error`. Older records have
    // none.
    let last_run_cannot_run = last_run
        .as_ref()
        .filter(|run| run.status == "failed")
        .and_then(|run| run.cannot_run.as_ref())
        .map(|why| why.dream_fact());

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
        "last_run": last_run.as_ref().map(last_run_json).unwrap_or(Value::Null),
        "last_run_error": last_run_error.unwrap_or(Value::Null),
        "last_run_cannot_run": last_run_cannot_run.unwrap_or(Value::Null),
    }))
}

/// A dream run as `memory_dream_status` reports it: its outcome and the
/// tool calls that failed in it. A record from before failures were kept
/// reports none.
fn last_run_json(run: &crate::engine::mission::record::MissionRunEntry) -> Value {
    json!({
        "status": run.status,
        "triggered_at": run.triggered_at,
        "session_id": run.session_id,
        "failure_count": run.failure_count,
        "failures": run.failures,
        "hit_limit": run.hit_limit,
    })
}

/// Handle one JSON-RPC message for a caller with `view`, at the door this
/// Linggen's config makes of it. `None` means a notification.
async fn handle_rpc(deps: &McpDeps<'_>, view: View, msg: &Value) -> Option<Value> {
    handle_door(deps, door_of(view, outside_mode(deps).await), msg).await
}

/// The same for a caller at `door`.
async fn handle_door(deps: &McpDeps<'_>, door: Door, msg: &Value) -> Option<Value> {
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let id = msg.get("id").cloned();
    if id.is_none() || method.starts_with("notifications/") {
        return None;
    }
    let id = id.unwrap();
    let response = match method {
        "initialize" => rpc_result(id, initialize_result(door)),
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(id, tools_list_result(door)),
        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or_default();
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match call_tool(deps, door, name, args).await {
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
            .map(super::loopback_guard::origin_is_loopback_page)
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
/// An approved computer's request is its presence, as memory's device list
/// shows it.
pub(crate) async fn post_handler(
    State(state): State<Arc<ServerState>>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    use crate::engine::permission::ReadStart;
    use crate::memory_host::caller::{self, Caller};
    if !origin_allowed(&headers) {
        return one_shot((StatusCode::FORBIDDEN, "origin not allowed").into_response());
    }
    let caller = caller::resolve(peer.ip(), &headers);
    let Some(view) = view_of(&caller) else {
        return one_shot((StatusCode::FORBIDDEN, "this door is for computers").into_response());
    };
    if let Caller::Machine(device) = &caller {
        crate::memory_host::note_machine(&device.id, peer.ip(), "/mcp");
    }
    let at_this_mac = matches!(caller, Caller::Owner)
        && crate::server::api::ling_mem_connect::is_at_this_mac(peer, &headers)
        && caller::token_from_headers(&headers).is_none();
    let deps = McpDeps {
        bridge: &state.bridge,
        state: Some(&state),
        read_origin: if at_this_mac { ReadStart::HostAtThisMac } else { ReadStart::NoStanding }.standing(),
        at_this_mac,
    };
    match handle_rpc(&deps, view, &body).await {
        Some(response) => {
            note_old_plugin(&headers, &response);
            one_shot(Json(response).into_response())
        }
        None => one_shot(StatusCode::ACCEPTED.into_response()),
    }
}

/// A retired tool name answered "update the plugin": the host that called
/// it is noted (`perception::old_plugin`), so the person hears of it.
fn note_old_plugin(headers: &HeaderMap, response: &Value) {
    let Some(retired) = response["result"]["structuredContent"]["retired"].as_str() else {
        return;
    };
    let client = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok());
    let host = crate::perception::old_plugin::host_of(client);
    crate::perception::old_plugin::note(host, retired);
}

/// `GET /mcp` — this server never pushes; clients poll nothing.
pub(crate) async fn get_handler() -> Response {
    one_shot(StatusCode::METHOD_NOT_ALLOWED.into_response())
}

/// The input schema tool `name` is published with to this machine.
#[cfg(test)]
pub(crate) fn tool_schema(name: &str) -> Option<Value> {
    catalog()
        .tools_list(View::Local)
        .into_iter()
        .find(|t| t["name"] == name)
        .map(|t| t["inputSchema"].clone())
}

/// The v1 text of each view: what every host that has not been moved to v2
/// reads, and what the locked fingerprints hold.
#[cfg(test)]
fn instructions(view: View) -> &'static str {
    server_text(door_of(view, OutsideAgents::V1))
}

#[cfg(test)]
mod folded_tests;

#[cfg(test)]
mod repo_check;

#[cfg(test)]
mod tests;
