//! The door's published surface and RPC: the twelve tools and their
//! definitions, the server text per view, the answers to an old plugin,
//! and the door's own posture (origin, keep-alive, who comes in).

use super::*;

fn hub() -> BridgeHub {
    BridgeHub::new()
}

// No ServerState in unit tests — agent_run errors cleanly, everything
// else is state-independent.
fn deps(hub: &BridgeHub) -> McpDeps<'_> {
    McpDeps {
        bridge: hub,
        state: None,
    }
}

async fn rpc(view: View, msg: Value) -> Value {
    handle_rpc(&deps(&hub()), view, &msg).await.unwrap()
}

async fn call(view: View, name: &str, args: Value) -> Value {
    rpc(
        view,
        json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": name, "arguments": args}}),
    )
    .await
}

/// Past the warm-up's bound (a short one here) a call that embeds is
/// refused with the warming fact as a tool error, and nothing is applied;
/// a call that does not embed answers as usual.
#[tokio::test]
async fn the_host_door_reports_warmup_without_applying_writes() {
    use crate::memory_host::{TestHost, TEST_HOST};
    use ling_mem::hosted::{hold_embedding_warmup, open_quiet_fake};
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let store = open_quiet_fake(dir.path(), String::new()).await.unwrap();
    TEST_HOST.scope(TestHost::Store(store.clone()), async {
        let added = call(View::Local, "memory_add", json!({"content": "original"})).await;
        assert_eq!(added["result"]["isError"], false, "{added}");
        let row: Value = serde_json::from_str(text(&added)).unwrap();
        let id = &row["fact"]["id"];
        let (release, warmup) = hold_embedding_warmup(&store, Duration::from_millis(50)).await;
        for view in [View::Local, View::Remote] {
            for (name, args) in [
                ("memory_search", json!({"query": "original"})),
                ("memory_search", json!({"op": "semantic", "query": "original"})),
                ("memory_add", json!({"content": "retry"})),
                ("memory_update", json!({"id": id, "summary": "retry"})),
            ] {
                let answer = tokio::time::timeout(Duration::from_secs(1), call(view, name, args))
                    .await.expect("the probe is still blocked");
                assert_eq!(answer["result"]["isError"], true, "{answer}");
                let message = text(&answer);
                assert!(message.starts_with("Memory is warming up ("), "{message}");
                assert!(message.ends_with("s elapsed)."), "{message}");
            }
            for (name, args) in [
                ("memory_search", json!({"op": "list"})),
                ("memory_search", json!({"op": "get", "id": id})),
                ("memory_upkeep", json!({"op": "role"})),
            ] {
                let answer = tokio::time::timeout(Duration::from_secs(1), call(view, name, args))
                    .await.expect("non-embedding calls must answer during warm-up");
                assert_eq!(answer["result"]["isError"], false, "{answer}");
            }
        }
        release.send(()).unwrap();
        warmup.await.unwrap();
        let listed = call(View::Local, "memory_search", json!({"op": "list"})).await;
        let rows: Value = serde_json::from_str(text(&listed)).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1, "{rows}");
        assert_eq!(rows[0]["id"], *id);
        assert!(rows[0]["summary"].is_null(), "{rows}");
    }).await;
}

/// During the warm-up a search with a query and an add wait for it, then
/// run; the automatic recall (`skip_if_warming`) is answered at once.
#[tokio::test]
async fn the_host_door_waits_for_the_warmup_then_runs() {
    use crate::memory_host::{TestHost, TEST_HOST};
    use ling_mem::hosted::{hold_embedding_warmup, open_quiet_fake};
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().unwrap();
    let store = open_quiet_fake(dir.path(), String::new()).await.unwrap();
    TEST_HOST.scope(TestHost::Store(store.clone()), async {
        let (release, warmup) = hold_embedding_warmup(&store, ling_mem::embed::WARM_WAIT).await;
        let recall = json!({"query": "tea", "skip_if_warming": true});
        let skipped = tokio::time::timeout(Duration::from_secs(1), call(View::Local, "memory_search", recall))
            .await
            .expect("a recall never waits for the warm-up");
        assert_eq!(skipped["result"]["isError"], true, "{skipped}");
        assert!(text(&skipped).starts_with("Memory is warming up ("), "{skipped}");

        let started = Instant::now();
        let held = Duration::from_millis(300);
        let (added, found, ()) = tokio::join!(
            call(View::Local, "memory_add", json!({"content": "Alex drinks green tea"})),
            call(View::Local, "memory_search", json!({"query": "green tea"})),
            async {
                tokio::time::sleep(held).await;
                release.send(()).unwrap();
            },
        );
        assert!(started.elapsed() >= held, "they answered before the warm-up ended");
        assert_eq!(added["result"]["isError"], false, "{added}");
        assert_eq!(found["result"]["isError"], false, "{found}");
        warmup.await.unwrap();
        let listed = call(View::Local, "memory_search", json!({"op": "list"})).await;
        assert!(text(&listed).contains("Alex drinks green tea"), "{listed}");
    }).await;
}

