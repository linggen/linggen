//! The twelve-tool door, before it is published: its views, every browser,
//! agent and engine-upkeep branch resolved to the dispatcher, bridge op,
//! arguments and timeout the legacy tool used, the refusals, and memory
//! reached through the engine's own path with no account and no model.

use super::catalog::{catalog, AgentOp, Dispatch};
use super::*;
use ling_mem::catalog::{Resolved, View};

fn hub() -> BridgeHub {
    BridgeHub::new()
}

fn deps(hub: &BridgeHub) -> McpDeps<'_> {
    McpDeps {
        bridge: hub,
        state: None,
    }
}

fn names(view: View) -> Vec<String> {
    folded_tools_list(view)["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

/// The branch a local call resolves to, and its arguments.
fn call(tool: &str, args: Value) -> (&'static Branch<Dispatch>, Value) {
    match catalog().resolve(View::Local, tool, args.clone()) {
        Resolved::Call { branch, args } => (branch, args),
        Resolved::Refused(why) => panic!("{tool} {args}: refused: {why}"),
        Resolved::Unknown => panic!("{tool}: unknown"),
    }
}

fn refused(tool: &str, args: Value) -> String {
    match catalog().resolve(View::Local, tool, args.clone()) {
        Resolved::Refused(why) => why,
        Resolved::Call { branch, .. } => panic!("{tool} {args}: ran as {}", branch.op),
        Resolved::Unknown => panic!("{tool}: unknown"),
    }
}

fn text(result: &Value) -> &str {
    result["content"][0]["text"].as_str().unwrap()
}

/// This machine sees twelve tools; an approved computer the five memory
/// tools, and of memory_upkeep everything but dream_run. A phone or an
/// unknown caller gets no view at all.
#[test]
fn each_caller_sees_its_view() {
    use crate::memory_host::caller::Caller;
    assert_eq!(names(View::Local).len(), 12);
    assert_eq!(
        names(View::Remote),
        ["memory_search", "memory_add", "memory_update", "memory_delete", "memory_upkeep"]
    );
    let ops = |view| {
        let tools = folded_tools_list(view);
        tools["tools"][4]["inputSchema"]["properties"]["op"]["enum"].clone()
    };
    let (local, remote) = (ops(View::Local), ops(View::Remote));
    assert!(local.as_array().unwrap().contains(&json!("dream_run")));
    assert!(!remote.as_array().unwrap().contains(&json!("dream_run")));
    assert!(remote.as_array().unwrap().contains(&json!("dream_status")));
    assert_eq!(remote.as_array().unwrap().len(), 12);

    let row = |kind| crate::state_fs::devices::PairedDevice {
        kind: Some(kind),
        ..Default::default()
    };
    use crate::state_fs::devices::DeviceKind;
    assert_eq!(view_of(&Caller::Owner), Some(View::Local));
    assert_eq!(view_of(&Caller::Machine(row(DeviceKind::Machine))), Some(View::Remote));
    assert_eq!(view_of(&Caller::Phone(row(DeviceKind::Phone))), None);
    assert_eq!(view_of(&Caller::Unknown), None);
}

/// What a view cannot see it cannot call, by its name or by its op: the
/// call is filtered by the same branches as the list.
#[tokio::test]
async fn a_hidden_tool_or_op_cannot_be_called() {
    let hub = hub();
    for (tool, args) in [
        ("agent_run", json!({"prompt": "hi"})),
        ("agent_status", json!({"op": "models"})),
        ("browser_read", json!({"op": "page"})),
        ("browser_act", json!({"op": "click", "ref": "n1"})),
    ] {
        let err = call_folded(&deps(&hub), View::Remote, tool, args).await.unwrap_err();
        assert_eq!(err, format!("unknown tool: {tool}"));
    }
    let res = call_folded(&deps(&hub), View::Remote, "memory_upkeep", json!({"op": "dream_run"}))
        .await
        .unwrap();
    assert_eq!(res["isError"], true);
    assert!(text(&res).contains("\"dream_run\" is not one of"), "{}", text(&res));
    let legacy = call_folded(&deps(&hub), View::Local, "memory_dream_run", json!({})).await;
    assert_eq!(legacy.unwrap_err(), "unknown tool: memory_dream_run");
}

/// The legacy tool a folded bridge branch stands for: same module and op,
/// the same timeout, and for a tab branch the `action` the legacy
/// `browser_tabs` call carried.
#[test]
fn every_browser_branch_runs_the_legacy_bridge_call() {
    let url = "https://example.com/a";
    let cases = [
        ("browser_open", json!({"op": "navigate", "url": url}), "browser_navigate", json!({"url": url})),
        ("browser_open", json!({"op": "navigate", "url": "back"}), "browser_navigate", json!({"url": "back"})),
        ("browser_open", json!({"op": "tab_open", "url": url}), "browser_tabs", json!({"url": url, "action": "open"})),
        ("browser_read", json!({"op": "page", "filter": "interactive"}), "browser_read_page", json!({"filter": "interactive"})),
        ("browser_read", json!({"op": "screenshot", "region": {"x": 0, "y": 0, "width": 9, "height": 9}}), "browser_screenshot", json!({"region": {"x": 0, "y": 0, "width": 9, "height": 9}})),
        ("browser_read", json!({"op": "console", "limit": "5"}), "browser_read_console", json!({"limit": 5})),
        ("browser_read", json!({"op": "tabs"}), "browser_tabs", json!({"action": "list"})),
        ("browser_act", json!({"op": "click", "ref": "n4", "double": "true"}), "browser_click", json!({"ref": "n4", "double": true})),
        ("browser_act", json!({"op": "click", "coordinate": [10, 20]}), "browser_click", json!({"coordinate": [10, 20]})),
        ("browser_act", json!({"op": "type", "text": "hi", "ref": "n2", "clear": true}), "browser_type", json!({"text": "hi", "ref": "n2", "clear": true})),
        ("browser_act", json!({"op": "key", "keys": "Enter", "repeat": "3"}), "browser_key", json!({"keys": "Enter", "repeat": 3})),
        ("browser_act", json!({"op": "scroll", "direction": "down", "amount": 300}), "browser_scroll", json!({"direction": "down", "amount": 300})),
        ("browser_act", json!({"op": "wait", "for": "ms", "value": "500"}), "browser_wait", json!({"for": "ms", "value": "500"})),
        ("browser_act", json!({"op": "wait", "for": "selector", "value": "#go"}), "browser_wait", json!({"for": "selector", "value": "#go"})),
        ("browser_act", json!({"op": "tab_switch"}), "browser_tabs", json!({"action": "switch"})),
        ("browser_act", json!({"op": "tab_close"}), "browser_tabs", json!({"action": "close"})),
    ];
    for (tool, args, legacy, sent) in cases {
        let (branch, got) = call(tool, args.clone());
        let old = catalog().legacy_branch(legacy).unwrap();
        assert_eq!(branch.dispatch, old.dispatch, "{tool} {args}");
        assert_eq!(branch.timeout_ms, old.timeout_ms, "{tool} {args}: timeout");
        assert_eq!(got, sent, "{tool} {args}");
    }
}

/// Bad browser values are refused before the bridge is asked.
#[test]
fn bad_browser_values_are_refused() {
    let cases = [
        ("browser_open", json!({"op": "navigate", "url": "example.com"}), "absolute URL"),
        ("browser_open", json!({"op": "tab_open", "url": "back"}), "absolute URL"),
        ("browser_open", json!({"op": "tab_open"}), "needs `url`"),
        ("browser_open", json!({"url": "https://a.b"}), "needs op"),
        ("browser_read", json!({"op": "page", "filter": "some"}), "`filter` must be one of"),
        ("browser_read", json!({"op": "page", "url": "https://a.b"}), "unknown field `url`"),
        ("browser_read", json!({"op": "screenshot", "region": {"x": "left"}}), ".x must be a number"),
        ("browser_read", json!({"op": "screenshot", "region": {"z": 1}}), "unknown field `z`"),
        ("browser_read", json!({"op": "console", "filter": "all"}), "`filter` belongs to op=page"),
        ("browser_read", json!({"op": "switch"}), "is not one of: page, screenshot, tabs, console"),
        ("browser_act", json!({"op": "click"}), "needs `ref`"),
        ("browser_act", json!({"op": "click", "ref": "n1", "coordinate": [1, 2]}), "not both"),
        ("browser_act", json!({"op": "click", "coordinate": [1]}), "at least 2 items"),
        ("browser_act", json!({"op": "click", "ref": "n1", "button": "back"}), "`button` must be one of"),
        ("browser_act", json!({"op": "type"}), "needs `text`"),
        ("browser_act", json!({"op": "key", "keys": "a", "repeat": 21}), "`repeat` must be 1–20"),
        ("browser_act", json!({"op": "scroll", "direction": "sideways"}), "`direction` must be one of"),
        ("browser_act", json!({"op": "wait", "for": "load", "value": "x"}), "takes no `value`"),
        ("browser_act", json!({"op": "wait", "for": "selector"}), "needs `value`"),
        ("browser_act", json!({"op": "wait", "for": "ms", "value": "20000"}), "0–10000"),
        ("browser_act", json!({"op": "wait", "for": "noon"}), "`for` must be one of"),
        ("browser_act", json!({"op": "tab_close", "url": "https://a.b"}), "unknown field `url`"),
        ("browser_act", json!({"op": "type", "text": "x", "direction": "up"}), "`direction` belongs to op=scroll"),
    ];
    for (tool, args, want) in cases {
        let why = refused(tool, args.clone());
        assert!(why.contains(want), "{tool} {args}: {why}");
    }
}

/// The agent branches: discovery and session reads apart, stop and close
/// apart, permit's scope with allow_session only, grants screened as before
/// (read and write folders, no tool list).
#[test]
fn every_agent_branch_resolves_or_is_refused() {
    let agent = |tool, args: Value| match call(tool, args).0.dispatch {
        Dispatch::Agent(op) => op,
        other => panic!("{other:?}"),
    };
    assert_eq!(agent("agent_run", json!({"prompt": "p"})), AgentOp::Run);
    assert_eq!(
        agent("agent_run", json!({"prompt": "p", "write_dir": "/w", "permission_mode": "acceptEdits", "grants": {"read": ["/a"], "write": ["/b"]}})),
        AgentOp::Run
    );
    assert_eq!(agent("agent_status", json!({"op": "session", "session_id": "s"})), AgentOp::Status);
    assert_eq!(agent("agent_status", json!({"op": "models"})), AgentOp::Readiness);
    assert_eq!(
        agent("agent_permit", json!({"session_id": "s", "request_id": "r", "decision": "allow_session", "scope": "dir"})),
        AgentOp::Permit
    );
    assert_eq!(agent("agent_end", json!({"op": "stop", "session_id": "s"})), AgentOp::Stop);
    let (branch, args) = call("agent_end", json!({"op": "close", "session_id": "s", "remove_worktree": "true"}));
    assert_eq!((branch.dispatch, args), (Dispatch::Agent(AgentOp::Close), json!({"session_id": "s", "remove_worktree": true})));

    let cases = [
        ("agent_run", json!({}), "needs `prompt`"),
        ("agent_run", json!({"prompt": "p", "permission_mode": "yolo"}), "`permission_mode` must be one of"),
        ("agent_run", json!({"prompt": "p", "grants": {"tools": ["Bash"]}}), "unknown field `tools`"),
        ("agent_run", json!({"prompt": "p", "op": "run"}), "takes no op"),
        ("agent_status", json!({"session_id": "s"}), "pass op=\"session\""),
        ("agent_status", json!({"op": "models", "session_id": "s"}), "`session_id` belongs to op=session"),
        ("agent_status", json!({"op": "session"}), "needs `session_id`"),
        ("agent_permit", json!({"session_id": "s", "request_id": "r", "decision": "allow_once", "scope": "dir"}), "allow_session only"),
        ("agent_permit", json!({"session_id": "s", "request_id": "r", "decision": "always"}), "`decision` must be one of"),
        ("agent_permit", json!({"session_id": "s", "decision": "deny"}), "needs `request_id`"),
        ("agent_end", json!({"op": "stop", "session_id": "s", "remove_worktree": true}), "`remove_worktree` belongs to op=close, not op=stop"),
        ("agent_end", json!({"session_id": "s"}), "needs op"),
    ];
    for (tool, args, want) in cases {
        let why = refused(tool, args.clone());
        assert!(why.contains(want), "{tool} {args}: {why}");
    }
}

/// The two upkeep ops only the engine runs sit in memory_upkeep beside
/// memory's eleven.
#[test]
fn the_engines_upkeep_ops_join_memorys() {
    assert_eq!(call("memory_upkeep", json!({"op": "dream_status"})).0.dispatch, Dispatch::DreamStatus);
    let (branch, args) = call("memory_upkeep", json!({"op": "dream_run", "day": "2026-11-01"}));
    assert_eq!((branch.dispatch, args), (Dispatch::DreamRun, json!({"day": "2026-11-01"})));
    assert_eq!(call("memory_upkeep", json!({"op": "sweep"})).0.dispatch, Dispatch::Memory("sweep"));
    let why = refused("memory_upkeep", json!({"op": "dream_status", "day": "2026-11-01"}));
    assert!(why.contains("`day` belongs to op=dream_lease/dream_run"), "{why}");
}

/// A folded call runs as the legacy one does: the bridge reached (and
/// refused without an extension), the agent and dream dispatchers reached
/// (and unavailable without the daemon), a refusal a tool error.
#[tokio::test]
async fn a_folded_call_reaches_the_legacy_dispatcher() {
    let hub = hub();
    let d = deps(&hub);
    let res = call_folded(&d, View::Local, "browser_read", json!({"op": "tabs"})).await.unwrap();
    assert_eq!(res["isError"], true);
    assert!(text(&res).contains("not connected"));
    let reason = crate::cannot_run::CannotRun::of_message(text(&res), None);
    assert_eq!(reason, Some(crate::cannot_run::CannotRun::ExtensionNotConnected));
    for (tool, args) in [
        ("agent_status", json!({"op": "models"})),
        ("memory_upkeep", json!({"op": "dream_status"})),
        ("memory_upkeep", json!({"op": "dream_run"})),
    ] {
        let res = call_folded(&d, View::Local, tool, args).await.unwrap();
        assert_eq!(res["isError"], true, "{tool}");
        assert!(text(&res).contains("unavailable"), "{tool}: {}", text(&res));
    }
    let res = call_folded(&d, View::Local, "browser_act", json!({"op": "fly"})).await.unwrap();
    assert_eq!(res["isError"], true);
    assert!(text(&res).contains("is not one of"));
    assert_eq!(
        call_folded(&d, View::Local, "browser_fly", json!({})).await.unwrap_err(),
        "unknown tool: browser_fly"
    );
}

/// Memory through the engine's door, on a scratch store with the
/// deterministic embedder: no account, no generative model. A store that is
/// not open or not hosted is a tool error with a reason of its own.
#[tokio::test]
async fn memory_runs_through_the_engines_door_without_an_account_or_a_model() {
    use crate::memory_host::{TestHost, TEST_HOST};
    let dir = tempfile::tempdir().unwrap();
    let store = ling_mem::hosted::open_quiet_fake(dir.path(), "http://127.0.0.1:1/mem".into())
        .await
        .unwrap();
    let hub = hub();
    let run = |tool: &'static str, args: Value| {
        let host = TestHost::Store(store.clone());
        let hub = &hub;
        TEST_HOST.scope(host, async move {
            call_folded(&deps(hub), View::Local, tool, args).await.unwrap()
        })
    };
    let added = run("memory_add", json!({"content": "Alex keeps bees", "tier": "semantic"})).await;
    assert_eq!(added["isError"], false, "{added}");
    let row: Value = serde_json::from_str(text(&added)).unwrap();
    let id = row["fact"]["id"].as_str().unwrap().to_string();
    let hits = run("memory_search", json!({"query": "Alex keeps bees"})).await;
    let hits: Value = serde_json::from_str(text(&hits)).unwrap();
    assert_eq!(hits[0]["id"], json!(id));
    let got = run("memory_search", json!({"op": "get", "id": id})).await;
    assert!(text(&got).contains("Alex keeps bees"));
    let days = run("memory_upkeep", json!({"op": "days"})).await;
    assert_eq!(days["isError"], false, "{days}");
    run("memory_delete", json!({"id": id})).await;
    let again = run("memory_delete", json!({"id": id})).await;
    assert!(text(&again).contains("already_gone"), "{}", text(&again));

    let closed = TEST_HOST
        .scope(TestHost::NotOpen("store locked by another process".into()), async {
            call_folded(&deps(&hub), View::Local, "memory_search", json!({"query": "q"})).await.unwrap()
        })
        .await;
    assert_eq!(closed["isError"], true);
    assert!(text(&closed).starts_with("store locked by another process — "), "{}", text(&closed));
    assert!(text(&closed).contains("ling status"), "{}", text(&closed));
    let absent = TEST_HOST
        .scope(TestHost::Absent, async {
            call_folded(&deps(&hub), View::Local, "memory_add", json!({"content": "x"})).await.unwrap()
        })
        .await;
    assert_eq!(absent["isError"], true);
    let reason = crate::cannot_run::CannotRun::of_message(text(&absent), None);
    assert!(matches!(reason, Some(crate::cannot_run::CannotRun::StoreUnavailable { .. })), "{reason:?}");
}

/// Every folded schema is a strict object; every branch has a form a
/// refusal can show.
#[test]
fn every_folded_schema_is_strict() {
    for t in folded_tools_list(View::Local)["tools"].as_array().unwrap() {
        let s = &t["inputSchema"];
        assert_eq!(s["type"], "object", "{}", t["name"]);
        assert_eq!(s["additionalProperties"], false, "{}", t["name"]);
        assert!(t["description"].as_str().is_some_and(|d| !d.is_empty()), "{}", t["name"]);
    }
}
