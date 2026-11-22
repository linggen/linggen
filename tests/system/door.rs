//! A delegated run through Linggen's one MCP door (`/mcp`), on the fake
//! model: a permission request that is pending before any watch connects
//! is seen in `agent_status` and answered with `agent_permit`; an outward
//! request takes only allow_once or deny; `agent_end op=stop` releases a
//! run that waits on its caller; `op=close` never removes a folder that is
//! not a clean, merged linked worktree.

use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use serde_json::{json, Value};

fn rpc(name: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}})
}

/// A call's text, and whether it was a tool error.
async fn door(w: &World, name: &str, args: Value) -> (String, bool) {
    let res = w.api.post("/mcp", rpc(name, args)).await;
    let said = res["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{name}: {res}"))
        .to_string();
    (said, res["result"]["isError"] == json!(true))
}

/// The session a "still running" answer names.
fn session_of(said: &str) -> String {
    let at = said
        .find("Linggen session ")
        .unwrap_or_else(|| panic!("no session in: {said}"));
    let rest = &said[at + "Linggen session ".len()..];
    rest.split(|c: char| c.is_whitespace() || c == '.')
        .next()
        .unwrap()
        .to_string()
}

async fn status(w: &World, sid: &str) -> Value {
    let (said, err) = door(
        w,
        "agent_status",
        json!({"op": "session", "session_id": sid}),
    )
    .await;
    assert!(!err, "{said}");
    serde_json::from_str(&said).unwrap()
}

