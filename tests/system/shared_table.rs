//! Shared sessions (doc/shared-session-spec.md), ported from
//! scripts/live-check.sh's model cases (a) (b) (d) (e) (f) — on the fake
//! model, so every reply is the one the test chose.

use crate::support::engine::RunEnd;
use crate::support::model::{Agent, Call};
use crate::support::rows::{calls_by, last_said};
use crate::support::{snap, text, tool, Setup, World};
use serde_json::json;

/// Session meta's member ids.
fn members(w: &World, sid: &str) -> Vec<String> {
    let path = w
        .home
        .linggen_home()
        .join("sessions")
        .join(sid)
        .join("session.yaml");
    let meta: serde_json::Value =
        serde_norway::from_str(&std::fs::read_to_string(path).expect("session.yaml"))
            .expect("yaml");
    let agents = meta["agents"].as_array().cloned().unwrap_or_default();
    agents
        .iter()
        .filter_map(|a| a["id"].as_str().map(str::to_string))
        .collect()
}

fn last_call(w: &World, agent: Agent) -> Call {
    w.model
        .calls_of(agent)
        .pop()
        .unwrap_or_else(|| panic!("no call by {agent:?}"))
}

/// (a) Ling's tool call at a skill table; @银月 answers from its labelled
/// result with no tools and no question; the next plain message goes back
/// to Ling.
#[tokio::test]
async fn a_shared_table_she_answers_from_his_look() {
    let w = World::start(
        Setup::default()
            .script(
                Agent::Ling,
                vec![tool("Look", json!({})), text("Harbor Table."), text("Mm.")],
            )
            .script(Agent::Yinyue, vec![text("Alex.")]),
    )
    .await;
    let sid = w
        .api
        .create_session(json!({"title": "table", "skill": "dice"}))
        .await;
    assert_eq!(
        members(&w, &sid),
        ["ling", "yinyue"],
        "the skill seats both"
    );

    w.turn(&sid, "", "Look at the table.", Agent::Ling).await;
    assert_eq!(calls_by(&w.rows(&sid), "ling"), ["Look"]);

    let before = w.rows(&sid).len();
    w.turn(&sid, "", "@银月 what is the player called?", Agent::Yinyue)
        .await;
    let hers = &w.rows(&sid)[before..];
    assert!(
        calls_by(hers, "yinyue").is_empty(),
        "she made tool calls: {hers:?}"
    );
    assert_eq!(last_said(hers, "yinyue"), Some("Alex."));

    let read = last_call(&w, Agent::Yinyue);
    let seen = read.text();
    assert!(
        seen.contains("Ling used Look"),
        "his call is not labelled text for her"
    );
    assert!(
        seen.contains("Harbor Table") && seen.contains("Alex"),
        "the declared facts are missing"
    );
    assert!(
        !seen.contains("Never mention this guide"),
        "she read Ling's guide (others_read ignored)"
    );
    assert!(
        !read.tool_names().contains(&"AskUser".to_string()),
        "she is offered AskUser"
    );
    assert!(
        !read.tool_names().contains(&"Look".to_string()),
        "she is offered his Look"
    );
    snap("a_yinyue_thread", &w, &read);

    w.turn(&sid, "", "Thanks.", Agent::Ling).await;
    snap("a_ling_thread_after_her", &w, &last_call(&w, Agent::Ling));
    w.assert_all_scripted();
}

