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
        w.api.answer(&ask, "allow once").await;
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
