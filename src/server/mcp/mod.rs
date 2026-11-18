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

mod catalog;
mod dispatch;

use catalog::{catalog, on_legacy_door, Dispatch, Render};
use ling_mem::catalog::{Branch, Resolved, View};

const PROTOCOL_VERSION: &str = "2025-06-18";
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
        "instructions": "Linggen front door: browser_* controls one visible Chrome tab via linggen-browser; read_page gives refs for click/type. Mutations may need browser permission; no_bridge means disconnected. x_* reads the signed-in X session. Memory tools are at /mcp/memory on this port; add it for memory_search, memory_add and its protocol. memory_dream_status shows undreamed days, first_unscanned, open_issues and failures: offer /linggen:dream (your model) or memory_dream_run (Linggen); show last_run_error. If open_issues > 0 offer /linggen:solve: read memory_issues, verify each against rows, git, code and docs, fix using memory_add + replace_ids, close with memory_issue_resolve. Ask only when evidence cannot settle it or a user-voice row is involved: one plain fact question with recommendation. agent_* delegates to local agents. For code changes create a git worktree and pass its absolute path as write_dir and your permission_mode: plan is read-only, default/acceptEdits grant edit, auto/bypassPermissions grant admin there; other paths remain read-only. Actions beyond grants wait for your permission. Give a self-contained brief with files, cause, tests and short reply; only the final reply returns, work appears in the web UI. If still running, do not poll: start a new background watcher on the returned ws://127.0.0.1:9527/api/agent/wait?session_id=… URL for every agent_run and session_id follow-up. Answer permission_request with agent_permit (allow_once, allow_session for path/folder this session, or deny); unanswered requests are denied after 10 minutes. Read reply and followup from finished; if the watch closes or expires first, call agent_status once and, if still running, start a new watch; never poll. Check denied and act yourself or request a wider write_dir. Follow up with agent_run(session_id=…), omitting write_dir, permission_mode and model; agent_models lists models. Review the diff before merging; agent_session_close with remove_worktree=true removes only a clean linked worktree whose HEAD is on a remote branch."
    })
}

/// The branch this door publishes as legacy tool `name`.
fn legacy_branch(name: &str) -> Option<&'static Branch<Dispatch>> {
    catalog().legacy_branch(name).filter(|b| on_legacy_door(b))
}

/// The input schema one tool is advertised with.
pub(crate) fn tool_schema(name: &str) -> Option<Value> {
    legacy_branch(name).map(|b| (b.schema)())
}

fn tools_list_result() -> Value {
    let tools: Vec<Value> = catalog()
        .legacy()
        .filter(|b| on_legacy_door(b))
        .filter_map(|b| b.legacy_def())
        .collect();
    json!({ "tools": tools })
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
}

/// A legacy call: the branch this door publishes under `name`, run with
/// its arguments as sent.
async fn call_tool(deps: &McpDeps<'_>, name: &str, args: Value) -> Result<Value, String> {
    let Some(branch) = legacy_branch(name) else {
        return Err(format!("unknown tool: {name}"));
    };
    Ok(dispatch::run(deps, branch, args).await)
}

// ── the twelve-tool door ────────────────────────────────────────────────────
//
// Built and tested beside the legacy surface; `/mcp` publishes it when it
// replaces that surface in one switch, with the plugin that calls it.

/// The view a caller gets: this machine everything, an approved computer
/// memory only, anyone else nothing.
#[cfg_attr(not(test), allow(dead_code))]
fn view_of(caller: &crate::memory_host::caller::Caller) -> Option<View> {
    use crate::memory_host::caller::Caller;
    match caller {
        Caller::Owner => Some(View::Local),
        Caller::Machine(_) => Some(View::Remote),
        Caller::Phone(_) | Caller::Unknown => None,
    }
}

/// `tools/list` of the twelve-tool door for `view`.
#[cfg_attr(not(test), allow(dead_code))]
fn folded_tools_list(view: View) -> Value {
    json!({ "tools": catalog().tools_list(view) })
}

/// `tools/call` of the twelve-tool door: resolved against the branches
/// `view` can call — the same filter as its `tools/list` — checked, then
/// run by the branch's dispatcher. A refusal is a tool error carrying the
/// form that works; a tool the view cannot see is unknown.
#[cfg_attr(not(test), allow(dead_code))]
async fn call_folded(
    deps: &McpDeps<'_>,
    view: View,
    name: &str,
    args: Value,
) -> Result<Value, String> {
    match catalog().resolve(view, name, args) {
        Resolved::Unknown => Err(format!("unknown tool: {name}")),
        Resolved::Refused(why) => Ok(tool_content(why, true)),
        Resolved::Call { branch, args } => Ok(dispatch::run(deps, branch, args).await),
    }
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

/// Handle one JSON-RPC message. `None` means a notification (no response).
async fn handle_rpc(deps: &McpDeps<'_>, msg: &Value) -> Option<Value> {
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
        "initialize" => rpc_result(id, initialize_result()),
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(id, tools_list_result()),
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
            .map(super::loopback_guard::origin_is_loopback_page)
            .unwrap_or(false),
    }
}