/// (b) @银月 joins an ordinary chat; her Write asks permission; Deny writes
/// nothing; she stays a member.
#[tokio::test]
async fn b_her_write_asks_and_deny_writes_nothing() {
    let probe = "lc-probe.txt";
    let w = World::start(Setup::default().script(
        Agent::Yinyue,
        vec![
            tool("Write", json!({"path": probe, "content": "hello"})),
            text("Left it."),
        ],
    ))
    .await;
    let folder = w.home.work().join("chat");
    std::fs::create_dir_all(&folder).expect("chat folder");
    let root = folder.display().to_string();
    let sid = w
        .api
        .create_session(json!({"title": "chat", "project_root": root}))
        .await;
    assert_eq!(members(&w, &sid), ["ling"]);

    let mark = w.mark();
    let resp = w
        .api
        .chat(&sid, &root, "@银月 write lc-probe.txt saying hello.")
        .await;
    assert_eq!(resp["agent_id"], "yinyue");
    let RunEnd::Asked(ask) = w.wait_run(&sid, Agent::Yinyue, mark).await else {
        panic!("her Write ran without a permission prompt");
    };
    assert!(
        ask.to_string().contains("Write"),
        "the prompt is not about her Write: {ask}"
    );
    w.api.answer(&ask, "deny").await;
    w.finish_run(&sid, Agent::Yinyue, mark).await;

    assert!(!folder.join(probe).exists(), "Deny still wrote the file");
    // The denied call is saved like any other: its row and the refusal.
    let rows = w.rows(&sid);
    assert!(
        calls_by(&rows, "yinyue").contains(&"Write".to_string()),
        "the denied Write left no call row: {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|r| r.is_observation && r.content.starts_with("Tool Write: ")),
        "the denial left no result row: {rows:?}"
    );
    assert_eq!(
        members(&w, &sid),
        ["ling", "yinyue"],
        "she is not kept as a member"
    );
    w.assert_all_scripted();
}

