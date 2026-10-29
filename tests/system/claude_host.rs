//! Hosting a Claude session: widgets only the session's owner answers,
//! folder trust before project settings load, follow-ups that wait for the
//! turn, a Stop that lands while tokens stream, and session approvals that
//! live in memory. The SDK is a shell fixture speaking the bridge protocol.
use crate::support::engine::RunEnd;
use crate::support::{Setup, World};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const MODEL: &str = r#"
[[models]]
id = "claude-key"
provider = "anthropic"
url = "http://127.0.0.1:9"
model = "claude-test"
api_key = "fixture-key"
supports_tools = true
"#;
/// The conversation id the bridge fixture's SDK reports.
const SDK_ID: &str = "5d1c2a9e-7b3f-4c1d-9e2a-0f6b8c4d2e10";
const BRIDGE: &str = r#"#!/bin/sh
[ "$1" = install ] && exit 0
IFS= read -r start
emit() { printf '%s\n' "$1"; }
case "$start" in *'"validateOnly":true'*)
 emit '{"type":"mode_result","ok":true,"commands":[]}'
 exit 0;;
esac
printf '%s\n' "$start" >> "$PWD/.sdk-starts.log"
env | cut -d= -f1 | sort > "$PWD/.sdk-env.log"
finish() {
 emit "{\"type\":\"text\",\"text\":\"$1\"}"
 emit "{\"type\":\"result\",\"success\":true,\"text\":\"$1\",\"session_id\":\"5d1c2a9e-7b3f-4c1d-9e2a-0f6b8c4d2e10\",\"usage\":{}}"
 emit '{"type":"done"}'
}
RULE='{"type":"addRules","behavior":"allow","destination":"session","rules":[{"toolName":"Bash","ruleContent":"env"}]}'
emit '{"type":"init","session_id":"5d1c2a9e-7b3f-4c1d-9e2a-0f6b8c4d2e10","permissionMode":"default","commands":[]}'
case "$start" in
*'"prompt":"ask"'*)
 emit "{\"type\":\"request\",\"kind\":\"permission\",\"id\":1,\"input\":{\"tool\":\"Bash\",\"args\":{\"command\":\"env\"},\"suggestions\":[$RULE]}}"
 IFS= read -r reply
 case "$reply" in *'"remember":true'*) :;; *) exit 3;; esac
 emit "{\"type\":\"request\",\"kind\":\"permission_updates\",\"id\":2,\"input\":{\"updates\":[$RULE]}}"
 IFS= read -r saved
 finish ANSWERED;;
*'"prompt":"wait"'*)
 emit '{"type":"tool_start","id":"t1","tool":"Bash","args":{"command":"make"}}'
 emit '{"type":"request","kind":"permission","id":1,"input":{"tool":"Bash","args":{"command":"make"},"suggestions":[]}}'
 IFS= read -r reply
 finish WAITED;;
*'"prompt":"steer"'*)
 emit '{"type":"text","text":"WORKING"}'
 while IFS= read -r line; do
  case "$line" in *'"type":"steer"'*)
   printf '%s\n' "$line" >> "$PWD/.sdk-steers.log"
   id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
   emit "{\"type\":\"mode_result\",\"id\":$id,\"ok\":true}"
   break;;
  esac
 done
 finish STEERED;;
*'"prompt":"late"'*)
 while IFS= read -r -t 1 line; do
  case "$line" in *'"type":"steer"'*) printf '%s\n' "$line" >> "$PWD/.sdk-steers.log";; esac
 done
 finish LATE;;
*'"prompt":"bg"'*)
 emit '{"type":"tasks","tasks":[{"id":"t1","kind":"shell","title":"sleep 600"}]}'
 emit '{"type":"task","id":"t1","activity":"tick"}'
 while IFS= read -r line; do
  case "$line" in *'"type":"stop_task"'*)
   printf '%s\n' "$line" >> "$PWD/.sdk-stops.log"
   id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
   emit "{\"type\":\"mode_result\",\"id\":$id,\"ok\":true}"
   emit '{"type":"task","id":"t1","status":"stopped","summary":"stopped by the person"}'
   emit '{"type":"tasks","tasks":[]}'
   break;;
  esac
 done
 finish STOPPED;;