/// Wait until the session's status satisfies `done`.
async fn status_until(w: &World, sid: &str, what: &str, done: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..200 {
        let now = status(w, sid).await;
        if done(&now) {
            return now;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("{what}: {}", status(w, sid).await);
}

fn pending(status: &Value) -> Vec<Value> {
    status["pending"].as_array().cloned().unwrap_or_default()
}

/// The run asks to push: agent_run returns at once, the request is in
/// agent_status before anything watched, a scope with allow_once is
/// refused before anything is answered, allow_once runs that one command,
/// and the run finishes with its reply.
#[tokio::test]
async fn a_request_before_any_watch_is_answered_through_the_door() {
    let setup = Setup::default().script(
        Agent::Ling,
        vec![
            tool(
                "Bash",
                json!({"cmd": "git push no-such-remote-door-test main"}),
            ),
            text("Tried the push."),
        ],
    );
    let w = World::start(setup).await;
    let (said, err) = door(
        &w,
        "agent_run",
        json!({"prompt": "Push it.", "write_dir": w.work(), "permission_mode": "acceptEdits"}),
    )
    .await;
    assert!(!err && said.contains("Still running"), "{said}");
    let sid = session_of(&said);
    let asked = status_until(&w, &sid, "a pending request", |s| !pending(s).is_empty()).await;
    let request = &pending(&asked)[0];
    assert_eq!(request["tool"], "Bash", "{request}");
    let id = request["id"].as_str().unwrap().to_string();

    let (said, err) = door(
        &w,
        "agent_permit",
        json!({"session_id": sid, "request_id": id, "decision": "allow_once", "scope": "dir"}),
    )
    .await;
    assert!(
        err && said.contains("allow_session only"),
        "scope goes with allow_session: {said}"
    );
    let (said, err) = door(
        &w,
        "agent_permit",
        json!({"session_id": sid, "request_id": id, "decision": "allow_once"}),
    )
    .await;
    assert!(!err, "{said}");
    let done = status_until(&w, &sid, "the run's end", |s| {
        s["running"] == json!(false) && s["status"] == "completed"
    })
    .await;
    assert_eq!(done["reply"], "Tried the push.", "{done}");
    assert_eq!(
        w.model.calls_of(Agent::Ling).len(),
        2,
        "the command's answer reached the model"
    );
}

/// An outward command (a push) asks even where the run may run programs:
/// a session-wide grant is refused for it, deny answers it, and the run
/// goes on to its reply.
#[tokio::test]
async fn an_outward_request_takes_allow_once_or_deny_only() {
    let setup = Setup::default().script(
        Agent::Ling,
        vec![
            tool(
                "Bash",
                json!({"cmd": "git push no-such-remote-door-test main"}),
            ),
            text("Did not push."),
        ],
    );
    let w = World::start(setup).await;
    let (said, _) = door(
        &w,
        "agent_run",
        json!({"prompt": "Push it.", "write_dir": w.work(), "permission_mode": "acceptEdits"}),
    )
    .await;
    assert!(said.contains("Still running"), "{said}");
    let sid = session_of(&said);
    let asked = status_until(&w, &sid, "the outward request", |s| !pending(s).is_empty()).await;
    let request = &pending(&asked)[0];
    assert_eq!(request["outward"], true, "{request}");
    let id = request["id"].as_str().unwrap().to_string();
    let (said, err) = door(
        &w,
        "agent_permit",
        json!({"session_id": sid, "request_id": id, "decision": "allow_session"}),
    )
    .await;
    assert!(err, "a session-wide grant for an outward request: {said}");
    assert!(
        !pending(&status(&w, &sid).await).is_empty(),
        "the refused answer left the request open"
    );
    let (said, err) = door(
        &w,
        "agent_permit",
        json!({"session_id": sid, "request_id": id, "decision": "deny"}),
    )
    .await;
    assert!(!err, "{said}");
    let done = status_until(&w, &sid, "the run's end", |s| s["running"] == json!(false)).await;
    assert_eq!(done["reply"], "Did not push.", "{done}");
}

/// A run waiting on its caller is stopped by agent_end op=stop: the wait
/// is released at once (no ten-minute expiry), the request is gone, the
/// model is not called again; op=close with remove_worktree keeps a folder
/// that is no linked worktree.
#[tokio::test]
async fn stop_releases_a_waiting_run_and_close_keeps_what_it_should() {
    let setup = Setup::default().script(
        Agent::Ling,
        vec![tool(
            "Bash",
            json!({"cmd": "git push no-such-remote-door-test main"}),
        )],
    );
    let w = World::start(setup).await;
    let (said, _) = door(
        &w,
        "agent_run",
        json!({"prompt": "Push it.", "write_dir": w.work(), "permission_mode": "acceptEdits"}),
    )
    .await;
    let sid = session_of(&said);
    status_until(&w, &sid, "a pending request", |s| !pending(s).is_empty()).await;

    let (said, err) = door(
        &w,
        "agent_end",
        json!({"op": "stop", "session_id": sid, "remove_worktree": true}),
    )
    .await;
    assert!(
        err && said.contains("belongs to op=close"),
        "stop takes no removal: {said}"
    );
    let (said, err) = door(&w, "agent_end", json!({"op": "stop", "session_id": sid})).await;
    assert!(!err, "{said}");
    let stopped = status_until(&w, &sid, "the stopped run", |s| {
        s["running"] == json!(false)
    })
    .await;
    assert!(pending(&stopped).is_empty(), "{stopped}");
    assert_eq!(
        w.model.calls_of(Agent::Ling).len(),
        1,
        "the model was called again after stop"
    );

    // The write_dir is a repository's own checkout, not a linked worktree.
    let git = |args: &[&str]| {
        let ran = std::process::Command::new("git")
            .args(args)
            .current_dir(w.work())
            .status()
            .unwrap();
        assert!(ran.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@example.invalid",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "start",
    ]);
    let (said, err) = door(
        &w,
        "agent_end",
        json!({"op": "close", "session_id": sid, "remove_worktree": true}),
    )
    .await;
    assert!(err && said.contains("not a linked worktree"), "{said}");
    assert!(
        std::path::Path::new(&w.work()).is_dir(),
        "close removed the write_dir"
    );
}

/// An approved computer — a `machine` row's token, over the LAN — reaches
/// the same `/mcp` and sees the five memory tools only: `memory_upkeep`
/// without `dream_run`, no agent and no browser tool, and a hidden tool is
/// unknown to it. A phone's token is refused at the door.
#[tokio::test]
async fn an_approved_computer_sees_memory_only() {
    let w = World::start(Setup {
        lan: true,
        ..Setup::default()
    })
    .await;
    let rows = json!([
        {"id": "mac-two", "name": "Second Mac", "secret": "tok-mac-two", "created_at": 0, "kind": "machine"},
        {"id": "phone-one", "name": "A phone", "secret": "tok-phone", "created_at": 0, "kind": "phone"},
    ]);
    std::fs::write(
        w.home.linggen_home().join("paired-devices.json"),
        rows.to_string(),
    )
    .unwrap();
    // The engine trusts its cached device list for a second before it looks
    // at the file again.
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let ip = {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        socket.connect("192.0.2.1:9").unwrap();
        socket.local_addr().unwrap().ip()
    };
    let url = format!("http://{ip}:{}/mcp", w.engine.port);
    let client = reqwest::Client::new();
    let post = |token: &'static str, body: Value| {
        let req = client
            .post(url.clone())
            .header("x-linggen-device", token)
            .json(&body);
        async move {
            let resp = req.send().await.expect("LAN POST");
            let status = resp.status().as_u16();
            (status, resp.json::<Value>().await.unwrap_or(Value::Null))
        }
    };
    let (status, list) = post(
        "tok-mac-two",
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    assert_eq!(status, 200, "{list}");
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "memory_search",
            "memory_add",
            "memory_update",
            "memory_delete",
            "memory_upkeep"
        ]
    );
    let ops = &list["result"]["tools"][4]["inputSchema"]["properties"]["op"]["enum"];
    assert!(
        !ops.as_array().unwrap().contains(&json!("dream_run")),
        "{ops}"
    );
    let (_, init) = post(
        "tok-mac-two",
        json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {}}),
    )
    .await;
    assert!(
        !init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("agent_run"),
        "{init}"
    );
    let (_, ran) = post("tok-mac-two", rpc("agent_run", json!({"prompt": "hi"}))).await;
    assert_eq!(ran["error"]["message"], "unknown tool: agent_run", "{ran}");
    let (_, ran) = post("tok-mac-two", rpc("app", json!({"op": "list"}))).await;
    assert_eq!(ran["error"]["message"], "unknown tool: app", "an approved computer has no app tool: {ran}");
    let (_, days) = post("tok-mac-two", rpc("memory_upkeep", json!({"op": "days"}))).await;
    assert_eq!(days["result"]["isError"], false, "{days}");
    let (status, _) = post(
        "tok-phone",
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    assert_eq!(status, 403, "a phone is not let in");
}
