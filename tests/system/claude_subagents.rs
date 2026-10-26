//! SDK child events reuse the shared subagent view without native inference.
use crate::support::engine::RunEnd;
use crate::support::{Setup, World};
use serde_json::{json, Value};

const MODEL: &str = r#"
[[models]]
id = "claude-key"
provider = "anthropic"
url = "http://127.0.0.1:9"
model = "claude-test"
api_key = "fixture-key"
supports_tools = true
"#;
const BRIDGE: &str = r#"#!/bin/sh
[ "$1" = install ] && exit 0
IFS= read -r start
emit() { printf '%s\n' "$1"; }
emit '{"type":"init","session_id":"sdk-children","permissionMode":"default","commands":[]}'
emit '{"type":"tool_start","id":"launch","tool":"Agent","args":{"prompt":"Review fixture"}}'
emit '{"type":"subagent_start","agent_id":"a","name":"reviewer","task":"Review fixture"}'
case "$start" in
*stop-child*)
 IFS= read -r command
 case "$command" in *'"type":"stop_task"'*'"task_id":"a"'*) :;; *) exit 2;; esac
 id=$(printf '%s' "$command" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
 emit "{\"type\":\"mode_result\",\"id\":$id,\"ok\":true}"
 emit '{"type":"subagent_end","agent_id":"a","status":"stopped","text":"Stopped by user"}'
 ;;
*)
 emit '{"type":"subagent_start","agent_id":"b","name":"reviewer","task":"Review another fixture"}'
 emit '{"type":"subagent_text","agent_id":"a","text":"PRIVATE_A"}'
 emit '{"type":"tool_start","agent_id":"a","id":"read-a","tool":"Read","args":{"file_path":"fixture.txt"}}'
 emit '{"type":"tool_end","agent_id":"a","id":"read-a","tool":"Read","result":"fixture data","failed":false}'
 emit '{"type":"subagent_text","agent_id":"b","text":"PRIVATE_B"}'
 emit '{"type":"subagent_end","agent_id":"a","status":"completed"}'
 emit '{"type":"subagent_end","agent_id":"b","status":"completed"}'
 ;;
esac
emit '{"type":"tool_end","id":"launch","tool":"Agent","result":"children complete","failed":false}'
emit '{"type":"text","text":"MAIN_ANSWER"}'
case "$start" in *'"resume":"sdk-children"'*) cost=0.25;; *) cost=0.125;; esac
emit "{\"type\":\"result\",\"success\":true,\"text\":\"MAIN_ANSWER\",\"session_id\":\"sdk-children\",\"cost\":$cost,\"usage\":{}}"
emit '{"type":"done"}'
"#;
async fn world() -> (World, String) {
    let w = World::start(Setup::default()).await;
    let mut config = w.api.get("/api/config").await;
    let extra: Value = toml::from_str(MODEL).unwrap();
    config["models"]
        .as_array_mut()
        .unwrap()
        .push(extra["models"][0].clone());
    w.api.post("/api/config", config).await;
    let bun = w.home.linggen_home().join("bin/bun");
    std::fs::create_dir_all(bun.parent().unwrap()).unwrap();
    std::fs::write(&bun, BRIDGE).unwrap();
    std::fs::set_permissions(&bun, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let sid = w
        .api
        .create_session(
            json!({"title":"SDK children", "runtime":"claude_agent_sdk", "project_root":w.work()}),
        )
        .await;
    (w, sid)
}
async fn finish(w: &World, sid: &str, mark: crate::support::engine::Mark) {
    let api = w.api.clone();
    let id = sid.to_string();
    let end = w
        .engine
        .wait_run(Some(sid), "claude", mark, || {
            let api = api.clone();
            let id = id.clone();
            async move { api.pending(&id, None).await }
        })
        .await;
    assert!(matches!(end, RunEnd::Finished));
}
fn trees(state: &Value) -> Vec<Value> {
    state["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| row[0]["subagents"].as_array().cloned().unwrap_or_default())
        .collect()
}
#[tokio::test]
async fn child_text_and_tools_stay_nested_live_and_after_reload() {
    let (w, sid) = world().await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "children").await;
    finish(&w, &sid, mark).await;
    let state = w.api.history(&sid, &w.work()).await;
    let rows = state["messages"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "child messages leaked: {state}");
    assert_eq!(rows[1][1], "MAIN_ANSWER");
    let entries = trees(&state);
    assert_eq!(entries.len(), 2);
    assert_ne!(entries[0]["subagentId"], entries[1]["subagentId"]);
    assert_eq!(entries[0]["resultText"], "PRIVATE_A");
    assert_eq!(entries[0]["toolSteps"][0]["status"], "done");
    assert_eq!(entries[0]["status"], "done");
    let events = w.events();
    assert_eq!(
        events
            .iter()
            .filter(|event| event["phase"] == "subagent_spawned")
            .count(),
        2
    );
    assert!(events.iter().any(|event| event["kind"] == "message"
        && event["text"] == "PRIVATE_A"
        && event["data"]["parent_agent_id"] == "claude"));
    assert!(!w.rows(&sid).iter().any(|row| row
        .said_by("claude")
        .is_some_and(|text| text.contains("PRIVATE_"))));
    w.assert_all_scripted();
}
#[tokio::test]
async fn stopping_a_child_uses_sdk_control_and_leaves_its_parent_running() {
    let (w, sid) = world().await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "stop-child").await;
    let mut id = None;
    for _ in 0..100 {
        id = w
            .events()
            .iter()
            .find(|event| event["phase"] == "subagent_spawned")
            .and_then(|event| event["data"]["subagent_run_id"].as_str().map(str::to_owned));
        if id.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    w.api
        .post(
            "/api/agent-cancel",
            json!({"run_id":id.expect("child did not start")}),
        )
        .await;
    finish(&w, &sid, mark).await;
    let state = w.api.history(&sid, &w.work()).await;
    assert_eq!(trees(&state)[0]["status"], "failed");
    assert!(state["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row[1] == "MAIN_ANSWER"));
    w.assert_all_scripted();
}

#[tokio::test]
async fn session_estimate_accumulates_turns_and_clear_resets() {
    let (w, sid) = world().await;
    for turn in 1..=2 {
        let mark = w.mark();
        w.api.chat(&sid, &w.work(), "children").await;
        finish(&w, &sid, mark).await;
        let state = w.api.history(&sid, &w.work()).await;
        assert_eq!(
            state["claude_usage"]["estimated_cost_usd"],
            0.125 * turn as f64
        );
    }
    w.api
        .post("/api/chat/clear", json!({"session_id":sid}))
        .await;
    let state = w.api.history(&sid, &w.work()).await;
    assert!(state["claude_usage"].is_null());
    assert!(trees(&state).is_empty());
    w.assert_all_scripted();
}