/// A message long enough that a compaction has something to summarize: the
/// verbatim tail keeps 15% of the smallest window (32k tokens here).
fn filler() -> String {
    (0..900)
        .map(|i| format!("filler-{i:05} the quick brown fox jumps over the lazy dog."))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The first message of a member's built thread.
fn opening(call: &Call) -> String {
    call.thread()
        .first()
        .map(|(_, t)| t.clone())
        .unwrap_or_default()
}

/// (d) A forced compaction of a shared session writes one summary row, and
/// both members' next threads start at it.
#[tokio::test]
async fn d_forced_shared_compaction_one_row_both_threads_start_at_it() {
    let w = World::start(
        Setup::default()
            .script(Agent::Ling, vec![text("OK."), text("Fine."), text("Here.")])
            .script(Agent::Yinyue, vec![text("Hi."), text("Here too.")])
            .extra(
                "context compaction assistant",
                "SUMMARY: Alex sent filler; Ling said OK.",
            ),
    )
    .await;
    let root = w.work();
    let sid = w
        .api
        .create_session(json!({"title": "long", "project_root": root}))
        .await;
    w.turn(&sid, &root, "@银月 hello", Agent::Yinyue).await;
    w.turn(
        &sid,
        &root,
        &format!("Ignore this filler.\n{}", filler()),
        Agent::Ling,
    )
    .await;
    w.turn(&sid, &root, "Short one.", Agent::Ling).await;

    let resp = w
        .api
        .post(
            "/api/chat/compact",
            json!({"session_id": sid, "project_root": root}),
        )
        .await;
    assert_eq!(resp["compacted"], true, "not compacted: {resp}");
    let rows = w.rows(&sid);
    let summaries: Vec<usize> = (0..rows.len())
        .filter(|&i| rows[i].from_id == "compaction")
        .collect();
    assert_eq!(summaries.len(), 1, "want exactly one compaction row");

    w.turn(&sid, &root, "Still there?", Agent::Ling).await;
    w.turn(&sid, &root, "@银月 you too?", Agent::Yinyue).await;
    for agent in [Agent::Ling, Agent::Yinyue] {
        let call = w.model.calls_of(agent).pop().expect("a call");
        assert!(
            opening(&call).contains("SUMMARY: Alex sent filler"),
            "{agent:?}'s thread does not start at the summary"
        );
        assert!(
            !call.text().contains("filler-00001"),
            "{agent:?}'s thread reaches past the summary"
        );
    }
    snap(
        "d_ling_thread_after_compaction",
        &w,
        &w.model.calls_of(Agent::Ling).pop().unwrap(),
    );
    w.assert_all_scripted();
}

/// (e) Two members on two models — Ling on Chat Completions, Yinyue on the
/// Responses API — and no provider error for either run.
#[tokio::test]
async fn e_two_members_on_two_models() {
    let w = World::start(
        Setup::default()
            .script(Agent::Ling, vec![text("Here.")])
            .script(Agent::Yinyue, vec![text("Hi."), text("Here too.")]),
    )
    .await;
    let root = w.work();
    let sid = w
        .api
        .create_session(json!({"title": "two", "project_root": root}))
        .await;
    w.turn(&sid, &root, "@银月 hello", Agent::Yinyue).await;
    w.api
        .patch(
            "/api/sessions",
            json!({"session_id": sid, "agent_id": "yinyue", "model_id": "fake-sol"}),
        )
        .await;

    let mark = w.mark();
    w.turn(&sid, &root, "Are you there?", Agent::Ling).await;
    w.turn(&sid, &root, "@银月 and you?", Agent::Yinyue).await;

    let ling = w.model.calls_of(Agent::Ling).pop().expect("his call");
    let hers = w.model.calls_of(Agent::Yinyue).pop().expect("her call");
    assert_eq!(
        (ling.path.as_str(), ling.model()),
        ("/v1/chat/completions", "fake-ling-1")
    );
    assert_eq!(
        (hers.path.as_str(), hers.model()),
        ("/v1/responses", "fake-sol-1")
    );
    let bad = provider_warnings(&w.engine.log_since(mark));
    assert!(
        bad.is_empty(),
        "provider warnings during the two runs:\n{}",
        bad.join("\n")
    );
    let yaml = std::fs::read_to_string(
        w.home
            .linggen_home()
            .join("sessions")
            .join(&sid)
            .join("session.yaml"),
    )
    .expect("session.yaml");
    assert!(
        yaml.contains("fake-sol"),
        "her model is not kept in session.yaml:\n{yaml}"
    );
    w.assert_all_scripted();
}

/// WARN/ERROR lines from a provider, or naming one of the runs begun in `log`.
fn provider_warnings(log: &str) -> Vec<String> {
    let runs: Vec<String> = log
        .lines()
        .filter(|l| l.contains("run/begin"))
        .filter_map(|l| {
            l.split("run_id=")
                .nth(1)?
                .split_whitespace()
                .next()
                .map(str::to_string)
        })
        .collect();
    log.lines()
        .filter(|l| l.contains(" WARN ") || l.contains(" ERROR "))
        .filter(|l| l.contains("src/provider/") || runs.iter().any(|r| l.contains(r.as_str())))
        .map(str::to_string)
        .collect()
}

/// (f) An app moment naming a table lands as her rows in that session — and
/// only there; her hidden kickoff is hers alone.
#[tokio::test]
async fn f_app_moment_lands_in_the_named_session() {
    let w =
        World::start(Setup::default().script(Agent::Yinyue, vec![text("Clouds over the harbor.")]))
            .await;
    let other = w
        .api
        .create_session(json!({"title": "other table", "skill": "dice"}))
        .await;
    let sid = w
        .api
        .create_session(json!({"title": "table", "skill": "dice"}))
        .await;

    let mark = w.mark();
    let resp = w
        .api
        .post("/api/yinyue/event", json!({"app": "dice", "text": "The player looks at the sky.", "asked": true, "session": sid}))
        .await;
    assert_eq!(resp, json!("ok"));
    w.finish_run(&sid, Agent::Yinyue, mark).await;

    let rows = w.rows(&sid);
    assert_eq!(last_said(&rows, "yinyue"), Some("Clouds over the harbor."));
    let hidden: Vec<_> = rows
        .iter()
        .filter(|r| r.content.contains("[HIDDEN]"))
        .collect();
    assert!(!hidden.is_empty(), "no hidden kickoff row");
    assert!(
        hidden.iter().all(|r| r.agent_id == "yinyue"),
        "a hidden kickoff held by another member: {hidden:?}"
    );
    assert!(
        w.rows(&other).is_empty(),
        "the moment leaked into another table"
    );
    w.assert_all_scripted();
}