*'"prompt":"live"'*)
 uuid=$(printf '%s' "$start" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p')
 emit '{"type":"text","text":"FIRST"}'
 emit "{\"type\":\"result\",\"success\":true,\"usage\":{},\"uuids\":[\"$uuid\"]}"
 while IFS= read -r line; do
  case "$line" in
  *'"type":"prompt"'*)
   u=$(printf '%s' "$line" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p')
   emit '{"type":"text","text":"SECOND"}'
   emit "{\"type\":\"result\",\"success\":true,\"usage\":{},\"uuids\":[\"$u\"]}"
   case "$line" in *'"text":"notify"'*)
    emit '{"type":"text","text":"TASK ENDED"}'
    emit '{"type":"result","success":true,"usage":{},"uuids":[]}';;
   esac;;
  *'"type":"end"'*)
   printf 'end\n' >> "$PWD/.sdk-ends.log"
   emit '{"type":"done"}'
   exit 0;;
  esac
 done
 exit 0;;
*'"prompt":"child"'*)
 uuid=$(printf '%s' "$start" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p')
 emit '{"type":"text","text":"LAUNCHED"}'
 emit "{\"type\":\"result\",\"success\":true,\"usage\":{},\"uuids\":[\"$uuid\"]}"
 sleep 1
 emit '{"type":"subagent_start","agent_id":"c1","name":"counter","task":"count slowly"}'
 emit '{"type":"subagent_text","agent_id":"c1","text":"1"}'
 emit '{"type":"tool_start","agent_id":"c1","tool":"Bash","args":{"command":"sleep 2"},"id":"tc1"}'
 while IFS= read -r line; do
  case "$line" in
  *'"type":"prompt"'*)
   u=$(printf '%s' "$line" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p')
   emit '{"type":"subagent_text","agent_id":"c1","text":"2"}'
   emit '{"type":"text","text":"SECOND"}'
   emit "{\"type\":\"result\",\"success\":true,\"usage\":{},\"uuids\":[\"$u\"]}";;
  *'"type":"end"'*)
   emit '{"type":"done"}'
   exit 0;;
  esac
 done
 exit 0;;
*'"prompt":"stream"'*)
 while :; do
  emit '{"type":"token","text":"x"}'
  if IFS= read -r -t 1 line; then
   case "$line" in *'"type":"interrupt"'*)
    id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
    emit "{\"type\":\"mode_result\",\"id\":$id,\"ok\":true}"
    emit '{"type":"result","success":false,"errors":["interrupted"],"usage":{}}'
    emit '{"type":"done"}'
    exit 0;;
   esac
  fi
 done;;
*'"prompt":"move"'*)
 emit '{"type":"request","kind":"permission","id":1,"input":{"tool":"Bash","args":{"command":"make"},"suggestions":[]}}'
 IFS= read -r reply
 emit "{\"type\":\"cwd\",\"cwd\":\"$PWD/notes\"}"
 emit '{"type":"request","kind":"permission","id":2,"input":{"tool":"Bash","args":{"command":"make"},"suggestions":[]}}'
 IFS= read -r reply
 finish MOVED;;
*'"prompt":"cd"'*)
 emit "{\"type\":\"cwd\",\"cwd\":\"$PWD/repo/src\"}"
 emit "{\"type\":\"cwd\",\"cwd\":\"$PWD/notes\"}"
 finish MOVED;;
*) finish HELLO;;
esac
"#;

/// Mark the world's work folder trusted, as a past "Yes" would have.
pub fn trust_folder(w: &World) {
    let file = w
        .home
        .linggen_home()
        .join("runtimes/claude-agent-sdk/trusted-folders.json");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let work = std::path::Path::new(&w.work()).canonicalize().unwrap();
    std::fs::write(file, serde_json::to_vec(&json!([work])).unwrap()).unwrap();
}

async fn world() -> (World, String) {
    world_with(Setup::default()).await
}

