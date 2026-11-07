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
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    leave_plan_at(w, now, sessions);
}

/// [`leave_plan`] as a restart asked for at `now` left it.
pub fn leave_plan_at(w: &World, now: u64, sessions: serde_json::Value) {
    let dir = w.home.linggen_home().join("restart");
    std::fs::create_dir_all(&dir).unwrap();
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

/// A plan replayed — the engine went down after telling a session but
/// before the plan was gone — tells nobody twice: each continue turn is kept
/// in the transcript under its mark, and the plan goes only once all are.
#[tokio::test]
async fn a_replayed_plan_does_not_tell_a_session_twice() {
    let mut w = World::start(
        Setup::default().script(Agent::Ling, vec![text("Checking where I was."), text("Told twice.")]),
    )
    .await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let plan = json!([{"session_id": sid, "agent_id": "ling", "caller": false, "tasks": [],
                       "queued": [{"text": "kept for later", "images": [], "agent_id": "ling"}]}]);
    leave_plan_at(&w, at, plan.clone());
    let mark = w.mark();
    w.engine.restart(&w.home).await;
    assert!(matches!(w.wait_run(&sid, Agent::Ling, mark).await, RunEnd::Finished));
    let told = |w: &World| {
        crate::support::rows::read_raw(&w.home.linggen_home(), &sid)
            .into_iter()
            .filter(|row| row["client_id"] == format!("restart-{at}-ling"))
            .count()
    };
    assert_eq!(told(&w), 1, "the continue turn is kept under its mark");
    assert!(!w.home.linggen_home().join("restart/resume.json").exists());
    // The same plan again, as an engine that died before removing it left it.
    leave_plan_at(&w, at, plan);
    w.engine.restart(&w.home).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while w.home.linggen_home().join("restart/resume.json").exists() {
        assert!(Instant::now() < deadline, "a replayed plan that told everyone is retired");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(w.model.calls_of(Agent::Ling).len(), 1, "the session was told twice");
    assert_eq!(told(&w), 1);
}

/// [`leave_plan_at`], the helper having said `outcome` (`ok`, `rolled_back`, …).
fn leave_plan_after(w: &World, now: u64, outcome: serde_json::Value, sessions: serde_json::Value) {
    leave_plan_at(w, now, sessions);
    let mut status = outcome;
    status["at"] = json!(now);
    std::fs::write(w.home.linggen_home().join("restart/status.json"), status.to_string()).unwrap();
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// A restart whose new build would not come up: the helper swaps the old
/// one back and starts it, then says `rolled_back` — and that engine (the
/// old build again) continues every session in the plan at once, the
/// messages queued meanwhile with it, rather than leaving them for the next
/// start.
#[tokio::test]
async fn a_rolled_back_engine_continues_every_session_at_once() {
    let mut w = World::start(Setup::default().script(Agent::Ling, vec![text("Back on the old build.")])).await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    let started = Instant::now();
    leave_plan_after(&w, now_secs(), json!({"state": "rolled_back", "detail": "no /api/health within 60s"}), json!([{
        "session_id": sid, "agent_id": "ling", "caller": true, "tasks": [],
        "queued": [{"text": "sent during the restart", "images": [], "agent_id": "ling"}],
    }]));
    let mark = w.mark();
    w.engine.restart(&w.home).await;
    assert!(matches!(w.wait_run(&sid, Agent::Ling, mark).await, RunEnd::Finished));
    assert!(started.elapsed() < Duration::from_secs(60), "the rolled-back engine waited");
    let told = w.model.calls_of(Agent::Ling)[0].thread().last().unwrap().1.clone();
    assert!(told.contains("the previous one was restored: no /api/health within 60s"), "{told}");
    assert!(told.contains("sent during the restart"), "{told}");
    assert!(!w.home.linggen_home().join("restart/resume.json").exists());
}

/// A plan one session of which can never be told (its transcript cannot
/// be written) does not stay stuck: the other session continues, the plan
/// goes after a few tries, the stuck session's entry — the person's queued
/// message with it — is kept in the record, and a later restart is judged
/// on its own (here refused for its bad build, not for the old plan).
#[tokio::test]
async fn a_plan_that_cannot_be_fully_delivered_retires_and_keeps_the_rest() {
    let mut w = World::start(Setup::default().script(Agent::Ling, vec![text("Continuing.")])).await;
    let good = w.api.create_session(json!({"title": "Good"})).await;
    let stuck = w.api.create_session(json!({"title": "Stuck"})).await;
    let thread = w.home.linggen_home().join(format!("sessions/{stuck}/messages.jsonl"));
    let _ = std::fs::remove_file(&thread);
    std::fs::create_dir_all(&thread).unwrap();
    let at = now_secs();
    leave_plan_after(&w, at, json!({"state": "ok"}), json!([
        {"session_id": good, "agent_id": "ling", "caller": false, "tasks": []},
        {"session_id": stuck, "agent_id": "ling", "caller": false, "tasks": [],
         "queued": [{"text": "the person's words", "images": [], "agent_id": "ling"}]},
    ]));
    let mark = w.mark();
    w.engine.restart(&w.home).await;
    assert!(matches!(w.wait_run(&good, Agent::Ling, mark).await, RunEnd::Finished));
    let plan = w.home.linggen_home().join("restart/resume.json");
    let deadline = Instant::now() + Duration::from_secs(30);
    while plan.exists() {
        assert!(Instant::now() < deadline, "the stuck plan was never retired");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let record = w.home.linggen_home().join(format!("restart/unrecovered-{at}.json"));
    let kept: serde_json::Value = serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
    let sessions = kept["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1, "{kept}");
    assert_eq!(sessions[0]["session_id"], stuck.as_str());
    assert_eq!(sessions[0]["queued"][0]["text"], "the person's words");
    assert!(sessions[0]["not_delivered"].is_string(), "{kept}");
    let bogus = w.home.root.join("not-a-build");
    std::fs::write(&bogus, "#!/bin/sh\nexit 3\n").unwrap();
    std::fs::set_permissions(&bogus, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let status = w.api.post_status("/api/engine/restart", json!({"binary": bogus, "wait_secs": 0})).await;
    assert_eq!(status, 400, "a later restart is not refused for the old plan");
}
