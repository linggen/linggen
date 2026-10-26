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
 emit "{\"type\":\"result\",\"success\":true,\"text\":\"$1\",\"session_id\":\"sdk-host\",\"usage\":{}}"
 emit '{"type":"done"}'
}
RULE='{"type":"addRules","behavior":"allow","destination":"session","rules":[{"toolName":"Bash","ruleContent":"env"}]}'
emit '{"type":"init","session_id":"sdk-host","permissionMode":"default","commands":[]}'
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
*'"prompt":"stream"'*)
 while :; do emit '{"type":"token","text":"x"}'; sleep 0.02; done;;
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