async fn world_with(setup: Setup) -> (World, String) {
    let w = World::start(setup).await;
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
    // The folder carries Claude configuration of its own, so trust is asked.
    std::fs::write(std::path::Path::new(&w.work()).join("CLAUDE.md"), "project rules").unwrap();
    let sid = w
        .api
        .create_session(
            json!({"title":"SDK host", "runtime":"claude_agent_sdk", "project_root":w.work()}),
        )
        .await;
    (w, sid)
}

async fn next(w: &World, sid: &str, mark: crate::support::engine::Mark) -> RunEnd {
    let api = w.api.clone();
    let id = sid.to_string();
    w.engine
        .wait_run(Some(sid), "claude", mark, || {
            let api = api.clone();
            let id = id.clone();
            async move { api.pending(&id, None).await }
        })
        .await
}

fn asked(end: RunEnd) -> Value {
    match end {
        RunEnd::Asked(ask) => ask,
        RunEnd::Finished => panic!("the turn finished without asking"),
    }
}

fn starts(w: &World) -> Vec<Value> {
    std::fs::read_to_string(std::path::Path::new(&w.work()).join(".sdk-starts.log"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn options(ask: &Value) -> Vec<String> {
    ask["questions"][0]["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["label"].as_str().unwrap().to_string())
        .collect()
}

fn finished_with(w: &World, mark: crate::support::engine::Mark, sid: &str) -> Vec<String> {
    w.engine
        .log_since(mark)
        .lines()
        .filter(|l| l.contains("run/finish") && l.contains(sid))
        .filter_map(|l| l.split("status=").nth(1))
        .map(|rest| rest.split_whitespace().next().unwrap_or("").to_string())
        .collect()
}

#[tokio::test]
async fn trust_then_an_owner_only_permission_with_session_rules_in_memory() {
    let (w, sid) = world().await;
    // A rules file an older build left behind is never loaded again.
    let legacy = w
        .home
        .linggen_home()
        .join("sessions")
        .join(&sid)
        .join("claude-permissions.json");
    std::fs::write(
        &legacy,
        r#"[{"type":"addRules","behavior":"allow","rules":[{"toolName":"Write"}]}]"#,
    )
    .unwrap();
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "ask").await;

    let trust = asked(next(&w, &sid, mark).await);
    assert_eq!(trust["questions"][0]["header"], "Folder trust");
    assert!(
        starts(&w).is_empty(),
        "the SDK started before the folder was trusted"
    );
    w.api.answer(&trust, "Yes, trust").await;

    let ask = asked(next(&w, &sid, mark).await);
    let id = ask["question_id"].as_str().unwrap();
    assert!(
        id.starts_with("sdk-") && uuid::Uuid::parse_str(&id[4..]).is_ok(),
        "guessable widget id {id}"
    );
    assert!(options(&ask).contains(&"Allow and switch to auto".to_string()));
    let answer = |user_type: &str, user_id: &str| {
        json!({"question_id":id, "user_type":user_type, "user_id":user_id,
            "answers":[{"question_index":0, "selected":["Allow once"], "custom_text":null}]})
    };
    // A proxy-room consumer cannot answer the owner's prompt, and is told
    // nothing it could not learn from a made-up id.
    assert_eq!(
        w.api
            .post_status("/api/ask-user-response", answer("consumer", "room-user"))
            .await,
        404
    );
    assert_eq!(
        w.api
            .post_status("/api/ask-user-response", answer("pairing", ""))
            .await,
        404
    );
    assert_eq!(
        w.api.pending(&sid, None).await.len(),
        1,
        "the prompt was taken"
    );
    w.api.answer(&ask, "Allow for this session").await;
    assert!(matches!(next(&w, &sid, mark).await, RunEnd::Finished));

    let first = &starts(&w)[0];
    assert_eq!(first["settingSources"], json!(["user", "project", "local"]));
    assert_eq!(first["permissionUpdates"], json!([]));
    assert!(!legacy.exists());
    let env =
        std::fs::read_to_string(std::path::Path::new(&w.work()).join(".sdk-env.log")).unwrap();
    for leaked in ["LINGGEN_HOME", "LING_MEM_URL", "LINGGEN_EVENT_LOG"] {
        assert!(
            !env.lines().any(|k| k == leaked),
            "{leaked} reached the SDK"
        );
    }
    assert!(env.lines().any(|k| k == "ANTHROPIC_API_KEY"));

    // The approval holds for the next turn of this session, from memory.
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "hello").await;
    assert!(matches!(next(&w, &sid, mark).await, RunEnd::Finished));
    assert_eq!(
        starts(&w)[1]["permissionUpdates"][0]["rules"][0]["toolName"],
        "Bash"
    );
    assert!(!legacy.exists());

    // Clear ends the session's approvals with its conversation.
    w.api
        .post("/api/chat/clear", json!({"session_id":sid}))
        .await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "hello").await;
    assert!(matches!(next(&w, &sid, mark).await, RunEnd::Finished));
    assert_eq!(starts(&w)[2]["permissionUpdates"], json!([]));
}

#[tokio::test]
async fn a_declined_folder_runs_on_user_settings_only() {
    let (w, sid) = world().await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "hello").await;
    let trust = asked(next(&w, &sid, mark).await);
    w.api.answer(&trust, "No, run without").await;
    assert!(matches!(next(&w, &sid, mark).await, RunEnd::Finished));
    assert_eq!(starts(&w)[0]["settingSources"], json!(["user"]));
    // The answer holds for this session: no second prompt.
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "hello").await;
    assert!(matches!(next(&w, &sid, mark).await, RunEnd::Finished));
    assert_eq!(starts(&w)[1]["settingSources"], json!(["user"]));
    // Kept with the session, not in engine memory: a restart does not ask again.
    let declined = w
        .home
        .linggen_home()
        .join("sessions")
        .join(&sid)
        .join("claude-trust-declined.json");
    assert!(declined.exists());
    // Clear ends the conversation, and the answer with it.
    w.api.post("/api/chat/clear", json!({"session_id":sid})).await;
    assert!(!declined.exists());
}

