//! The folded memory tools against the legacy ones they replace: the same
//! steps on two scratch stores — one through memory's MCP server as hosts
//! call it today, one through the catalogue's folded branches — answer the
//! same, and leave the same store behind. No account and no generative
//! model: the deterministic embedder only.

use crate::support::*;
use ling_mem::catalog::memory::{catalog, run};
use ling_mem::catalog::{Resolved, View};
use serde_json::{json, Value};

/// A legacy `tools/call`: its parsed text, or its JSON-RPC error message.
async fn legacy(m: &Mem, tool: &str, args: Value) -> Result<Value, String> {
    let env = m
        .mcp("tools/call", json!({"name": tool, "arguments": args}))
        .await;
    if let Some(message) = env["error"]["message"].as_str() {
        return Err(message.to_string());
    }
    let text = env["result"]["content"][0]["text"].as_str().unwrap();
    Ok(serde_json::from_str(text).unwrap())
}

/// A folded call: resolved by the catalogue, run as a branch runs.
async fn folded(m: &Mem, tool: &str, args: Value) -> Result<Value, String> {
    match catalog().resolve(View::Local, tool, args) {
        Resolved::Call { branch, args } => run(&m.state, branch, args).await.map_err(|e| e.message),
        Resolved::Refused(why) => Err(format!("refused: {why}")),
        Resolved::Unknown => panic!("{tool} is not a folded tool"),
    }
}

/// `v` with this store's own ids (and the usual stamps) as placeholders.
fn same(v: &Value, ids: &[String]) -> Value {
    match v {
        Value::String(s) if ids.contains(s) => json!("<id>"),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), same(v, ids))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(|v| same(v, ids)).collect()),
        _ => v.clone(),
    }
}

fn id_of(v: &Value, at: &str) -> String {
    v.pointer(at).and_then(Value::as_str).unwrap().to_string()
}

/// Two stores, the steps of a session, a scan, a dream and a solve on
/// each: legacy tools on one, folded tools on the other. Every answer and
/// the store each leaves behind match.
#[tokio::test]
async fn the_folded_tools_answer_and_write_as_the_legacy_ones() {
    let (old, new) = (Mem::start().await, Mem::start().await);
    let (mut old_ids, mut new_ids) = (Vec::new(), Vec::new());
    let d3 = days_ago(3);

    // Each step: (legacy tool, args, folded tool, args).
    let voice = json!({"content": "Alex prefers green tea", "type": "preference", "from": "user", "tier": "semantic"});
    let a = legacy(&old, "memory_add", voice.clone()).await.unwrap();
    let b = folded(&new, "memory_add", voice).await.unwrap();
    old_ids.push(id_of(&a, "/fact/id"));
    new_ids.push(id_of(&b, "/fact/id"));
    assert_eq!(scrub(&same(&a, &old_ids)), scrub(&same(&b, &new_ids)), "add");

    let staged = json!({"content": "Alex walked to the harbour", "occurred_at": local_noon(&d3)});
    let a = legacy(&old, "memory_add", staged.clone()).await.unwrap();
    let b = folded(&new, "memory_add", staged).await.unwrap();
    old_ids.push(id_of(&a, "/fact/id"));
    new_ids.push(id_of(&b, "/fact/id"));
    assert_eq!(scrub(&same(&a, &old_ids)), scrub(&same(&b, &new_ids)), "episodic add");
    assert_eq!(b["fact"]["tier"], "episodic", "the default tier is episodic");

    let (o, n) = (old_ids[0].clone(), new_ids[0].clone());
    let steps: Vec<(&str, Value, &str, Value)> = vec![
        ("memory_search", json!({"query": "green tea"}), "memory_search", json!({"query": "green tea"})),
        ("memory_get", json!({"id": o}), "memory_search", json!({"op": "get", "id": n})),
        ("memory_list", json!({"tier": "semantic"}), "memory_search", json!({"op": "list", "tier": "semantic"})),
        ("memory_list", json!({"day": d3, "unjudged": true}), "memory_search", json!({"op": "list", "day": d3, "unjudged": true})),
        ("memory_update", json!({"id": o, "summary": "tea"}), "memory_update", json!({"id": n, "summary": "tea"})),
        ("memory_session_start", json!({}), "memory_upkeep", json!({"op": "session_start"})),
        ("memory_days", json!({}), "memory_upkeep", json!({"op": "days"})),
        ("memory_harvest_day", json!({"date": d3}), "memory_upkeep", json!({"op": "harvest_day", "date": d3})),
        ("memory_dream_lease", json!({"day": d3, "holder": "h"}), "memory_upkeep", json!({"op": "dream_lease", "day": d3, "holder": "h"})),
        ("memory_remember_day", json!({"date": d3, "judged": 1, "holder": "h"}), "memory_upkeep", json!({"op": "remember_day", "date": d3, "judged": 1, "holder": "h"})),
        ("memory_dream_lease", json!({"day": d3, "holder": "h", "action": "release"}), "memory_upkeep", json!({"op": "dream_lease", "day": d3, "holder": "h", "action": "release"})),
        ("memory_role", json!({}), "memory_upkeep", json!({"op": "role"})),
        ("memory_sweep", json!({"dry_run": true}), "memory_upkeep", json!({"op": "sweep", "dry_run": true})),
        ("memory_chains", json!({}), "memory_upkeep", json!({"op": "chains"})),
        ("memory_days", json!({"undreamed_only": true}), "memory_upkeep", json!({"op": "days", "undreamed_only": true})),
    ];
    for (lt, la, ft, fa) in steps {
        let a = legacy(&old, lt, la).await.unwrap();
        let b = folded(&new, ft, fa).await.unwrap();
        assert_eq!(scrub(&same(&a, &old_ids)), scrub(&same(&b, &new_ids)), "{lt}");
    }

    let a = legacy(&old, "memory_issue_add", json!({"kind": "chain", "row_ids": [o], "note": "n"})).await.unwrap();
    let b = folded(&new, "memory_upkeep", json!({"op": "issue_add", "kind": "chain", "row_ids": [n], "note": "n"})).await.unwrap();
    old_ids.push(id_of(&a, "/issue/id"));
    new_ids.push(id_of(&b, "/issue/id"));
    assert_eq!(scrub(&same(&a, &old_ids)), scrub(&same(&b, &new_ids)), "issue_add");
    let (oi, ni) = (old_ids[2].clone(), new_ids[2].clone());
    let steps: Vec<(&str, Value, &str, Value)> = vec![
        ("memory_issues", json!({}), "memory_upkeep", json!({"op": "issues"})),
        ("memory_issue_resolve", json!({"id": oi, "outcome": "resolved", "note": "done"}), "memory_upkeep", json!({"op": "issue_resolve", "id": ni, "outcome": "resolved", "note": "done"})),
        ("memory_issues", json!({"status": "all"}), "memory_upkeep", json!({"op": "issues", "status": "all"})),
        ("memory_delete", json!({"id": o}), "memory_delete", json!({"id": n})),
        ("memory_delete", json!({"id": o}), "memory_delete", json!({"id": n})),
        ("memory_list", json!({}), "memory_search", json!({"op": "list"})),
    ];
    for (lt, la, ft, fa) in steps {
        let a = legacy(&old, lt, la).await.unwrap();
        let b = folded(&new, ft, fa).await.unwrap();
        assert_eq!(scrub(&same(&a, &old_ids)), scrub(&same(&b, &new_ids)), "{lt}");
    }

    // What the folded writes left behind reads the same through the legacy
    // tools on that store.
    let a = legacy(&new, "memory_list", json!({})).await.unwrap();
    let b = folded(&new, "memory_search", json!({"op": "list"})).await.unwrap();
    assert_eq!(a, b);
}

