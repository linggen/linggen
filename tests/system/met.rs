//! An agent that appears only once met (`engine::agent::met`): Yinyue is
//! nowhere on a fresh install, found for good by the game that finds her, and
//! kept by anyone who already had her.

use crate::support::engine::RunEnd;
use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use serde_json::{json, Value};
use std::time::Duration;

fn fresh() -> Setup {
    Setup {
        fixture: Some("fresh"),
        ..Setup::default()
    }
}

/// The agent ids `GET /api/agent-files` lists — the agent list and the `@`
/// picker read the same funnel.
async fn listed(w: &World) -> Vec<String> {
    let q = format!(
        "/api/agent-files?project_root={}",
        urlencoding::encode(&w.work())
    );
    let all = w.api.get(&q).await;
    all.as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a["agent_id"].as_str().map(str::to_string))
        .collect()
}

/// The retained `mac/agents` topic, once it has been published.
async fn topic(w: &World) -> Value {
    for _ in 0..50 {
        let resp = reqwest::get(format!(
            "{}/api/topic/latest?topic=mac&op=agents",
            w.engine.base_url()
        ))
        .await
        .expect("topic");
        if resp.status().is_success() {
            let body: Value = resp.json().await.expect("topic json");
            return body["payload"].clone();
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("mac/agents was never published");
}

/// The topic's entry for her.
async fn her(w: &World) -> Value {
    let t = topic(w).await;
    t["agents"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["id"] == "yinyue")
        .cloned()
        .unwrap_or_else(|| panic!("she is not declared in {t}"))
}

/// Wait until her entry on the topic says `present`.
async fn wait_present(w: &World, present: bool) -> Value {
    for _ in 0..60 {
        let entry = her(w).await;
        if entry["present"] == present {
            return entry;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("she never read present={present}");
}

fn latch(w: &World) -> Option<Value> {
    let path = w.home.linggen_home().join("met/yinyue.json");
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn set_joined(w: &World, state: &str) {
    let dir = w.home.linggen_home().join("skills/lingjing/data");
    std::fs::create_dir_all(&dir).expect("data dir");
    std::fs::write(dir.join("state.json"), state).expect("state.json");
}

/// A fresh install: she is on no list, takes no message, speaks no line — and
/// Ling asks for the permission and gets the answer, as she always did.
#[tokio::test]
async fn before_she_is_met_she_is_nowhere_and_ling_covers() {
    let w = World::start(fresh().script(
        Agent::Ling,
        vec![
            text("Hello."),
            tool("Write", json!({"path": "probe.txt", "content": "hello"})),
            text("Wrote it."),
        ],
    ))
    .await;
    assert_eq!(listed(&w).await, ["ling", "memory"].map(String::from));
    let entry = her(&w).await;
    assert_eq!(entry["present"], false);
    assert!(entry["met"].is_null());
    assert!(latch(&w).is_none(), "nothing is written before she is met");

    // Not addressable: `@银月` is plain words to Ling; a message sent to her
    // by id is refused; her own endpoints answer absent.
    let root = w.work();
    let sid = w
        .api
        .create_session(json!({"title": "chat", "project_root": root}))
        .await;
    let mark = w.mark();
    let resp = w.api.chat(&sid, &root, "@银月 hi").await;
    assert_eq!(resp["agent_id"], "ling", "{resp}");
    w.finish_run(&sid, Agent::Ling, mark).await;

    let refused = w
        .api
        .post(
            "/api/chat",
            json!({"session_id": sid, "project_root": root, "message": "hi", "agent_id": "yinyue"}),
        )
        .await;
    assert_eq!(refused["status"], "absent", "{refused}");
    let url = format!("{}/api/yinyue/chat", w.engine.base_url());
    let talk = reqwest::Client::new()
        .post(url)
        .json(&json!({"text": "hello?"}))
        .send()
        .await
        .expect("yinyue chat");
    assert_eq!(talk.status(), 409);

    // Ling's permission prompt reaches the plain UI path and is answerable.
    let mark = w.mark();
    w.api
        .chat(&sid, &root, "Write probe.txt saying hello.")
        .await;
    let RunEnd::Asked(ask) = w.wait_run(&sid, Agent::Ling, mark).await else {
        panic!("the Write ran without a permission prompt");
    };
    w.api.answer(&ask, "allow once").await;
    w.finish_run(&sid, Agent::Ling, mark).await;
    assert!(w.home.work().join("probe.txt").exists());
    assert!(latch(&w).is_none());
    w.assert_all_scripted();
}

/// The game sets the flag: she appears, the latch says why — and nothing the
/// save does afterwards takes her away.
#[tokio::test]
async fn she_is_met_when_the_game_finds_her_and_stays() {
    let w = World::start(fresh().script(Agent::Yinyue, vec![text("Here.")])).await;
    assert!(!listed(&w).await.contains(&"yinyue".to_string()));

    set_joined(&w, r#"{"companion":{"joined":"2026-10-09"}}"#);
    let entry = wait_present(&w, true).await;
    assert_eq!(entry["met"]["reason"], "trigger");
    assert!(listed(&w).await.contains(&"yinyue".to_string()));
    assert_eq!(latch(&w).expect("the latch")["reason"], "trigger");

    // The save is forgotten, then gone: still met.
    set_joined(&w, "{}");
    tokio::time::sleep(Duration::from_secs(7)).await;
    assert!(listed(&w).await.contains(&"yinyue".to_string()));
    assert_eq!(her(&w).await["present"], true);
    std::fs::remove_file(
        w.home
            .linggen_home()
            .join("skills/lingjing/data/state.json"),
    )
    .expect("remove the save");
    assert_eq!(her(&w).await["present"], true);

    // And she answers.
    let mark = w.mark();
    w.api
        .post("/api/yinyue/chat", json!({"text": "hello?"}))
        .await;
    w.wait_agent(Agent::Yinyue, mark).await;
    w.assert_all_scripted();
}

/// Someone who already had her keeps her: the first start latches "history".
#[tokio::test]
async fn an_existing_install_keeps_her() {
    let w = World::start(Setup::default()).await;
    let entry = her(&w).await;
    assert_eq!(entry["present"], true);
    assert_eq!(entry["met"]["reason"], "history");
    assert_eq!(latch(&w).expect("the latch")["reason"], "history");
    assert!(listed(&w).await.contains(&"yinyue".to_string()));
}