/// Who may use the front door: this machine and the machines it approved.
/// Its tools act as the owner (an agent started here reads the owner's
/// memory), so a phone — its own person — is not let in.
fn door_open_to(caller: &crate::memory_host::caller::Caller) -> bool {
    caller.is_owner()
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
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !origin_allowed(&headers) {
        return one_shot((StatusCode::FORBIDDEN, "origin not allowed").into_response());
    }
    if !door_open_to(&crate::memory_host::caller::resolve(peer.ip(), &headers)) {
        return one_shot((StatusCode::FORBIDDEN, "this door is for computers").into_response());
    }
    let deps = McpDeps {
        bridge: &state.bridge,
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
mod folded_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn hub() -> BridgeHub {
        BridgeHub::new()
    }

    #[test]
    fn the_dream_status_names_a_last_runs_failed_calls_and_old_rows_read_as_before() {
        let line = r#"{"run_id":"r","triggered_at":1,"status":"completed","skipped":false,"failures":[{"tool":"fetch","kind":"exit","exit":3,"tail":"down"}],"failure_count":1}"#;
        let run: crate::engine::mission::record::MissionRunEntry =
            serde_json::from_str(line).unwrap();
        let v = last_run_json(&run);
        assert_eq!(v["status"], "completed");
        assert_eq!(v["failure_count"], 1);
        assert_eq!(v["failures"][0]["exit"], 3);

        let old = r#"{"run_id":"r","triggered_at":1,"status":"failed","skipped":false}"#;
        let run: crate::engine::mission::record::MissionRunEntry =
            serde_json::from_str(old).unwrap();
        let v = last_run_json(&run);
        assert_eq!(
            (v["status"].clone(), v["failure_count"].clone()),
            (json!("failed"), json!(0))
        );
        assert_eq!(v["failures"], json!([]));
        assert!(run.error.is_none(), "an old row grew an error");
    }

    // No ServerState in unit tests — agent_run errors cleanly, everything
    // else is state-independent.
    fn deps(hub: &BridgeHub) -> McpDeps<'_> {
        McpDeps {
            bridge: hub,
            state: None,
        }
    }

    #[tokio::test]
    async fn initialize_reports_tools_capability() {
        let msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} });
        let res = handle_rpc(&deps(&hub()), &msg).await.unwrap();
        assert_eq!(res["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(res["result"]["serverInfo"]["name"], "linggen");
        assert!(res["result"]["capabilities"]["tools"].is_object());
        let instructions = res["result"]["instructions"].as_str().unwrap();
        assert!(
            instructions.chars().count() <= 2048,
            "instructions: {} characters",
            instructions.chars().count()
        );
        for text in [
            "git worktree",
            "permission_mode",
            "do not poll",
            "agent_session_close",
            "agent_permit",
            "permission_request",
            "finished",
            "if still running, start a new watch",
        ] {
            assert!(instructions.contains(text), "missing {text}");
        }
    }

    #[tokio::test]
    async fn tools_list_is_browser_x_agent_and_the_two_dream_tools() {
        let msg = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let res = handle_rpc(&deps(&hub()), &msg).await.unwrap();
        let tools = res["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 24);
        assert!(tools.iter().any(|t| t["name"] == "browser_navigate"));
        assert!(tools.iter().any(|t| t["name"] == "x_search"));
        assert!(tools.iter().any(|t| t["name"] == "x_mentions"));
        assert!(tools.iter().any(|t| t["name"] == "agent_run"));
        assert!(tools.iter().any(|t| t["name"] == "agent_permit"));
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

    /// JSON with every object's keys sorted, so a fingerprint does not depend
    /// on whether a build preserves insertion order.
    fn canonical(v: &Value) -> String {
        match v {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let fields: Vec<String> = keys
                    .iter()
                    .map(|k| format!("{}:{}", Value::String((*k).clone()), canonical(&map[*k])))
                    .collect();
                format!("{{{}}}", fields.join(","))
            }
            Value::Array(items) => {
                let items: Vec<String> = items.iter().map(canonical).collect();
                format!("[{}]", items.join(","))
            }
            other => other.to_string(),
        }
    }

    fn fingerprint(v: &Value) -> String {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(canonical(v).as_bytes());
        digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
    }

    /// The twenty-four tools this door publishes, each with its whole
    /// definition (name, description, input schema) fingerprinted, and the
    /// server text. Building the one-door catalogue must not change any of
    /// them by accident: a change here is a change every connected host sees.
    #[tokio::test]
    async fn the_published_surface_is_locked() {
        let msg = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let res = handle_rpc(&deps(&hub()), &msg).await.unwrap();
        let actual: Vec<(String, String)> = res["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| (t["name"].as_str().unwrap().to_string(), fingerprint(t)))
            .collect();
        let expected: &[(&str, &str)] = &[
            ("browser_navigate", "ede6425916ca80bd"),
            ("browser_read_page", "7f3ac2e5a13d6d0d"),
            ("browser_click", "21d8ee320d7df0b7"),
            ("browser_type", "54fe4fd64c77ff4b"),
            ("browser_key", "87106ed080c7b4d7"),
            ("browser_scroll", "ad0652d9734ed73d"),
            ("browser_screenshot", "1c56148ee8d1bcbe"),
            ("browser_wait", "e13838e1f6e6d952"),
            ("browser_tabs", "82e4e1b35b8be51d"),
            ("browser_read_console", "226424d552016e56"),
            ("x_search", "a2aaae3f477419d2"),
            ("x_targets", "3a209e6d2b0f6c6c"),
            ("x_following", "65a1df5d1b583079"),
            ("x_whotofollow", "a0ca332f28195bb9"),
            ("x_own", "5ae3eda92299c571"),
            ("x_mentions", "253a788156648105"),
            ("memory_dream_status", "8210ce18f2b46074"),
            ("memory_dream_run", "0ccc699fdb330775"),
            ("agent_run", "9fdfb9719cf9afcc"),
            ("agent_status", "4ceddf9574826bb1"),
            ("agent_permit", "71f9feab2f5f626b"),
            ("agent_stop", "b0266b865355cc07"),
            ("agent_models", "62c279cbad122db8"),
            ("agent_session_close", "0dfce92d6e1636c6"),
        ];
        let shown: Vec<String> = actual
            .iter()
            .map(|(n, f)| format!("(\"{n}\", \"{f}\"),"))
            .collect();
        let want: Vec<(String, String)> = expected
            .iter()
            .map(|(n, f)| (n.to_string(), f.to_string()))
            .collect();
        assert_eq!(actual, want, "actual:\n{}", shown.join("\n"));
        let text = fingerprint(&initialize_result()["instructions"]);
        assert_eq!(text, "08bdcdfc29350c43", "instructions changed: {text}");
    }

    /// The door's JSON-RPC shapes: `ping` answers an empty object, a tool's
    /// failure is a result with `isError`, an unknown tool is invalid params.
    #[tokio::test]
    async fn the_rpc_shapes_are_locked() {
        let ping = json!({ "jsonrpc": "2.0", "id": 9, "method": "ping" });
        let res = handle_rpc(&deps(&hub()), &ping).await.unwrap();
        assert_eq!(res, json!({ "jsonrpc": "2.0", "id": 9, "result": {} }));
        let call = json!({
            "jsonrpc": "2.0", "id": 10, "method": "tools/call",
            "params": { "name": "browser_read_page", "arguments": {} }
        });
        let res = handle_rpc(&deps(&hub()), &call).await.unwrap();
        let result = res["result"].as_object().unwrap();
        let mut keys: Vec<&String> = result.keys().collect();
        keys.sort();
        assert_eq!(keys, ["content", "isError"]);
        assert_eq!(res["result"]["content"][0]["type"], "text");
        let unknown = json!({
            "jsonrpc": "2.0", "id": 11, "method": "tools/call",
            "params": { "name": "memory_search", "arguments": { "query": "q" } }
        });
        let res = handle_rpc(&deps(&hub()), &unknown).await.unwrap();
        assert_eq!(
            res["error"],
            json!({ "code": -32602, "message": "unknown tool: memory_search" })
        );
    }

    /// The catalogue's entry is the only place a tool's name decides
    /// anything: no dispatcher or renderer matches on a tool name (a second
    /// list that can drift from the catalogue).
    #[tokio::test]
    async fn nothing_outside_the_catalogue_matches_on_a_tool_name() {
        let msg = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let res = handle_rpc(&deps(&hub()), &msg).await.unwrap();
        let names: Vec<String> = res["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        for (file, source) in DISPATCH_SOURCES {
            let code = source.split("#[cfg(test)]").next().unwrap();
            for name in &names {
                let arm = format!("\"{name}\" =>");
                assert!(!code.contains(&arm), "{file} matches on {name}");
            }
        }
    }

    const DISPATCH_SOURCES: &[(&str, &str)] = &[
        ("mcp/mod.rs", include_str!("mod.rs")),
        ("mcp/dispatch.rs", include_str!("dispatch.rs")),
    ];

    #[tokio::test]
    async fn notifications_get_no_response() {
        let msg = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle_rpc(&deps(&hub()), &msg).await.is_none());
    }

    /// The front door's tools act as the owner, so only the owner's own
    /// computers come in.
    #[test]
    fn the_front_door_is_for_computers() {
        use crate::memory_host::caller::Caller;
        use crate::state_fs::devices::{DeviceKind, PairedDevice};
        let row = |kind| PairedDevice {
            id: "d".into(),
            name: "d".into(),
            secret: "s".into(),
            created_at: 0,
            kind: Some(kind),
            account: None,
            device_id: None,
            settings: Default::default(),
            relay_grant: None,
            superseded_grant: None,
            ..Default::default()
        };
        assert!(door_open_to(&Caller::Owner));
        assert!(door_open_to(&Caller::Machine(row(DeviceKind::Machine))));
        assert!(!door_open_to(&Caller::Phone(row(DeviceKind::Phone))));
        assert!(!door_open_to(&Caller::Unknown));
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
        headers.insert(ORIGIN, "http://localhost.evil.example".parse().unwrap());
        assert!(!origin_allowed(&headers));
    }
}