/// The store's own refusals come back as they always did; the catalogue's
/// come before the store is asked, and the store is left untouched.
#[tokio::test]
async fn refusals_come_from_the_store_or_before_it() {
    let m = Mem::start().await;
    let voice = folded(&m, "memory_add", json!({"content": "Alex prefers tea", "from": "user", "tier": "semantic"}))
        .await
        .unwrap();
    let id = id_of(&voice, "/fact/id");

    let blank = folded(&m, "memory_add", json!({"content": " "})).await.unwrap_err();
    assert_eq!(blank, "content must not be empty");
    let guarded = folded(&m, "memory_add", json!({"content": "Alex prefers coffee", "replace_ids": [id]}))
        .await
        .unwrap_err();
    assert!(guarded.contains("user-voice"), "{guarded}");
    let future = folded(&m, "memory_upkeep", json!({"op": "harvest_day", "date": today()}))
        .await
        .unwrap_err();
    let legacy_future = legacy(&m, "memory_harvest_day", json!({"date": today()})).await.unwrap_err();
    assert_eq!(future, legacy_future);

    let before = m.ok("count", json!({})).await;
    let strict = folded(&m, "memory_add", json!({"content": "x", "tags": ["a"]})).await.unwrap_err();
    assert!(strict.starts_with("refused:"), "{strict}");
    assert_eq!(m.ok("count", json!({})).await, before, "nothing was written");
    let kept = folded(&m, "memory_search", json!({"op": "get", "id": id})).await.unwrap();
    assert_eq!(kept["content"], "Alex prefers tea");
}

/// Every memory branch is visible to an approved computer too: the five
/// tools and all eleven upkeep ops the store runs.
#[tokio::test]
async fn a_remote_computer_sees_every_memory_branch() {
    let local = catalog().tools_list(View::Local);
    let remote = catalog().tools_list(View::Remote);
    assert_eq!(local, remote);
    assert_eq!(remote.len(), 5);
}

/// The legacy soft-empty search, through the folded tool with no op: the
/// empty fields decide nothing and the call is the plain semantic search.
#[tokio::test]
async fn a_soft_empty_search_needs_no_op_either() {
    let m = Mem::start().await;
    folded(&m, "memory_add", json!({"content": "Alex keeps bees", "tier": "semantic"}))
        .await
        .unwrap();
    let args = json!({"query": "Alex keeps bees", "type": "", "from": "", "until": "", "since": "", "apps": [], "tier": null});
    let hits = folded(&m, "memory_search", args.clone()).await.unwrap();
    assert_eq!(hits.as_array().unwrap().len(), 1);
    assert_eq!(hits, legacy(&m, "memory_search", args).await.unwrap());
}
