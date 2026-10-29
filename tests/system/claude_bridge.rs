//! The Claude session host against a stand-in bridge: a `bun` script in
//! `LINGGEN_HOME/bin` that speaks the runner's JSON-lines protocol, so the
//! host's event loop, interaction widgets and per-session files are
//! exercised without the SDK, Bun or a network.

use crate::support::engine::RunEnd;
use crate::support::{Setup, World};
use serde_json::{json, Value};

const MODEL: &str = r#"
[[models]]
id = "claude-key"
provider = "anthropic"
url = "http://127.0.0.1:9"
model = "claude-test"
api_key = "test-not-a-key"
context_window = 32000
supports_tools = true
"#;

/// Picks its scenario from the prompt in the `start` line.
const BRIDGE: &str = r#"#!/bin/sh
[ "$1" = install ] && exit 0
IFS= read -r start
printf '%s\n' "$start" >> bridge-starts.jsonl
emit() { printf '%s\n' "$1"; }
init() { emit '{"type":"init","session_id":"sdk-1","model":"m","permissionMode":"default","commands":[{"name":"clear"}]}'; }
case "$start" in
*'"prompt":"crash"'*) echo "bridge exploded" >&2; exit 1;;
*'"prompt":"fail"'*)
  init
  emit '{"type":"result","success":false,"errors":["budget exhausted"]}'
  exit 0;;
*'"prompt":"ask"'*)
  init
  emit '{"type":"request","kind":"permission","id":1,"input":{"tool":"Bash","args":{"command":"ls"},"suggestions":[]}}'
  IFS= read -r reply
  case "$reply" in *'"allow":false'*) word=denied;; *) word=allowed;; esac
  emit "{\"type\":\"text\",\"text\":\"tool $word\"}";;
*'"prompt":"mode'*)
  init
  emit '{"type":"text","text":"ready"}'
  while IFS= read -r line; do
    case "$line" in *'"type":"mode"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      case "$start" in
      *modefail*) emit "{\"type\":\"mode_result\",\"id\":$id,\"ok\":false,\"error\":\"mode unavailable here\"}";;
      *) emit "{\"type\":\"mode_result\",\"id\":$id,\"ok\":true}"
         emit '{"type":"status","permissionMode":"acceptEdits"}';;
      esac
      break;;
    esac
  done;;
*'"prompt":"withdraw"'*)
  init
  emit '{"type":"request","kind":"permission","id":1,"input":{"tool":"Bash","args":{"command":"ls"},"suggestions":[]}}'
  sleep 1
  emit '{"type":"cancel_request","id":1}'
  emit '{"type":"text","text":"withdrawn"}';;
*'"prompt":"hang"'*)
  init
  sleep 600 &
  echo $! > bridge-child.pid
  emit '{"type":"text","text":"ready"}'
  wait;;
*'"prompt":"reset"'*)
  init
  emit '{"type":"text","text":"before reset"}'
  emit '{"type":"reset","session_id":"sdk-2"}'
  emit '{"type":"text","text":"after reset"}';;
*)
  init
  resume=$(printf '%s' "$start" | sed -n 's/.*"resume":"\([^"]*\)".*/\1/p')
  emit "{\"type\":\"text\",\"text\":\"resume=$resume\"}";;
esac
emit '{"type":"result","success":true,"text":"","usage":{"input_tokens":1,"output_tokens":1}}'
case "$start" in
*'"prompt":"hint"'*) emit '{"type":"suggestion","text":"run the tests"}';;
esac
emit '{"type":"done"}'
"#;

async fn world() -> (World, String) {
    let w = world_with(Setup::default()).await;
    let sid = session_in(&w, &w.work()).await;
    (w, sid)
}

