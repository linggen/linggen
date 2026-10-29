//! After the engine restarts itself (doc/background-tasks-spec.md § Engine
//! restart), every session in its plan continues with a turn saying what
//! happened — a native one and a Claude one alike — and the messages the
//! person had queued there come back behind that turn, in their order.

use crate::support::engine::RunEnd;
use crate::support::model::Agent;
use crate::support::{text, Setup, World};
use serde_json::json;
use std::time::{Duration, Instant};

/// What the restarting engine leaves for the next one, and how the helper said it went.
pub fn leave_plan(w: &World, sessions: serde_json::Value) {
    let dir = w.home.linggen_home().join("restart");
    std::fs::create_dir_all(&dir).unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let plan = json!({"requested_at": now, "from_version": "1.10.0", "to_version": "ling 1.10.1", "sessions": sessions});
    std::fs::write(dir.join("resume.json"), plan.to_string()).unwrap();
    std::fs::write(dir.join("status.json"), json!({"state": "ok", "at": now}).to_string()).unwrap();
}

#[tokio::test]
async fn a_native_session_continues_after_the_restart_it_asked_for() {
    let mut w = World::start(Setup::default().script(
        Agent::Ling,
        vec![text("Building it now."), text("Back on the new build; checking where I was.")],
    ))
    .await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "Rebuild and restart the engine").await;
    assert!(matches!(w.wait_run(&sid, Agent::Ling, mark).await, RunEnd::Finished));
    leave_plan(&w, json!([{"session_id": sid, "agent_id": "ling", "caller": true, "tasks": ["npm run dev (sh-1)"]}]));
    w.engine.restart(&w.home).await;
    let deadline = Instant::now() + Duration::from_secs(30);
    while w.model.calls_of(Agent::Ling).len() < 2 {
        assert!(Instant::now() < deadline, "the session never continued");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let calls = w.model.calls_of(Agent::Ling);
    let told = calls[1].thread().last().map(|(_, t)| t.clone()).unwrap_or_default();
    assert!(told.contains("The engine restarted (from 1.10.0 to ling 1.10.1; the new build is up and healthy)"), "{told}");
    assert!(told.contains("You asked for this restart"), "{told}");
    assert!(told.contains("npm run dev (sh-1)"), "{told}");
    assert!(!w.home.linggen_home().join("restart/resume.json").exists(), "a plan runs once");
}

#[tokio::test]
async fn messages_queued_behind_the_interrupted_turn_come_back_after_it_in_order() {
    let mut w = World::start(Setup::default().script(
        Agent::Ling,
        vec![text("Building it now."), text("Back; checking."), text("First answered."), text("Second answered.")],
    ))
    .await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "Rebuild and restart the engine").await;
    assert!(matches!(w.wait_run(&sid, Agent::Ling, mark).await, RunEnd::Finished));
    let queued = |text: &str| json!({"text": text, "images": [], "agent_id": "ling", "project_root": w.work()});
    leave_plan(&w, json!([{
        "session_id": sid, "agent_id": "ling", "caller": true, "tasks": [],
        "queued": [queued("first queued message"), queued("second queued message")],
    }]));
    w.engine.restart(&w.home).await;
    let deadline = Instant::now() + Duration::from_secs(30);
    while w.model.calls_of(Agent::Ling).len() < 4 {
        assert!(Instant::now() < deadline, "only {} turns ran", w.model.calls_of(Agent::Ling).len());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let last = |i: usize| w.model.calls_of(Agent::Ling)[i].thread().last().map(|(_, t)| t.clone()).unwrap_or_default();
    assert!(last(1).contains("The engine restarted"), "the continue turn runs first: {}", last(1));
    assert!(last(2).contains("first queued message"), "{}", last(2));
    assert!(last(3).contains("second queued message"), "{}", last(3));
}

#[tokio::test]
async fn a_restart_on_a_build_that_does_not_run_is_refused_and_plans_nothing() {
    let w = World::start(Setup::default()).await;
    let bogus = w.home.root.join("not-a-build");
    std::fs::write(&bogus, "#!/bin/sh\nexit 3\n").unwrap();
    std::fs::set_permissions(&bogus, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let status = w
        .api
        .post_status("/api/engine/restart", json!({"binary": bogus, "wait_secs": 0}))
        .await;
    assert_eq!(status, 400);
    assert!(!w.home.linggen_home().join("restart/resume.json").exists());
}