#[tokio::test]
async fn a_folder_without_claude_configuration_is_not_asked_about() {
    let (w, sid) = world().await;
    std::fs::remove_file(std::path::Path::new(&w.work()).join("CLAUDE.md")).unwrap();
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "hello").await;
    assert!(matches!(next(&w, &sid, mark).await, RunEnd::Finished));
    assert_eq!(starts(&w)[0]["settingSources"], json!(["user"]));
}

#[tokio::test]
async fn a_follow_up_waits_for_the_running_turn_instead_of_cancelling_it() {
    let (w, sid) = world().await;
    trust_folder(&w);
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "wait").await;
    let ask = asked(next(&w, &sid, mark).await);
    // A page loaded now sees the live run's call so far, still running.
    let history = w.api.history(&sid, &w.work()).await;
    let live = history["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row[0]["run"] == "running")
        .unwrap_or_else(|| panic!("no live run row in {history:#}"));
    assert_eq!(live[0]["tools"][0]["tool"], "Bash");
    assert_eq!(live[0]["tools"][0]["status"], "running");
    let queued = w.api.chat(&sid, &w.work(), "hello").await;
    assert_eq!(queued["status"], "queued");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        w.api.pending(&sid, None).await.len(),
        1,
        "the follow-up aborted the turn"
    );
    w.api.answer(&ask, "Allow once").await;
    let deadline = Instant::now() + Duration::from_secs(20);
    while finished_with(&w, mark, &sid).len() < 2 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(finished_with(&w, mark, &sid), ["Completed", "Completed"]);
    let prompts: Vec<Value> = starts(&w).iter().map(|s| s["prompt"].clone()).collect();
    assert_eq!(prompts, [json!("wait"), json!("hello")]);
}

