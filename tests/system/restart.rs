//! After the engine restarts itself (doc/background-tasks-spec.md § Engine
//! restart), every session in its plan continues with a turn saying what
//! happened, and queued messages are provided in order for the agent to
//! assess rather than blindly replay.

use crate::support::engine::RunEnd;
use crate::support::model::Agent;
use crate::support::{text, Setup, World};
use serde_json::json;
use std::time::{Duration, Instant};

const PICTURE: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

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
async fn legacy_queued_messages_are_preserved_without_unverified_replay() {
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
    while w.model.calls_of(Agent::Ling).len() < 2 {
        assert!(Instant::now() < deadline, "the session never continued");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let calls = w.model.calls_of(Agent::Ling);
    let told = calls[1].thread().last().map(|(_, t)| t.clone()).unwrap_or_default();
    assert!(told.contains("first queued message"), "{told}");
    assert!(told.contains("second queued message"), "{told}");
    assert!(told.find("first queued message") < told.find("second queued message"), "{told}");
    assert!(told.contains("already processed is unknown"), "{told}");
    assert!(told.contains("Check the conversation and current state"), "{told}");
    assert!(told.contains("do not replay them blindly"), "{told}");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(w.model.calls_of(Agent::Ling).len(), 2, "queued messages must not run as separate turns");
    assert!(!w.home.linggen_home().join("restart/resume.json").exists());
}

#[tokio::test]
async fn multi_agent_queue_is_given_in_order_to_the_interrupted_agent() {
    let mut w = World::start(
        Setup::default()
            .script(Agent::Ling, vec![text("Continuing.")])
            .script(Agent::Yinyue, vec![text("Unexpected replay.")]),
    )
    .await;
    let sid = w.api.create_session(json!({"title": "Shared chat"})).await;
    leave_plan(
        &w,
        json!([{
            "session_id": sid, "agent_id": "ling", "caller": false, "tasks": [],
            "queued": [
                {"text": "ask Yinyue", "images": [PICTURE], "agent_id": "yinyue", "project_root": w.work()},
                {"text": "ask Ling", "images": [PICTURE], "agent_id": "ling", "project_root": w.work()}
            ]
        }]),
    );
    let mark = w.mark();
    w.engine.restart(&w.home).await;
    assert!(matches!(
        w.wait_run(&sid, Agent::Ling, mark).await,
        RunEnd::Finished
    ));
    let calls = w.model.calls_of(Agent::Ling);
    assert_eq!(calls.len(), 1);
    let told = calls[0].thread().last().unwrap().1.clone();
    let first = told.find("ask Yinyue").expect("first message included");
    let second = told.find("ask Ling").expect("second message included");
    assert!(first < second, "{told}");
    assert!(
        told.matches(".png").count() == 2 && !told.contains(PICTURE),
        "{told}"
    );
    assert!(told.contains("yinyue") && told.contains("ling"), "{told}");
    assert!(told.contains("do not replay them blindly"), "{told}");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(w.model.calls_of(Agent::Ling).len(), 1);
    // A resident herald may independently call Yinyue in her own session.
    assert!(w.model.calls_of(Agent::Yinyue).iter().all(|call| {
        !call.text().contains("ask Yinyue") && !call.text().contains("ask Ling")
    }));
    assert!(!w.home.linggen_home().join("restart/resume.json").exists());
}

#[tokio::test]
async fn queue_only_session_continues_with_the_first_queued_agent() {
    let mut w = World::start(
        Setup::default()
            .script(Agent::Ling, vec![text("Unexpected replay.")])
            .script(Agent::Yinyue, vec![text("Checking the queue.")]),
    )
    .await;
    let sid = w.api.create_session(json!({"title": "Queue only"})).await;
    leave_plan(
        &w,
        json!([{
            "session_id": sid,
            "queued": [
                {"text": "first for Yinyue", "images": [PICTURE], "agent_id": "yinyue", "project_root": w.work()},
                {"text": "then for Ling", "images": [], "agent_id": "ling", "project_root": w.work()}
            ]
        }]),
    );
    let mark = w.mark();
    w.engine.restart(&w.home).await;
    assert!(matches!(
        w.wait_run(&sid, Agent::Yinyue, mark).await,
        RunEnd::Finished
    ));
    let calls = w.model.calls_of(Agent::Yinyue);
    assert_eq!(calls.len(), 1);
    let told = calls[0].thread().last().unwrap().1.clone();
    assert!(
        told.find("first for Yinyue") < told.find("then for Ling"),
        "{told}"
    );
    assert!(
        told.contains("first for Yinyue") && told.contains("then for Ling"),
        "{told}"
    );
    assert!(
        told.contains(".png") && !told.contains(PICTURE) && told.contains("yinyue") && told.contains("ling"),
        "{told}"
    );
    assert!(
        told.contains("already processed is unknown")
            && told.contains("do not replay them blindly"),
        "{told}"
    );
    assert!(told.contains("This session had messages queued at the restart"), "{told}");
    assert!(!told.contains("Your turn was running"), "{told}");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(w.model.calls_of(Agent::Yinyue).len(), 1);
    assert!(w.model.calls_of(Agent::Ling).is_empty());
    assert!(!w.home.linggen_home().join("restart/resume.json").exists());
}

#[tokio::test]
async fn idle_background_work_is_reported_and_listed_as_interrupted() {
    let mut w = World::start(Setup::default().script(Agent::Ling, vec![text("Checking the interrupted work.")])).await;
    let sid = w.api.create_session(json!({"title": "Background work"})).await;
    leave_plan(&w, json!([{
        "session_id": sid, "agent_id": "ling", "caller": false, "turn_interrupted": false,
        "tasks": [{"id":"sh-1", "kind":"shell", "title":"make", "status":"running",
                   "started_at":1, "owner_agent":"ling", "child_owner":false}], "queued": []
    }]));
    let mark = w.mark();
    w.engine.restart(&w.home).await;
    assert!(matches!(w.wait_run(&sid, Agent::Ling, mark).await, RunEnd::Finished));
    let calls = w.model.calls_of(Agent::Ling);
    let told = calls[0].thread().last().unwrap().1.clone();
    assert!(told.contains("background work was running"), "{told}");
    assert!(told.contains("make (sh-1)"), "{told}");
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