fn text(res: &Value) -> &str {
    res["result"]["content"][0]["text"].as_str().unwrap()
}

#[test]
fn the_dream_status_names_a_last_runs_failed_calls_and_old_rows_read_as_before() {
    let line = r#"{"run_id":"r","triggered_at":1,"status":"completed","skipped":false,"failures":[{"tool":"fetch","kind":"exit","exit":3,"tail":"down"}],"failure_count":1}"#;
    let run: crate::engine::mission::record::MissionRunEntry = serde_json::from_str(line).unwrap();
    let v = last_run_json(&run);
    assert_eq!(v["status"], "completed");
    assert_eq!(v["failure_count"], 1);
    assert_eq!(v["failures"][0]["exit"], 3);

    let old = r#"{"run_id":"r","triggered_at":1,"status":"failed","skipped":false}"#;
    let run: crate::engine::mission::record::MissionRunEntry = serde_json::from_str(old).unwrap();
    let v = last_run_json(&run);
    assert_eq!(
        (v["status"].clone(), v["failure_count"].clone()),
        (json!("failed"), json!(0))
    );
    assert_eq!(v["failures"], json!([]));
    assert!(run.error.is_none(), "an old row grew an error");
}

/// The server text for each view: at most 2,048 characters, and the facts
/// a caller acts on. This machine's names delegation, permission, the
/// usage limit and the browser; an approved computer's names memory only.
#[tokio::test]
async fn the_server_text_fits_and_carries_what_a_caller_acts_on() {
    for view in [View::Local, View::Remote] {
        let res = rpc(
            view,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
        )
        .await;
        assert_eq!(res["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(res["result"]["serverInfo"]["name"], "linggen");
        assert!(res["result"]["capabilities"]["tools"].is_object());
        let text = res["result"]["instructions"].as_str().unwrap();
        let n = text.chars().count();
        assert!(n <= 2048, "{view:?}: {n} characters");
        for fact in [
            "without an account",
            "stable identity (name, role",
            "search the subject",
            "from=user",
            "memory_add + replace_ids; never add then delete",
            "user_directed:true only after their answer",
            "Cite recalled facts",
            "memory_upkeep (op=dream_status)",
        ] {
            assert!(text.contains(fact), "{view:?} misses {fact}");
        }
    }
    let local = instructions(View::Local);
    for fact in [
        "new background watcher",
        "/api/agent/wait",
        "agent_permit: allow_once, allow_session",
        "Outward requests take allow_once or deny only",
        "the deny floor cannot be granted",
        "10 minutes means deny",
        "from finished",
        "agent_status (op=session) once; if still running, start a new watch",
        "Do not poll",
        "On a usage limit, read its reset time",
        "pick a ready model from agent_status (op=models) or come back after it; never retry before",
        "per-site permission in Chrome",
        "Read again after page changes",
    ] {
        assert!(local.contains(fact), "local misses {fact}");
    }
    let remote = instructions(View::Remote);
    for absent in ["agent_run", "agent_permit", "Browser", "dream_run"] {
        assert!(!remote.contains(absent), "remote names {absent}");
    }
    assert!(remote.contains("with your own model"));
}

/// Each view's list, from the one table.
#[tokio::test]
async fn tools_list_is_the_views_catalogue() {
    let names = |res: Value| -> Vec<String> {
        res["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    };
    let list = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
    let local = names(rpc(View::Local, list.clone()).await);
    assert_eq!(local.len(), 12);
    assert_eq!(
        &local[..5],
        [
            "memory_search",
            "memory_add",
            "memory_update",
            "memory_delete",
            "memory_upkeep"
        ]
    );
    let remote = names(rpc(View::Remote, list).await);
    assert_eq!(
        remote,
        [
            "memory_search",
            "memory_add",
            "memory_update",
            "memory_delete",
            "memory_upkeep"
        ]
    );
    // memory_upkeep's text names exactly the ops its view can call.
    for (view, count) in [(View::Local, 13), (View::Remote, 12)] {
        let upkeep = &catalog().tools_list(view)[4];
        let ops: Vec<&str> = upkeep["inputSchema"]["properties"]["op"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(ops.len(), count, "{view:?}");
        let said = upkeep["description"].as_str().unwrap();
        assert!(
            said.contains(&format!("Ops here: {}.", ops.join(", "))),
            "{view:?}: {said}"
        );
        assert_eq!(said.contains("dream_run"), view == View::Local, "{said}");
    }
}

/// The memory tools a host sees here and the ones the engine's own agents
/// see at `/mem/mcp` come from the same branches: the four one-branch
/// tools are the same definition, and every op of memory's upkeep has the
/// same fields at both.
#[test]
fn memory_on_the_door_is_memorys_own_branches() {
    use ling_mem::catalog::memory;
    let door = catalog().tools_list(View::Local);
    let inside = memory::catalog().tools_list(View::Local);
    for i in 0..4 {
        assert_eq!(door[i], inside[i], "{}", inside[i]["name"]);
    }
    let ops = |t: &Value| {
        t["inputSchema"]["properties"]["op"]["enum"]
            .as_array()
            .unwrap()
            .clone()
    };
    let (door_ops, inside_ops) = (ops(&door[4]), ops(&inside[4]));
    assert!(
        inside_ops.iter().all(|op| door_ops.contains(op)),
        "{door_ops:?}"
    );
    assert_eq!(
        door_ops.len(),
        inside_ops.len() + 2,
        "the engine adds dream_status and dream_run"
    );
    for b in memory::catalog().visible(View::Local, "memory_upkeep") {
        let door_branch = catalog()
            .branch_of(View::Local, "memory_upkeep", &json!({"op": b.op}))
            .unwrap();
        assert_eq!((door_branch.schema)(), (b.schema)(), "{}", b.op);
    }
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

/// The twelve tools this door publishes to this machine and the five to an
/// approved computer, each with its whole definition (name, description,
/// input schema) fingerprinted, and both server texts: a change here is a
/// change every connected host sees.
#[tokio::test]
async fn the_published_surface_is_locked() {
    let shown = |view: View| -> Vec<(String, String)> {
        catalog()
            .tools_list(view)
            .iter()
            .map(|t| (t["name"].as_str().unwrap().to_string(), fingerprint(t)))
            .collect()
    };
    let want = |expected: &[(&str, &str)]| -> Vec<(String, String)> {
        expected
            .iter()
            .map(|(n, f)| (n.to_string(), f.to_string()))
            .collect()
    };
    let print = |actual: &[(String, String)]| -> String {
        actual
            .iter()
            .map(|(n, f)| format!("(\"{n}\", \"{f}\"),"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let local = shown(View::Local);
    let expected_local: &[(&str, &str)] = &[
        ("memory_search", "3961fa2c6c739601"),
        ("memory_add", "85c36d750751513b"),
        ("memory_update", "21da0a4313b7b843"),
        ("memory_delete", "bbc85f7c3b3268e1"),
        ("memory_upkeep", "641714d169511379"),
        ("agent_run", "b81061f2a6510c81"),
        ("agent_status", "cfdb72c208890513"),
        ("agent_permit", "da375d9cdd613884"),
        ("agent_end", "289a89471b91f423"),
        ("browser_open", "97f5d840f195a315"),
        ("browser_read", "f9802d55ffbe79be"),
        ("browser_act", "258452acd0f37bb6"),
    ];
    assert_eq!(local, want(expected_local), "local:\n{}", print(&local));
    let remote = shown(View::Remote);
    let expected_remote: &[(&str, &str)] = &[
        ("memory_search", "3961fa2c6c739601"),
        ("memory_add", "85c36d750751513b"),
        ("memory_update", "21da0a4313b7b843"),
        ("memory_delete", "bbc85f7c3b3268e1"),
        ("memory_upkeep", "e9ff93db14123772"),
    ];
    assert_eq!(remote, want(expected_remote), "remote:\n{}", print(&remote));
    let texts = (
        fingerprint(&json!(instructions(View::Local))),
        fingerprint(&json!(instructions(View::Remote))),
    );
    assert_eq!(
        texts,
        (
            "c1363a248e1c4d6f".to_string(),
            "c018af3021faaa2a".to_string()
        ),
        "server texts changed: {texts:?}"
    );
}

/// The door's JSON-RPC shapes: `ping` answers an empty object, a tool's
/// failure is a result with `isError`, an unknown tool is invalid params.
#[tokio::test]
async fn the_rpc_shapes_are_locked() {
    let res = rpc(
        View::Local,
        json!({ "jsonrpc": "2.0", "id": 9, "method": "ping" }),
    )
    .await;
    assert_eq!(res, json!({ "jsonrpc": "2.0", "id": 9, "result": {} }));
    // A browser call with no extension: a failure result whose reason is a
    // field of its own (`structuredContent.cannot_run`), beside its words.
    let res = call(View::Local, "browser_read", json!({"op": "page"})).await;
    let result = res["result"].as_object().unwrap();
    let mut keys: Vec<&String> = result.keys().collect();
    keys.sort();
    assert_eq!(keys, ["content", "isError", "structuredContent"]);
    assert_eq!(res["result"]["content"][0]["type"], "text");
    let why = &res["result"]["structuredContent"]["cannot_run"];
    let mut fields: Vec<&String> = why.as_object().unwrap().keys().collect();
    fields.sort();
    assert_eq!(fields, ["line", "reason"]);
    let res = call(View::Local, "browser_fly", json!({})).await;
    assert_eq!(
        res["error"],
        json!({ "code": -32602, "message": "unknown tool: browser_fly" })
    );
}

/// Every name published before the fold, called now: nothing runs, the
/// answer says to update the plugin, carries `UPGRADE_PLUGIN` and names
/// what does the work now. An approved computer is told the same (for a
/// tool its view lacks, the name it is told is unknown to it too).
#[tokio::test]
async fn a_retired_name_is_told_to_update_and_runs_nothing() {
    for retired in catalog().retired() {
        let res = call(View::Local, retired.name, json!({"id": "x", "query": "q"})).await;
        assert_eq!(res["result"]["isError"], true, "{}", retired.name);
        assert_eq!(
            res["result"]["structuredContent"]["code"], "UPGRADE_PLUGIN",
            "{}",
            retired.name
        );
        let said = text(&res);
        assert!(
            said.contains("update the Linggen plugin and restart your host"),
            "{said}"
        );
        assert!(
            said.contains("nothing ran") && said.contains(retired.now),
            "{said}"
        );
    }
    let res = call(View::Remote, "memory_get", json!({"id": "x"})).await;
    assert_eq!(res["result"]["structuredContent"]["code"], "UPGRADE_PLUGIN");
}

/// A current name called in an older shape — `agent_status` with only a
/// session, `agent_end` or `memory_upkeep` with no op — is told to update
/// or send the form that works; it runs nothing. A call with an op that is
/// wrong is a plain refusal.
#[tokio::test]
async fn an_old_shape_on_a_new_name_is_told_to_update() {
    for (name, args) in [
        ("agent_status", json!({"session_id": "s"})),
        ("agent_end", json!({"session_id": "s"})),
        ("memory_upkeep", json!({"date": "2026-11-01"})),
        ("browser_act", json!({"ref": "n1"})),
    ] {
        let res = call(View::Local, name, args).await;
        assert_eq!(res["result"]["isError"], true, "{name}");
        assert_eq!(
            res["result"]["structuredContent"]["code"], "UPGRADE_PLUGIN",
            "{name}"
        );
        let said = text(&res);
        assert!(
            said.contains("update the Linggen plugin and reconnect"),
            "{said}"
        );
        assert!(said.contains("op"), "the form that works: {said}");
    }
    let res = call(View::Local, "agent_status", json!({"op": "fly"})).await;
    assert!(res["result"].get("structuredContent").is_none(), "{res}");
    // The commonest call needs no op, and is no old shape.
    let res = call(View::Local, "memory_search", json!({"id": "a"})).await;
    assert!(res["result"].get("structuredContent").is_none(), "{res}");
}

/// Whole-word: `browser_read` is a tool, `browser_read_page` retired.
fn names_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        let part = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        !part(before) && !part(after)
    })
}

/// No published text and no answer the door writes names a retired tool:
/// a model told to call one would call into "update the plugin".
#[test]
fn nothing_published_names_a_retired_tool() {
    let mut texts = vec![
        instructions(View::Local).to_string(),
        instructions(View::Remote).to_string(),
        serde_json::to_string(&catalog().tools_list(View::Local)).unwrap(),
        serde_json::to_string(&catalog().tools_list(View::Remote)).unwrap(),
    ];
    for (file, source) in ANSWER_SOURCES {
        let code = source.split("#[cfg(test)]").next().unwrap();
        texts.push(format!("{file}: {code}"));
    }
    let code_only = ["mod.rs", "catalog.rs"];
    for retired in catalog().retired() {
        for text in &texts {
            if code_only
                .iter()
                .any(|f| text.starts_with(&format!("{f}: ")))
            {
                continue;
            }
            assert!(
                !names_word(text, retired.name),
                "names {}: {}",
                retired.name,
                &text[..60.min(text.len())]
            );
        }
    }
}

/// The non-test source of what writes an answer a caller reads.
const ANSWER_SOURCES: &[(&str, &str)] = &[
    ("dispatch.rs", include_str!("dispatch.rs")),
    ("mcp_agent.rs", include_str!("../mcp_agent.rs")),
];

/// The catalogue's entry is the only place a tool's name decides
/// anything: no code of the door's — its server, its dispatchers, the agent
/// and grant handlers they call — matches on a tool name, in any shape (a
/// match arm, an `if` test, an alternation), which would be a second list
/// that can drift from the catalogue.
#[test]
fn no_door_file_but_the_catalogue_matches_on_a_tool_name() {
    let names: Vec<String> = catalog()
        .tools()
        .iter()
        .map(|t| t.name.to_string())
        .chain(catalog().retired().iter().map(|r| r.name.to_string()))
        .collect();
    let mut wrong = Vec::new();
    for (file, source) in DISPATCH_SOURCES {
        let code = source.split("#[cfg(test)]").next().unwrap();
        for name in &names {
            let quoted = format!("\"{name}\"");
            let shapes = [
                format!("{quoted} =>"),
                format!("{quoted} |"),
                format!("| {quoted}"),
                format!("== {quoted}"),
                format!("!= {quoted}"),
                format!("Some({quoted})"),
            ];
            if shapes.iter().any(|s| code.contains(s.as_str())) {
                wrong.push(format!("{file} matches on {name}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Every non-test source file of the door: the catalogue alone may name
/// a tool.
const DISPATCH_SOURCES: &[(&str, &str)] = &[
    ("mcp/mod.rs", include_str!("mod.rs")),
    ("mcp/dispatch.rs", include_str!("dispatch.rs")),
    ("mcp_agent.rs", include_str!("../mcp_agent.rs")),
    ("mcp_grants.rs", include_str!("../mcp_grants.rs")),
];

#[tokio::test]
async fn notifications_get_no_response() {
    let msg = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    assert!(handle_rpc(&deps(&hub()), View::Local, &msg).await.is_none());
}

/// The door's tools act as the owner, so only the owner's own computers
/// come in: this machine with everything, an approved computer with
/// memory, a phone and an unknown caller not at all.
#[test]
fn the_front_door_is_for_computers() {
    use crate::memory_host::caller::Caller;
    use crate::state_fs::devices::{DeviceKind, PairedDevice};
    let row = |kind| PairedDevice {
        id: "d".into(),
        kind: Some(kind),
        ..Default::default()
    };
    assert_eq!(view_of(&Caller::Owner), Some(View::Local));
    assert_eq!(
        view_of(&Caller::Machine(row(DeviceKind::Machine))),
        Some(View::Remote)
    );
    assert_eq!(view_of(&Caller::Phone(row(DeviceKind::Phone))), None);
    assert_eq!(view_of(&Caller::Unknown), None);
}

#[test]
fn mcp_responses_disable_keep_alive() {
    let resp = one_shot((StatusCode::OK, "ok").into_response());
    assert_eq!(resp.headers().get(CONNECTION).unwrap(), "close");
}

#[tokio::test]
async fn unknown_method_is_rpc_error() {
    let res = rpc(
        View::Local,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/list" }),
    )
    .await;
    assert_eq!(res["error"]["code"], -32601);
}

#[tokio::test]
async fn unknown_tool_is_invalid_params() {
    let res = call(View::Local, "browser_fly", json!({})).await;
    assert_eq!(res["error"]["code"], -32602);
}

#[tokio::test]
async fn call_without_bridge_is_tool_error_not_rpc_error() {
    let res = call(View::Local, "browser_read", json!({"op": "tabs"})).await;
    assert_eq!(res["result"]["isError"], true);
    assert!(text(&res).contains("not connected"));
}

#[tokio::test]
async fn agent_run_without_state_is_tool_error() {
    let res = call(View::Local, "agent_run", json!({"prompt": "hi"})).await;
    assert_eq!(res["result"]["isError"], true);
}

#[tokio::test]
async fn dream_ops_without_state_are_tool_errors() {
    for args in [
        json!({"op": "dream_status"}),
        json!({"op": "dream_run", "day": "2026-07-01"}),
    ] {
        let res = call(View::Local, "memory_upkeep", args.clone()).await;
        assert_eq!(res["result"]["isError"], true, "{args}");
        assert!(text(&res).contains("unavailable"), "{args}: {}", text(&res));
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