fn steers(w: &World) -> Vec<Value> {
    std::fs::read_to_string(std::path::Path::new(&w.work()).join(".sdk-steers.log"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn steer_sends_a_queued_message_into_the_running_turn() {
    let (w, sid) = world().await;
    trust_folder(&w);
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "steer").await;
    let deadline = Instant::now() + Duration::from_secs(20);
    // A steer joins a turn the SDK has begun (before that, it waits as the
    // next turn): wait for the turn's first words.
    while !w.rows_opt(&sid).unwrap_or_default().iter().any(|r| r.said_by("claude") == Some("WORKING")) {
        assert!(Instant::now() < deadline, "the turn never began");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let queued = w.api.chat(&sid, &w.work(), "skip the tests").await;
    assert_eq!(queued["status"], "queued");
    let id = queued["queued_id"].as_str().expect("a queued send names its item");
    w.api.post("/api/queue/steer", json!({"id": id})).await;
    while finished_with(&w, mark, &sid).is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(finished_with(&w, mark, &sid), ["Completed"], "the steer started its own turn");
    let sent = steers(&w);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["text"], "skip the tests");
    assert_eq!(starts(&w).len(), 1);
    assert_eq!(
        w.api.post_status("/api/queue/steer", json!({"id": id})).await,
        404,
        "a steered message is no longer waiting"
    );
}

#[tokio::test]
async fn a_steer_the_turn_never_took_runs_once_as_the_next_turn() {
    let (w, sid) = world().await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "late").await;
    // The turn waits on folder trust: no runner reads the steer yet.
    let trust = asked(next(&w, &sid, mark).await);
    let queued = w.api.chat(&sid, &w.work(), "second").await;
    let id = queued["queued_id"].as_str().unwrap();
    w.api.post("/api/queue/steer", json!({"id": id})).await;
    tokio::time::sleep(Duration::from_secs(11)).await;
    w.api.answer(&trust, "Yes, trust this folder").await;
    let deadline = Instant::now() + Duration::from_secs(20);
    while finished_with(&w, mark, &sid).len() < 2 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(finished_with(&w, mark, &sid), ["Completed", "Completed"]);
    assert!(steers(&w).is_empty(), "the abandoned steer still reached the SDK");
    let prompts: Vec<Value> = starts(&w).iter().map(|s| s["prompt"].clone()).collect();
    assert_eq!(prompts, [json!("late"), json!("second")]);
}

#[tokio::test]
async fn a_picture_sent_as_bare_base64_reaches_the_sdk_as_a_data_url() {
    let (w, sid) = world().await;
    trust_folder(&w);
    let mark = w.mark();
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    w.api
        .post(
            "/api/chat",
            json!({"session_id": sid, "project_root": w.work(), "message": "hello", "images": [PNG]}),
        )
        .await;
    let deadline = Instant::now() + Duration::from_secs(20);
    while finished_with(&w, mark, &sid).is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(starts(&w)[0]["images"], json!([format!("data:image/png;base64,{PNG}")]));
}

#[tokio::test]
async fn a_background_task_is_listed_and_the_person_stops_it() {
    let (w, sid) = world().await;
    trust_folder(&w);
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "bg").await;
    assert_eq!(
        w.api.post_status("/api/tasks/stop", json!({"session_id": sid, "id": "nope"})).await,
        404
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    // Listed once the SDK reports it; stopping it reaches the SDK.
    while w.api.post_status("/api/tasks/stop", json!({"session_id": sid, "id": "t1"})).await != 200 {
        assert!(Instant::now() < deadline, "the task was never listed");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    while finished_with(&w, mark, &sid).is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(finished_with(&w, mark, &sid), ["Completed"], "stopping a task never stops the turn");
    let stops = std::fs::read_to_string(std::path::Path::new(&w.work()).join(".sdk-stops.log")).unwrap();
    assert!(stops.contains(r#""task_id":"t1""#), "{stops}");
}

async fn until_finished_runs(w: &World, mark: crate::support::engine::Mark, sid: &str, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while finished_with(w, mark, sid).len() < n {
        assert!(Instant::now() < deadline, "only {:?} finished", finished_with(w, mark, sid));
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn a_runner_outlives_its_turn_and_takes_the_next_message() {
    let (w, sid) = world().await;
    trust_folder(&w);
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "live").await;
    until_finished_runs(&w, mark, &sid, 1).await;
    w.api.chat(&sid, &w.work(), "again").await;
    until_finished_runs(&w, mark, &sid, 2).await;
    assert_eq!(finished_with(&w, mark, &sid), ["Completed", "Completed"]);
    assert_eq!(starts(&w).len(), 1, "the second message reached the same runner");
}

#[tokio::test]
async fn a_turn_the_sdk_starts_itself_gets_its_own_run() {
    let (w, sid) = world().await;
    trust_folder(&w);
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "live").await;
    until_finished_runs(&w, mark, &sid, 1).await;
    w.api.chat(&sid, &w.work(), "notify").await;
    until_finished_runs(&w, mark, &sid, 3).await;
    assert_eq!(finished_with(&w, mark, &sid), ["Completed", "Completed", "Completed"]);
    let rows = w.rows(&sid);
    assert!(rows.iter().any(|r| r.said_by("claude") == Some("TASK ENDED")));
    assert_eq!(starts(&w).len(), 1);
}

#[tokio::test]
async fn a_background_subagent_never_holds_the_next_message() {
    let (w, sid) = world().await;
    trust_folder(&w);
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "child").await;
    until_finished_runs(&w, mark, &sid, 1).await;
    // The child's first output comes once the turn has fully closed.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    // The child runs on after its turn's result; the person's next message
    // starts at once instead of waiting for it.
    w.api.chat(&sid, &w.work(), "again").await;
    until_finished_runs(&w, mark, &sid, 2).await;
    assert_eq!(finished_with(&w, mark, &sid), ["Completed", "Completed"]);
    let begun = w.engine.log_since(mark).lines().filter(|l| l.contains("run/begin") && l.contains(&sid)).count();
    assert_eq!(begun, 2, "the child's output began a run of its own");
    let rows = w.rows(&sid);
    assert!(rows.iter().any(|r| r.said_by("claude") == Some("SECOND")));
    // The child is kept once, on the turn that spawned it.
    let file = w.home.linggen_home().join("sessions").join(&sid).join("claude-subagents.json");
    let saved: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let runs = saved.as_object().unwrap();
    assert_eq!(runs.len(), 1, "{saved:#}");
    let (run, children) = runs.iter().next().unwrap();
    assert_eq!(children[0]["subagentId"], json!(format!("sdk-sub-{run}-c1")));
    assert_eq!(children[0]["resultText"], "1\n\n2");
    assert_eq!(children[0]["toolCount"], 1);
}

#[tokio::test]
async fn an_idle_runner_exits() {
    let setup = Setup { env: vec![("LINGGEN_CLAUDE_IDLE_EXIT_SECS", "1".into())], ..Setup::default() };
    let (w, sid) = world_with(setup).await;
    trust_folder(&w);
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "live").await;
    until_finished_runs(&w, mark, &sid, 1).await;
    let ends = std::path::Path::new(&w.work()).join(".sdk-ends.log");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ends.exists() {
        assert!(Instant::now() < deadline, "the idle runner was never ended");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // The next message starts a fresh runner on the saved conversation.
    w.api.chat(&sid, &w.work(), "hello").await;
    until_finished_runs(&w, mark, &sid, 2).await;
    assert_eq!(starts(&w).len(), 2);
}

#[tokio::test]
async fn a_claude_session_continues_after_a_restart_interrupted_it() {
    let (mut w, sid) = world().await;
    trust_folder(&w);
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "hello").await;
    until_finished_runs(&w, mark, &sid, 1).await;
    crate::restart::leave_plan(&w, json!([{"session_id": sid, "agent_id": "claude", "caller": false, "tasks": []}]));
    w.engine.restart(&w.home).await;
    let deadline = Instant::now() + Duration::from_secs(30);
    while starts(&w).len() < 2 {
        assert!(Instant::now() < deadline, "the Claude session never continued");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let prompt = starts(&w)[1]["prompt"].as_str().unwrap_or_default().to_string();
    assert!(prompt.contains("The engine restarted (from 1.10.0 to ling 1.10.1"), "{prompt}");
    assert!(prompt.contains("the restart interrupted it"), "{prompt}");
    assert_eq!(starts(&w)[1]["resume"], SDK_ID, "it continues the same SDK conversation");
}

#[tokio::test]
async fn messages_queued_in_a_claude_session_come_back_after_its_restart_turn() {
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    let (mut w, sid) = world().await;
    trust_folder(&w);
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "hello").await;
    until_finished_runs(&w, mark, &sid, 1).await;
    let queued = json!([
        {"text": "first queued", "images": [], "agent_id": "claude", "project_root": w.work()},
        {"text": "second queued", "images": [PNG], "agent_id": "claude", "project_root": w.work()},
    ]);
    crate::restart::leave_plan(&w, json!([{"session_id": sid, "agent_id": "claude", "caller": false, "tasks": [], "queued": queued}]));
    w.engine.restart(&w.home).await;
    let deadline = Instant::now() + Duration::from_secs(30);
    while starts(&w).len() < 4 {
        assert!(Instant::now() < deadline, "only {} turns ran", starts(&w).len());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let started = starts(&w);
    let prompt = |i: usize| started[i]["prompt"].as_str().unwrap_or_default().to_string();
    assert!(prompt(1).contains("The engine restarted"), "the continue turn runs first: {}", prompt(1));
    assert_eq!(prompt(2), "first queued");
    assert_eq!(prompt(3), "second queued");
    assert_eq!(started[3]["images"], json!([format!("data:image/png;base64,{PNG}")]));
}

#[tokio::test]
async fn a_removed_queued_message_never_runs() {
    let (w, sid) = world().await;
    trust_folder(&w);
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "wait").await;
    let ask = asked(next(&w, &sid, mark).await);
    let queued = w.api.chat(&sid, &w.work(), "hello").await;
    let id = queued["queued_id"].as_str().unwrap();
    let taken = w
        .api
        .post(
            "/api/queue/remove",
            json!({"project_root": w.work(), "session_id": sid, "agent_id": "claude", "id": id}),
        )
        .await;
    assert_eq!(taken["message"], "hello", "Edit gets the message back");
    w.api.answer(&ask, "Allow once").await;
    let deadline = Instant::now() + Duration::from_secs(20);
    while finished_with(&w, mark, &sid).is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(finished_with(&w, mark, &sid), ["Completed"]);
    let prompts: Vec<Value> = starts(&w).iter().map(|s| s["prompt"].clone()).collect();
    assert_eq!(prompts, [json!("wait")]);
}

/// The session's queue as the last queue event left it: each message's text.
fn queued_now(w: &World, sid: &str) -> Vec<String> {
    let events = w.events();
    let Some(last) = events.iter().rev().find(|e| e["kind"] == "queue" && e["session_id"] == sid) else {
        return Vec::new();
    };
    last["data"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["preview"].as_str().map(String::from))
        .collect()
}

/// Wait for the event log to show the session's queue as `want`.
async fn until_queued(w: &World, sid: &str, want: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while queued_now(w, sid) != want {
        assert!(Instant::now() < deadline, "queue is {:?}, not {want:?}", queued_now(w, sid));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A `cd` mid-turn moves the session's folder, never its queue: a remove
/// sent from the new folder takes only its message, the rest stay listed
/// and run in order with what is sent after.
#[tokio::test]
async fn a_queue_survives_a_cd_between_queueing_and_removing() {
    let (w, sid) = world().await;
    trust_folder(&w);
    std::fs::create_dir_all(std::path::Path::new(&w.work()).join("notes")).unwrap();
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "move").await;
    let first_ask = asked(next(&w, &sid, mark).await);
    let one = w.api.chat(&sid, &w.work(), "one").await;
    w.api.chat(&sid, &w.work(), "two").await;
    until_queued(&w, &sid, &["one", "two"]).await;

    let moved = w.mark();
    w.api.answer(&first_ask, "Allow once").await;
    let second_ask = asked(next(&w, &sid, moved).await);
    let notes = format!("{}/notes", w.work());
    assert_eq!(session_yaml(&w, &sid)["cwd"], json!(notes), "the session followed the cd");

    // The page sends the folder it shows now: the one the session moved to.
    let taken = w
        .api
        .post(
            "/api/queue/remove",
            json!({"project_root": notes, "session_id": sid, "agent_id": "claude", "id": one["queued_id"]}),
        )
        .await;
    assert_eq!(taken["message"], "one");
    until_queued(&w, &sid, &["two"]).await;
    w.api.chat(&sid, &notes, "three").await;
    until_queued(&w, &sid, &["two", "three"]).await;

    w.api.answer(&second_ask, "Allow once").await;
    until_finished_runs(&w, mark, &sid, 3).await;
    let prompts: Vec<Value> = starts(&w).iter().map(|s| s["prompt"].clone()).collect();
    assert_eq!(prompts, [json!("move"), json!("two"), json!("three")]);
}

#[tokio::test]
async fn stop_lands_while_tokens_stream() {
    let (w, sid) = world().await;
    trust_folder(&w);
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "stream").await;
    let deadline = Instant::now() + Duration::from_secs(20);
    let run_id = loop {
        let log = w.engine.log_since(mark);
        let begun = log
            .lines()
            .find(|l| l.contains("run/begin") && l.contains(&sid))
            .and_then(|l| l.split("run_id=").nth(1))
            .map(|rest| rest.split_whitespace().next().unwrap().to_string());
        if let (Some(id), false) = (begun, starts(&w).is_empty()) {
            break id;
        }
        assert!(Instant::now() < deadline, "the stream never started");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    let stopped = Instant::now();
    w.api
        .post("/api/agent-cancel", json!({"run_id":run_id}))
        .await;
    while finished_with(&w, mark, &sid).is_empty() {
        assert!(
            stopped.elapsed() < Duration::from_secs(10),
            "Stop did not land while tokens streamed"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(finished_with(&w, mark, &sid), ["Cancelled"]);
}

fn session_yaml(w: &World, sid: &str) -> Value {
    let path = w.home.linggen_home().join("sessions").join(sid).join("session.yaml");
    serde_norway::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[tokio::test]
async fn a_claude_session_follows_the_sdks_cd_and_keeps_its_last_repo() {
    let (w, sid) = world().await;
    trust_folder(&w);
    let work = std::path::PathBuf::from(w.work());
    std::fs::create_dir_all(work.join("repo/.git")).unwrap();
    std::fs::create_dir_all(work.join("repo/src")).unwrap();
    std::fs::create_dir_all(work.join("notes")).unwrap();
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "cd").await;
    until_finished_runs(&w, mark, &sid, 1).await;
    // The folder follows each move; the project stays the last repo.
    let meta = session_yaml(&w, &sid);
    assert_eq!(meta["cwd"], json!(format!("{}/notes", w.work())));
    assert_eq!(meta["project_name"], "repo");
    // The next runner resumes the conversation in the folder it started in.
    w.api.chat(&sid, &w.work(), "hello").await;
    until_finished_runs(&w, mark, &sid, 2).await;
    assert_eq!(starts(&w)[1]["resume"], SDK_ID);
    assert_eq!(starts(&w)[1]["cwd"], starts(&w)[0]["cwd"]);
}

/// The SDK's own copy of the conversation, as it keeps it.
fn sdk_transcript(w: &World) -> (std::path::PathBuf, std::path::PathBuf) {
    let encoded: String = w.work().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let project = w.home.linggen_home().join("runtimes/claude-agent-sdk/state/projects").join(encoded);
    std::fs::create_dir_all(project.join(SDK_ID).join("subagents")).unwrap();
    std::fs::write(project.join(format!("{SDK_ID}.jsonl")), "{}").unwrap();
    std::fs::write(project.join("other.jsonl"), "{}").unwrap();
    (project.join(format!("{SDK_ID}.jsonl")), project)
}

#[tokio::test]
async fn clear_and_delete_remove_the_sdks_transcript_of_the_session_only() {
    let (w, sid) = world().await;
    trust_folder(&w);
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "hello").await;
    until_finished_runs(&w, mark, &sid, 1).await;
    let (transcript, project) = sdk_transcript(&w);
    w.api.post("/api/chat/clear", json!({"session_id": sid})).await;
    assert!(!transcript.exists(), "Clear starts a new conversation; the old copy goes");
    assert!(!project.join(SDK_ID).exists());
    assert!(project.join("other.jsonl").exists());

    w.api.chat(&sid, &w.work(), "hello").await;
    until_finished_runs(&w, mark, &sid, 2).await;
    let (transcript, project) = sdk_transcript(&w);
    w.api.delete("/api/sessions/all", json!({"session_id": sid})).await;
    assert!(!transcript.exists(), "the deleted session's transcript stayed");
    assert!(!project.join(SDK_ID).exists());
    assert!(project.join("other.jsonl").exists(), "another conversation was removed");
}
