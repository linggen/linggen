//! Memory's MCP endpoint on the router (`/mcp`): what Claude Code, Codex
//! and the engine's own agents see. Its `tools/call` answers a verb as the
//! store's own `/api/memory/<verb>` route answers it, in-process.

use crate::support::*;
use serde_json::{json, Value};

/// A `tools/call`'s answer: the parsed JSON of its single text item.
async fn call(m: &Mem, tool: &str, args: Value) -> Value {
    let env = m
        .mcp("tools/call", json!({"name": tool, "arguments": args}))
        .await;
    let content = env["result"]["content"]
        .as_array()
        .unwrap_or_else(|| panic!("{env}"));
    assert_eq!(content.len(), 1, "{env}");
    assert_eq!(content[0]["type"], "text");
    serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn initialize_and_tools_list() {
    let m = Mem::start().await;
    let init = m.mcp("initialize", json!({})).await["result"].clone();
    assert_eq!(init["protocolVersion"], "2024-11-05");
    assert_eq!(init["capabilities"], json!({"tools": {}}));
    assert_eq!(
        init["serverInfo"],
        json!({"name": "memory", "version": env!("CARGO_PKG_VERSION")})
    );
    let n = init["instructions"].as_str().unwrap().chars().count();
    assert!(n > 500 && n <= 2048, "instructions are {n} chars");

    let tools = m.mcp("tools/list", json!({})).await["result"]["tools"].clone();
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "memory_search",
            "memory_list",
            "memory_session_start",
            "memory_get",
            "memory_add",
            "memory_delete",
            "memory_update",
            "memory_harvest_day",
            "memory_days",
            "memory_remember_day",
            "memory_dream_lease",
            "memory_role",
            "memory_sweep",
            "memory_issues",
            "memory_issue_add",
            "memory_issue_resolve",
            "memory_chains",
        ]
    );
    assert_eq!(names.len(), 17);
    for t in tools.as_array().unwrap() {
        assert!(
            t["description"].as_str().is_some_and(|d| !d.is_empty()),
            "{}",
            t["name"]
        );
        assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
    }
}

#[tokio::test]
async fn add_search_list_and_delete_through_tools_call() {
    let m = Mem::start().await;
    let added = call(
        &m,
        "memory_add",
        json!({"content": "Alex prefers green tea", "tier": "semantic", "indexed": "false"}),
    )
    .await;
    assert_eq!(
        scrub(&added),
        json!({"action": "added", "fact": row("Alex prefers green tea", "fact", "semantic", "derived", json!({}))})
    );
    let id = added["fact"]["id"].clone();

    let hits = call(
        &m,
        "memory_search",
        json!({"query": "Alex prefers green tea"}),
    )
    .await;
    assert_eq!(hits.as_array().unwrap().len(), 1);
    assert_eq!(hits[0]["id"], id);
    assert!(hits[0].get("vector").is_none());

    let listed = call(&m, "memory_list", json!({"tier": "semantic", "limit": "5"})).await;
    assert_eq!(listed, json!([added["fact"]]));

    let removed = call(&m, "memory_delete", json!({"id": id})).await;
    assert_eq!(removed, json!({"id": id, "removed": true}));
    let again = call(&m, "memory_delete", json!({"id": id})).await;
    assert_eq!(
        again,
        json!({
            "id": id,
            "removed": false,
            "already_gone": true,
            "note": "row was already absent — treat as success; do not retry or verify",
        })
    );
}

/// `tier=episodic` is turned into the routing flag; on REST alone the
/// same body also lands episodic (the handler reads the tier too).
#[tokio::test]
async fn an_episodic_add_lands_in_the_staging_table() {
    let m = Mem::start().await;
    let added = call(
        &m,
        "memory_add",
        json!({"content": "Alex walked to the harbour", "tier": "episodic"}),
    )
    .await;
    assert_eq!(added["fact"]["tier"], "episodic");
    assert_eq!(m.ok("count", json!({"episodic": true})).await["count"], 1);
    let staged = call(
        &m,
        "memory_search",
        json!({"query": "Alex walked to the harbour", "tier": "episodic"}),
    )
    .await;
    assert_eq!(staged.as_array().unwrap().len(), 1);
    let curated = call(
        &m,
        "memory_search",
        json!({"query": "Alex walked to the harbour", "tier": "semantic"}),
    )
    .await;
    assert_eq!(curated, json!([]));
}

/// Fields a model fills with "" or [] are dropped before the verb sees
/// them (a bare `until: ""` would otherwise be a date error).
#[tokio::test]
async fn a_soft_empty_search_is_a_plain_search() {
    let m = Mem::start().await;
    call(
        &m,
        "memory_add",
        json!({"content": "Alex keeps bees", "tier": "semantic"}),
    )
    .await;
    let hits = call(
        &m,
        "memory_search",
        json!({"query": "Alex keeps bees", "type": "", "from": "", "until": "", "since": "", "apps": [], "tier": null}),
    )
    .await;
    assert_eq!(hits.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn refusals_are_json_rpc_errors() {
    let m = Mem::start().await;
    let env = m
        .mcp(
            "tools/call",
            json!({"name": "memory_add", "arguments": {"content": " "}}),
        )
        .await;
    assert_eq!(
        env["error"],
        json!({"code": -32603, "message": "content must not be empty"})
    );
    let env = m
        .mcp(
            "tools/call",
            json!({"name": "memory_forget", "arguments": {}}),
        )
        .await;
    assert_eq!(
        env["error"],
        json!({"code": -32602, "message": "unknown tool: memory_forget"})
    );
    let env = m.mcp("tools/call", json!({})).await;
    assert_eq!(
        env["error"],
        json!({"code": -32602, "message": "tools/call requires `name`"})
    );
    let env = m.mcp("resources/list", json!({})).await;
    assert_eq!(
        env["error"],
        json!({"code": -32601, "message": "method not found: resources/list"})
    );
    // A notification (no id) is answered 204, no body.
    let note = m
        .post(
            "/mcp",
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        )
        .await;
    assert_eq!(note, (204, json!("")));
}
