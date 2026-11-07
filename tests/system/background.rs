//! Native background tasks (doc/background-tasks-spec.md): a shell started
//! with `Bash run_in_background` never holds the turn; its end comes back to
//! the agent as a turn of its own, and the person can stop it.

use crate::support::engine::RunEnd;
use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use serde_json::json;
use std::time::{Duration, Instant};

/// Wait until Ling's model has been called `n` times.
async fn until_calls(w: &World, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while w.model.calls_of(Agent::Ling).len() < n {
        assert!(Instant::now() < deadline, "only {} calls", w.model.calls_of(Agent::Ling).len());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// What the model was last told in call `i` (0-based).
fn last_said(w: &World, i: usize) -> String {
    let calls = w.model.calls_of(Agent::Ling);
    calls[i].thread().last().map(|(_, text)| text.clone()).unwrap_or_default()
}

async fn first_turn(w: &World, sid: &str) {
    let mark = w.mark();
    w.api.chat(sid, &w.work(), "Run the tests in the background").await;
    if let RunEnd::Asked(ask) = w.wait_run(sid, Agent::Ling, mark).await {
        w.api.answer(&ask, "grant:all").await;
        w.finish_run(sid, Agent::Ling, mark).await;
    }
}

#[tokio::test]
async fn a_background_shell_ends_the_turn_at_once_and_its_end_is_a_turn_of_its_own() {
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![
            tool("Bash", json!({"cmd": "echo started; sleep 1; echo finished", "run_in_background": true})),
            text("Started it."),
            text("They passed."),
        ],
    ))
    .await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    let started = Instant::now();
    first_turn(&w, &sid).await;
    assert!(last_said(&w, 1).contains("Started in the background as task sh-"), "{}", last_said(&w, 1));
    // The third call is the task's end, told to Ling without a word from the person.
    until_calls(&w, 3).await;
    let notice = last_said(&w, 2);
    assert!(notice.contains("ended: completed (exit code 0)"), "{notice}");
    assert!(notice.contains("finished"), "the output tail is quoted: {notice}");
    assert!(started.elapsed() >= Duration::from_secs(1));
    w.assert_all_scripted();
}

#[tokio::test]
async fn the_person_stops_a_background_shell_and_the_agent_is_told() {
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![
            tool("Bash", json!({"cmd": "sleep 60", "run_in_background": true})),
            text("Started it."),
            text("Noted, it was stopped."),
        ],
    ))
    .await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    first_turn(&w, &sid).await;
    let said = last_said(&w, 1);
    let id = said
        .split("as task ")
        .nth(1)
        .and_then(|rest| rest.split('.').next())
        .expect("a task id")
        .to_string();
    assert_eq!(
        w.api.post_status("/api/tasks/stop", json!({"session_id": sid, "id": id})).await,
        200
    );
    until_calls(&w, 3).await;
    assert!(last_said(&w, 2).contains("ended: stopped"), "{}", last_said(&w, 2));
    w.assert_all_scripted();
}

fn running(marker: &str) -> bool {
    std::process::Command::new("pgrep")
        .args(["-f", marker])
        .output()
        .map(|out| !out.stdout.is_empty())
        .unwrap_or(false)
}

#[tokio::test]
async fn deleting_a_session_ends_its_background_shell_and_tells_no_one() {
    let marker = format!("bg-delete-{}", uuid::Uuid::new_v4().simple());
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![
            tool("Bash", json!({"cmd": format!("sleep 600; echo {marker}"), "run_in_background": true})),
            text("Started it."),
        ],
    ))
    .await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    first_turn(&w, &sid).await;
    assert!(running(&marker), "the background shell never started");
    w.api.delete("/api/sessions/all", json!({"session_id": sid})).await;
    assert!(!running(&marker), "the shell outlived its session");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(w.model.calls_of(Agent::Ling).len(), 2, "the deleted session got a turn");
    w.assert_all_scripted();
}

/// Wait until Ling's model has been called `n` times, granting every
/// permission asked in `sid` on the way.
async fn until_calls_granting(w: &World, sid: &str, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while w.model.calls_of(Agent::Ling).len() < n {
        assert!(Instant::now() < deadline, "only {} calls", w.model.calls_of(Agent::Ling).len());
        for ask in w.api.pending(sid, None).await {
            w.api.answer(&ask, "grant:all").await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A delegated run does not end while its own background shell runs: the
/// shell's end is its next turn, and what it says then is the result its
/// caller gets (2026-11-06, sess-1793970673: the child's end went to a run
/// already over, and the caller heard only "started").
#[tokio::test]
async fn a_subagent_waits_for_its_background_shell_and_its_caller_gets_the_informed_result() {
    let marker = format!("child-out-{}", uuid::Uuid::new_v4().simple());
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![
            // 0, the main chat: a foreground subagent.
            tool("Task", json!({"target_agent_id": "ling", "task": "Run the check in the background and report what it printed"})),
            // 1-3, the subagent: starts its shell, replies, is told its end.
            tool("Bash", json!({"cmd": format!("sleep 1; echo {marker}"), "run_in_background": true})),
            text("Started the check."),
            text("The check finished and printed its marker."),
            // 4, the main chat again, with the subagent's result.
            text("It reported back."),
        ],
    ))
    .await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "Have a helper run the check").await;
    until_calls_granting(&w, &sid, 5).await;
    let notice = last_said(&w, 3);
    assert!(notice.contains("ended: completed (exit code 0)"), "the subagent was not told: {notice}");
    assert!(notice.contains(&marker), "{notice}");
    let result = last_said(&w, 4);
    assert!(
        result.contains("The check finished and printed its marker."),
        "the caller got the subagent's early reply: {result}"
    );
    w.finish_run(&sid, Agent::Ling, mark).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(w.model.calls_of(Agent::Ling).len(), 5, "a notice came back as an extra turn");
    w.assert_all_scripted();
}

/// Stop on the chat stops a subagent waiting for its own shell, and the
/// shell with it.
#[tokio::test]
async fn stop_ends_a_subagent_waiting_for_its_shell_and_the_shell() {
    let marker = format!("child-wait-{}", uuid::Uuid::new_v4().simple());
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![
            tool("Task", json!({"target_agent_id": "ling", "task": "Start the server in the background"})),
            tool("Bash", json!({"cmd": format!("sleep 600; echo {marker}"), "run_in_background": true})),
            text("Started the server."),
        ],
    ))
    .await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "Have a helper start the server").await;
    until_calls_granting(&w, &sid, 3).await;
    assert!(running(&marker), "the subagent's shell never started");
    let run_id = w
        .engine
        .log_since(mark)
        .lines()
        .find(|l| l.contains("run/begin") && l.contains(&sid))
        .and_then(|l| l.split("run_id=").nth(1))
        .map(|rest| rest.split_whitespace().next().unwrap().to_string())
        .expect("the chat's run");
    tokio::time::sleep(Duration::from_millis(300)).await;
    w.api.post("/api/agent-cancel", json!({"run_id": run_id})).await;
    let deadline = Instant::now() + Duration::from_secs(8);
    while running(&marker) {
        assert!(Instant::now() < deadline, "the subagent's shell outlived Stop");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    while !w.engine.log_since(mark).lines().any(|l| l.contains("run/finish") && l.contains(&run_id)) {
        assert!(Instant::now() < deadline, "the chat's run never ended after Stop");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(w.model.calls_of(Agent::Ling).len(), 3);
    w.assert_all_scripted();
}
