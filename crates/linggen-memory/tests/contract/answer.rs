//! `http::verb::answer` — a verb answered as its route answers it, for a
//! caller whose args are JSON already (memory's MCP server, the engine's
//! door): the same status and the same bytes as a POST to the route.

use crate::support::*;
use serde_json::{json, Value};

/// The in-process answer, read as `support` reads a route's.
async fn answered(m: &Mem, verb: &str, args: Value) -> (u16, Value) {
    let res = ling_mem::http::verb::answer(m.state.clone(), ling_mem::Who::Owner, verb, args).await;
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&bytes).to_string();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

async fn same_as_the_route(m: &Mem, verb: &str, args: Value) -> (u16, Value) {
    let route = m.call(verb, args.clone()).await;
    let here = answered(m, verb, args).await;
    assert_eq!(here, route, "{verb}");
    here
}

#[tokio::test]
async fn every_answer_is_the_routes() {
    let m = Mem::start().await;
    // A write runs once: the route's answer to it is pinned elsewhere.
    let added = answered(
        &m,
        "add",
        json!({"content": "Alex keeps bees", "tier": "semantic"}),
    )
    .await;
    assert_eq!(
        (added.0, added.1["data"]["action"].clone()),
        (200, json!("added"))
    );
    same_as_the_route(&m, "count", json!({})).await;
    same_as_the_route(&m, "get", json!({"id": "nope"})).await;
    // A verb without arguments never reads its body.
    same_as_the_route(&m, "stats", json!("not an object")).await;
    let rejected = same_as_the_route(&m, "add", json!({"contents": "x"})).await;
    assert_eq!(
        rejected,
        (
            422,
            json!("Failed to deserialize the JSON body into the target type: missing field `content` at line 1 column 16")
        )
    );
    assert_eq!(
        same_as_the_route(&m, "wat", json!({})).await,
        (404, json!(""))
    );
    // `apply_schema` is a verb by name, but its route is `/api/schema/apply`:
    // at `/api/memory/apply_schema` there is nothing.
    assert_eq!(
        same_as_the_route(&m, "apply_schema", json!({})).await,
        (404, json!(""))
    );
}

/// A replica whose primary does not answer: a body it cannot read is
/// refused in axum's words, as its route refuses it, and nothing is queued.
#[tokio::test]
async fn an_offline_replicas_unreadable_write_is_refused_as_its_route_refuses_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("memory")).unwrap();
    let file = json!({"primary": "127.0.0.1:1", "token": "tok-b", "device_id": "dev-b-0001"});
    std::fs::write(dir.path().join("memory/.replica.json"), file.to_string()).unwrap();
    let b = Mem::open(dir, "").await;
    let refused = same_as_the_route(&b, "add", json!({"contents": "x"})).await;
    assert_eq!(refused.0, 422, "{refused:?}");
    let outbox = std::fs::read_to_string(b.data_dir().join("memory/.outbox.json"))
        .unwrap_or_else(|_| "[]".into());
    assert_eq!(serde_json::from_str::<Value>(&outbox).unwrap(), json!([]));
}

/// Over MCP, a body the route refuses reads as the route's own words — a
/// hidden field (accepted, never advertised, so the catalogue does not type
/// it) of the wrong type is the route's to refuse.
#[tokio::test]
async fn an_mcp_tool_call_the_route_refuses_says_the_routes_words() {
    let m = Mem::start().await;
    let env = m
        .mcp(
            "tools/call",
            json!({"name": "memory_add", "arguments": {"content": "x", "force": 5}}),
        )
        .await;
    assert_eq!(env["error"]["code"], -32603);
    let message = env["error"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("add: HTTP 422: Failed to deserialize the JSON body into the target type: force: invalid type"),
        "{message}"
    );
}