/// A world whose `bun` is the stand-in bridge.
async fn world_with(setup: Setup) -> World {
    let w = World::start(Setup {
        config_append: Some(MODEL),
        ..setup
    })
    .await;
    let bin = w.home.linggen_home().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let bun = bin.join("bun");
    std::fs::write(&bun, BRIDGE).unwrap();
    std::fs::set_permissions(&bun, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    crate::claude_host::trust_folder(&w);
    w
}

async fn session_in(w: &World, root: &str) -> String {
    w.api
        .create_session(
            json!({"title":"Claude", "runtime":"claude_agent_sdk", "project_root":root}),
        )
        .await
}

/// The `start` line the host sent for the latest turn run in `root`.
fn last_start(root: &str) -> Value {
    let text = std::fs::read_to_string(std::path::Path::new(root).join("bridge-starts.jsonl"))
        .expect("the bridge never started");
    let line = text
        .lines()
        .filter(|l| !l.contains("\"validateOnly\":true"))
        .last()
        .unwrap();
    serde_json::from_str(line).unwrap()
}

async fn wait(w: &World, sid: &str, mark: crate::support::engine::Mark) -> RunEnd {
    let api = w.api.clone();
    let sid = sid.to_string();
    w.engine
        .wait_run(Some(&sid.clone()), "claude", mark, || {
            let (api, sid) = (api.clone(), sid.clone());
            async move { api.pending(&sid, Some("claude")).await }
        })
        .await
}

async fn turn(w: &World, sid: &str, message: &str) {
    let mark = w.mark();
    w.api.chat(sid, &w.work(), message).await;
    match wait(w, sid, mark).await {
        RunEnd::Finished => {}
        RunEnd::Asked(ask) => panic!("unexpected question: {ask}"),
    }
}

/// Wait until the bridge has said `line` mid-turn.
async fn until_said(w: &World, sid: &str, line: &str) {
    for _ in 0..100 {
        let rows = w.rows_opt(sid).unwrap_or_default();
        if rows.iter().any(|r| r.said_by("claude") == Some(line)) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the bridge never said {line:?}");
}

/// Wait for the run to finish, however many questions it shows meanwhile.
async fn until_finished(w: &World, sid: &str, mark: crate::support::engine::Mark) {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(20) {
        if let RunEnd::Finished = wait(w, sid, mark).await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the run kept asking");
}

fn meta(w: &World, sid: &str) -> Value {
    let path = w
        .home
        .linggen_home()
        .join("sessions")
        .join(sid)
        .join("session.yaml");
    serde_norway::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

async fn set_mode(w: &World, sid: &str, mode: &str) -> reqwest::Response {
    reqwest::Client::new()
        .patch(format!("{}/api/sessions", w.engine.base_url()))
        .json(&json!({"session_id":sid, "claude_permission_mode":mode}))
        .send()
        .await
        .unwrap()
}

fn said(w: &World, sid: &str) -> Vec<String> {
    w.rows(sid)
        .iter()
        .filter_map(|r| r.said_by("claude").map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn a_second_turn_resumes_the_sdk_conversation_and_commands_are_cached() {
    let (w, sid) = world().await;
    turn(&w, &sid, "hello").await;
    assert_eq!(said(&w, &sid), ["resume="]);
    turn(&w, &sid, "again").await;
    assert_eq!(said(&w, &sid), ["resume=", "resume=sdk-1"]);
    let caps = w
        .api
        .get(&format!(
            "/api/sessions/claude-capabilities?session_id={sid}"
        ))
        .await;
    assert_eq!(caps["commands"][0]["name"], "clear");
}

#[tokio::test]
async fn the_sdks_guess_at_the_next_message_is_the_inputs_hint() {
    let (w, sid) = world().await;
    turn(&w, &sid, "hint").await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let hint = loop {
        let found = w.events().into_iter().find(|e| e["kind"] == "followups" && e["session_id"] == sid);
        if let Some(event) = found {
            break event;
        }
        assert!(std::time::Instant::now() < deadline, "no hint followed the turn");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert_eq!(hint["data"]["items"], serde_json::json!(["run the tests"]));
    assert!(hint["data"]["run_id"].is_string(), "the hint names the turn it follows: {hint}");
}

#[tokio::test]
async fn a_bridge_crash_reports_what_it_printed() {
    let (w, sid) = world().await;
    turn(&w, &sid, "crash").await;
    let last = said(&w, &sid).pop().unwrap();
    assert!(last.starts_with("Error:"), "{last}");
    assert!(last.contains("bridge exploded"), "{last}");
}

#[tokio::test]
async fn a_failed_result_is_an_error_not_a_reply() {
    let (w, sid) = world().await;
    turn(&w, &sid, "fail").await;
    let last = said(&w, &sid).pop().unwrap();
    assert!(last.contains("budget exhausted"), "{last}");
}

#[tokio::test]
async fn a_denied_permission_reaches_the_bridge_and_auto_is_not_offered_unverified() {
    let (w, sid) = world().await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "ask").await;
    let RunEnd::Asked(ask) = wait(&w, &sid, mark).await else {
        panic!("the bridge asked nothing");
    };
    let labels: Vec<&str> = ask["questions"][0]["options"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|o| o["label"].as_str())
        .collect();
    assert_eq!(labels, ["Allow once", "Deny"]);
    w.api.answer(&ask, "Deny").await;
    match wait(&w, &sid, mark).await {
        RunEnd::Finished => {}
        RunEnd::Asked(ask) => panic!("asked twice: {ask}"),
    }
    assert_eq!(said(&w, &sid).pop().as_deref(), Some("tool denied"));
}

#[tokio::test]
async fn an_sdk_reset_clears_the_transcript_and_moves_the_resume_pointer() {
    let (w, sid) = world().await;
    turn(&w, &sid, "reset").await;
    assert_eq!(said(&w, &sid), ["after reset"]);
    let checkpoint: Value = serde_json::from_slice(
        &std::fs::read(
            w.home
                .linggen_home()
                .join("sessions")
                .join(&sid)
                .join("claude.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(checkpoint["session_id"], "sdk-2");
}

#[tokio::test]
async fn a_live_mode_change_waits_for_the_sdk_acknowledgement() {
    let (w, sid) = world().await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "mode").await;
    until_said(&w, &sid, "ready").await;
    let response = set_mode(&w, &sid, "acceptEdits").await;
    assert_eq!(response.status(), 200);
    until_finished(&w, &sid, mark).await;
    assert_eq!(meta(&w, &sid)["claude_permission_mode"], "acceptEdits");
}

#[tokio::test]
async fn a_mode_the_sdk_refuses_is_not_saved() {
    let (w, sid) = world().await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "modefail").await;
    until_said(&w, &sid, "ready").await;
    let response = set_mode(&w, &sid, "plan").await;
    assert_eq!(response.status(), 409);
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("mode unavailable here"));
    until_finished(&w, &sid, mark).await;
    assert_ne!(meta(&w, &sid)["claude_permission_mode"], "plan");
}

#[tokio::test]
async fn a_withdrawn_sdk_request_closes_its_widget() {
    let (w, sid) = world().await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "withdraw").await;
    let RunEnd::Asked(_) = wait(&w, &sid, mark).await else {
        panic!("the bridge asked nothing");
    };
    until_finished(&w, &sid, mark).await;
    assert!(w.api.pending(&sid, Some("claude")).await.is_empty());
    assert_eq!(said(&w, &sid).pop().as_deref(), Some("withdrawn"));
}

/// The bridge ignores stdin and never exits: clear must still cancel the
/// turn, kill its process group and drop the resume pointer.
#[tokio::test]
async fn clear_stops_a_stuck_bridge_and_drops_the_resume_pointer() {
    let (w, sid) = world().await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "hang").await;
    until_said(&w, &sid, "ready").await;
    let checkpoint = w
        .home
        .linggen_home()
        .join("sessions")
        .join(&sid)
        .join("claude.json");
    assert!(checkpoint.exists());
    let started = std::time::Instant::now();
    w.api
        .post("/api/chat/clear", json!({"session_id":sid}))
        .await;
    assert!(
        started.elapsed() < std::time::Duration::from_secs(15),
        "clear took {:?}",
        started.elapsed()
    );
    until_finished(&w, &sid, mark).await;
    assert!(!checkpoint.exists());
    let pid =
        std::fs::read_to_string(std::path::Path::new(&w.work()).join("bridge-child.pid")).unwrap();
    let alive = || {
        std::process::Command::new("ps")
            .args(["-p", pid.trim()])
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    let start = std::time::Instant::now();
    while alive() && start.elapsed() < std::time::Duration::from_secs(3) {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(!alive(), "the bridge's child {} survived", pid.trim());
}

/// A Claude session starts with the memory block a native owner session
/// gets, and every turn carries the capture nudge — except a slash command,
/// which goes to the SDK as-is.
#[tokio::test]
async fn a_claude_turn_carries_the_owner_memory_block_and_capture_nudge() {
    let (w, sid) = world().await;
    turn(&w, &sid, "hello there").await;
    let start = last_start(&w.work());
    let session = start["sessionMemory"].as_str().unwrap();
    assert!(
        session.contains("# Always-on memory")
            && session.contains("## Usage rules for the always-on rows"),
        "not the native owner block: {session}"
    );
    let nudge = start["turnMemory"].as_str().unwrap();
    assert!(
        nudge.contains("Memory capture:"),
        "no capture nudge: {nudge}"
    );
    turn(&w, &sid, "/compact").await;
    assert_eq!(last_start(&w.work())["turnMemory"], "");
}

/// Same store, same scope rules as a native session: core and the project's
/// index at session start, the matching row recalled for the turn and shown
/// in the chat as the recall widget.
#[tokio::test]
async fn a_claude_session_sees_core_index_and_recall_like_a_native_one() {
    if !crate::support::memory::embedder_available() {
        eprintln!("SKIPPED (passes on nothing): no embedding model in ~/.cache/huggingface");
        return;
    }
    let w = world_with(Setup {
        embedder: true,
        memory_rows: Some("alex.json"),
        ..Setup::default()
    })
    .await;
    let harbor = w.home.work().join("harbor");
    std::fs::create_dir_all(harbor.join(".git")).unwrap();
    let harbor = harbor.display().to_string();
    let sid = session_in(&w, &harbor).await;
    turn(&w, &sid, "Which indentation does Harbor code use?").await;
    let start = last_start(&harbor);
    let session = start["sessionMemory"].as_str().unwrap();
    assert!(
        session.contains("Alex lives in Lisbon"),
        "no core: {session}"
    );
    assert!(
        session.contains("Harbor: tabs, not spaces"),
        "no index: {session}"
    );
    assert!(
        !session.contains("Lantern"),
        "a sibling project leaked: {session}"
    );
    let recall = start["turnMemory"].as_str().unwrap();
    assert!(recall.contains("From memory"), "nothing recalled: {recall}");
    assert!(
        recall.contains("tabs for indentation"),
        "wrong recall: {recall}"
    );
    assert!(
        w.rows(&sid).iter().any(|r| r.from_id == "memory-recall"),
        "no recall row in the chat"
    );
}

async fn answer_as(w: &World, ask: &Value, label: &str, who: Value) -> reqwest::StatusCode {
    let mut payload = json!({
        "question_id": ask["question_id"],
        "answers": [{"question_index": 0, "selected": [label], "custom_text": null}],
    });
    for (k, v) in who.as_object().unwrap() {
        payload[k] = v.clone();
    }
    reqwest::Client::new()
        .post(format!("{}/api/ask-user-response", w.engine.base_url()))
        .json(&payload)
        .send()
        .await
        .unwrap()
        .status()
}

/// A room consumer cannot approve the owner's SDK call, and a label the
/// widget never offered allows nothing.
#[tokio::test]
async fn only_the_owner_approves_and_only_with_an_offered_choice() {
    let (w, sid) = world().await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "ask").await;
    let RunEnd::Asked(ask) = wait(&w, &sid, mark).await else {
        panic!("the bridge asked nothing");
    };
    let consumer = json!({"user_type":"consumer", "user_id":"guest"});
    // Refused exactly like an unknown id: nothing says the prompt exists.
    assert_eq!(answer_as(&w, &ask, "Allow once", consumer).await, 404);
    assert_eq!(
        w.api.pending(&sid, Some("claude")).await.len(),
        1,
        "the question survived"
    );
    let forged = "Allow and switch to acceptEdits";
    assert_eq!(answer_as(&w, &ask, forged, json!({})).await, 200);
    until_finished(&w, &sid, mark).await;
    assert_eq!(said(&w, &sid).pop().as_deref(), Some("tool denied"));
}
