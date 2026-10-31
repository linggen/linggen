//! Prewritten sessions (tests/fixtures/home/sessions): a shared table, a
//! compacted chat, a run cut off before its reply — read back the way the
//! engine and the chat page read them.

use crate::support::engine::RunEnd;
use crate::support::model::Agent;
use crate::support::{snap, snap_text, text, tool, Setup, World};
use serde_json::json;

const SHARED: &str = "sess-1790000001-5a1ed001";
const COMPACTED: &str = "sess-1790000002-c0ac7ed2";
const INTERRUPTED: &str = "sess-1790000003-1e7e5ed3";

/// Claude sessions have one execution owner and persist SDK settings without
/// creating a native permission file or invoking the native model.
#[tokio::test]
async fn claude_session_creation_and_permissions_are_independent() {
    let w = World::start(Setup::default()).await;
    let native = w.api.create_session(json!({"title":"Native"})).await;
    let sid = w
        .api
        .create_session(
            json!({"title":"Claude", "runtime":"claude_agent_sdk", "project_root":w.work()}),
        )
        .await;
    w.api
        .patch(
            "/api/sessions",
            json!({"session_id":sid, "claude_permission_mode":"acceptEdits"}),
        )
        .await;
    let dir = w.home.linggen_home().join("sessions").join(&sid);
    let meta: serde_json::Value =
        serde_norway::from_str(&std::fs::read_to_string(dir.join("session.yaml")).unwrap())
            .unwrap();
    assert_eq!(meta["runtime"], "claude_agent_sdk");
    assert_eq!(meta["agents"], json!([{"id":"claude"}]));
    assert_eq!(meta["claude_permission_mode"], "acceptEdits");
    assert!(!dir.join("permission.json").exists());
    let native_meta = std::fs::read_to_string(
        w.home
            .linggen_home()
            .join("sessions")
            .join(&native)
            .join("session.yaml"),
    )
    .unwrap();
    assert!(!native_meta.contains("claude"));
    let client = reqwest::Client::new();
    for (path, payload) in [
        (
            "/api/sessions/permission",
            json!({"session_id":sid, "path":w.work(), "mode":"admin"}),
        ),
        (
            "/api/sessions",
            json!({"session_id":sid, "agent_id":"ling", "model_id":"test"}),
        ),
        (
            "/api/sessions",
            json!({"session_id":sid, "claude_permission_mode":"admin"}),
        ),
    ] {
        let response = client
            .patch(format!("{}{path}", w.engine.base_url()))
            .json(&payload)
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_client_error(),
            "{path} accepted native policy"
        );
    }
    // Selecting a native member cannot turn a Claude session into a native run.
    let response = client
        .post(format!("{}/api/chat", w.engine.base_url()))
        .json(&json!({
            "session_id":sid, "project_root":w.work(), "agent_id":"ling", "message":"Hello"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    // Apps may only create, bind and run native Linggen sessions.
    for (path, payload) in [
        (
            "/api/sessions",
            json!({"title":"App", "runtime":"claude_agent_sdk", "skill":"dice"}),
        ),
        (
            "/api/chat",
            json!({"session_id":sid, "project_root":w.work(), "agent_id":"claude", "skill_name":"dice", "message":"Hello"}),
        ),
    ] {
        let response = client
            .post(format!("{}{path}", w.engine.base_url()))
            .json(&payload)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            400,
            "{path} accepted a Claude app session"
        );
    }
    let response = client
        .get(format!("{}/api/skill-sessions/state", w.engine.base_url()))
        .query(&[("skill", "dice"), ("session_id", sid.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    w.assert_all_scripted();
}

#[tokio::test]
async fn claude_history_keeps_the_answer_and_restores_tools_without_observation_bubbles() {
    let w = World::start(Setup::default()).await;
    let sid = w
        .api
        .create_session(json!({"title":"Claude", "runtime":"claude_agent_sdk"}))
        .await;
    let rows = [
        json!({"from_id":"user", "to_id":"claude", "content":"Show the image", "is_observation":false}),
        json!({"from_id":"claude", "to_id":"user", "content":json!({"type":"tool", "tool":"mcp__linggen-display__DisplayImage", "args":{"source":"image.png"}}).to_string(), "is_observation":true}),
        json!({"from_id":"system", "to_id":"claude", "content":"Tool mcp__linggen-display__DisplayImage: image kept", "is_observation":true}),
        json!({"from_id":"claude", "to_id":"user", "content":"[turn1image1]", "is_observation":false}),
    ];
    let text = rows
        .into_iter()
        .enumerate()
        .map(|(i, mut row)| {
            row["agent_id"] = json!("claude");
            row["timestamp"] = json!(1790000000 + i);
            row["run_id"] = json!("sdk-image");
            format!("{row}\n")
        })
        .collect::<String>();
    std::fs::write(
        w.home
            .linggen_home()
            .join("sessions")
            .join(&sid)
            .join("messages.jsonl"),
        text,
    )
    .unwrap();
    let state = w.api.history(&sid, &w.work()).await;
    let messages = state["messages"].as_array().unwrap();
    assert_eq!(
        messages.len(),
        2,
        "tool observations became chat bubbles: {state}"
    );
    assert_eq!(messages[1][1], "[turn1image1]");
    assert_eq!(messages[1][0]["tools"][0]["status"], "done");
    w.assert_all_scripted();
}

/// Each member's thread, built from the one session thread: his own Look as
/// native call/result pairs, hers as labelled dialogue — and for her, his
/// Look as labelled text carrying only what it declares for others.
#[tokio::test]
async fn shared_session_builds_each_members_thread() {
    let w = World::start(
        Setup::default()
            .script(Agent::Ling, vec![text("Your roll.")])
            .script(Agent::Yinyue, vec![text("Good luck.")]),
    )
    .await;
    w.turn(SHARED, "", "Roll again?", Agent::Ling).await;
    w.turn(SHARED, "", "@银月 wish me luck", Agent::Yinyue)
        .await;

    let his = w.model.calls_of(Agent::Ling).pop().expect("his call");
    let hers = w.model.calls_of(Agent::Yinyue).pop().expect("her call");
    let thread = his.thread();
    let native_call = thread
        .iter()
        .any(|(r, t)| r == "assistant" && t.contains("[call Look"));
    let native_result = thread.iter().any(|(r, _)| r == "tool");
    assert!(
        native_call && native_result,
        "his Look is not a native call/result pair"
    );
    assert!(hers.text().contains("Ling used Look"));
    assert!(
        !hers.text().contains("Never mention this guide"),
        "she read his guide"
    );
    snap("shared_ling_thread", &w, &his);
    snap("shared_yinyue_thread", &w, &hers);
    w.assert_all_scripted();
}

/// A compacted session: the next thread starts at the summary row, and no
/// row before it reaches the model.
#[tokio::test]
async fn compacted_session_thread_starts_at_the_summary() {
    let w =
        World::start(Setup::default().script(Agent::Ling, vec![text("Yes, every thirty.")])).await;
    w.turn(
        COMPACTED,
        &w.work(),
        "Still every thirty on Sundays?",
        Agent::Ling,
    )
    .await;
    let call = w.model.calls_of(Agent::Ling).pop().expect("his call");
    let opening = call.thread().first().cloned().unwrap_or_default();
    assert_eq!(opening.0, "system", "the summary is not a system note");
    assert!(opening
        .1
        .contains("SUMMARY: Alex asked about the Cacilhas ferry"));
    assert!(
        !call.text().contains("(before the summary)"),
        "a row before the summary reached the model"
    );
    snap("compacted_ling_thread", &w, &call);
    w.assert_all_scripted();
}

/// A run cut off before its reply shows, in the history view, one display
/// row after its last row: `run: "interrupted"` with its calls.
#[tokio::test]
async fn interrupted_run_has_a_display_row() {
    let w = World::start(Setup::default()).await;
    let history = w.api.history(INTERRUPTED, &w.work()).await;
    let rows = history["messages"].as_array().cloned().unwrap_or_default();
    let cut = rows.iter().find(|r| r[0]["run"] == "interrupted");
    let cut = cut.unwrap_or_else(|| panic!("no interrupted row in {history:#}"));
    assert!(
        cut.to_string().contains("Glob"),
        "the row does not carry its call: {cut}"
    );
    let pretty = serde_json::to_string_pretty(&history["messages"]).expect("json");
    snap_text("interrupted_history", &w, &pretty);
    w.assert_all_scripted();
}

/// A message sent with no folder (a page still connecting has no session
/// list to read it from) runs in the session's own folder, not in `~`: the
/// Write lands in the session's cwd.
#[tokio::test]
async fn a_message_without_a_folder_runs_in_the_sessions_own() {
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![
            tool("Write", json!({"path": "probe.txt", "content": "hello"})),
            text("Wrote it."),
        ],
    ))
    .await;
    let mark = w.mark();
    w.api
        .chat(COMPACTED, "", "Write probe.txt saying hello.")
        .await;
    let RunEnd::Asked(ask) = w.wait_run(COMPACTED, Agent::Ling, mark).await else {
        panic!("the Write ran without a permission prompt");
    };
    w.api.answer(&ask, "grant:all").await;
    w.finish_run(COMPACTED, Agent::Ling, mark).await;

    let written = std::fs::read_to_string(w.home.work().join("probe.txt"));
    assert_eq!(
        written.ok().as_deref(),
        Some("hello"),
        "not written in the session's folder"
    );
    assert!(!w.home.home().join("probe.txt").exists(), "written in ~");
    w.assert_all_scripted();
}
