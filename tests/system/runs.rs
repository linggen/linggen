//! Every run stamps itself on what it sends and saves (chat-spec.md § Runs):
//! a turn with words between its calls, a stopped turn, and the turn after
//! it — each event a surface reads for a run and each row a run saves names
//! that run, so the chat binds them by id instead of guessing.

use crate::support::engine::RunEnd;
use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// UI event kinds that only a run sends.
const RUN_KINDS: &[&str] = &[
    "token",
    "text_segment",
    "content_block",
    "tool_progress",
    "turn_complete",
];

/// Senders whose rows open a turn or come before its run: never a run's.
const NOT_A_RUN: &[&str] = &["user", "memory-recall"];

#[tokio::test]
async fn every_run_event_and_row_names_its_run() {
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![
            tool("Glob", json!({"pattern": "*.md"})),
            text("Two notes here."),
            tool(
                "Bash",
                json!({"cmd": "echo started; sleep 120", "timeout_ms": 180_000}),
            ),
            tool("Glob", json!({"pattern": "*.md"})),
            text("Still here."),
        ],
    ))
    .await;
    let root = w.work();
    let sid = w
        .api
        .create_session(json!({"title": "runs", "project_root": root}))
        .await;

    w.turn(&sid, &root, "List the notes.", Agent::Ling).await;
    stop_a_sleeping_run(&w, &sid, &root).await;
    w.turn(&sid, &root, "Still there?", Agent::Ling).await;

    let rows = w.raw_rows(&sid);
    let row_runs = runs_of_rows(&rows);
    assert_eq!(row_runs.len(), 3, "three turns, three runs: {rows:#?}");

    let events: Vec<Value> = w
        .events()
        .into_iter()
        .filter(|e| e["session_id"] == sid.as_str())
        .collect();
    for e in events.iter().filter(|e| belongs_to_a_run(e)) {
        assert!(e["run_id"].is_string(), "a run's event has no run_id: {e}");
    }
    // Not a check of nothing: each kind a run sends came. (The fake model
    // cannot write words beside a call, so the text segment checked is a
    // final reply's — the same path saves and sends both.)
    for kind in [
        "token",
        "text_segment",
        "content_block",
        "turn_complete",
        "message",
    ] {
        assert!(
            events
                .iter()
                .any(|e| e["kind"] == kind && belongs_to_a_run(e)),
            "no {kind} event from a run"
        );
    }
    // Each turn's end names the run whose rows it closes, in order.
    let ends: Vec<String> = events
        .iter()
        .filter(|e| e["kind"] == "turn_complete")
        .filter_map(|e| e["run_id"].as_str().map(str::to_string))
        .collect();
    assert_eq!(ends, row_runs, "turn ends vs the runs on disk");
    // A run's outcome ends its bubble: it follows the run's own blocks and
    // message, and comes before the turn's end.
    let outcomes: Vec<(usize, &str)> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| e["kind"] == "run" && e["phase"] == "outcome")
        .filter_map(|(i, e)| e["run_id"].as_str().map(|r| (i, r)))
        .collect();
    // A stop ends its run with an outcome or an error; the other two always
    // end with one.
    assert!(
        outcomes.len() >= 2,
        "runs did not say they ended: {outcomes:?}"
    );
    for (at, run) in outcomes {
        let of_run = |e: &Value| e["run_id"] == run;
        let last_own = events
            .iter()
            .rposition(|e| of_run(e) && (e["kind"] == "content_block" || e["kind"] == "message"));
        assert!(
            last_own.is_some_and(|l| l < at),
            "{run}'s outcome overtook its blocks"
        );
        let end = events
            .iter()
            .position(|e| of_run(e) && e["kind"] == "turn_complete");
        assert!(
            end.is_some_and(|t| t > at),
            "{run}'s outcome came after its turn's end"
        );
    }
    // The stopped run's interrupted row carries its run.
    let history = w.api.history(&sid, &root).await;
    let cut = history["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| r[0]["run"] == "interrupted")
        .unwrap_or_else(|| panic!("no interrupted row: {history:#}"));
    assert_eq!(cut[0]["run_id"], row_runs[1].as_str(), "{cut}");
    w.assert_all_scripted();
}

/// Ask for a long Bash, allow it, stop it while it runs.
async fn stop_a_sleeping_run(w: &World, sid: &str, root: &str) {
    let mark = w.mark();
    w.api.chat(sid, root, "Wait a while.").await;
    let RunEnd::Asked(ask) = w.wait_run(sid, Agent::Ling, mark).await else {
        panic!("the Bash ran without a permission prompt");
    };
    w.api.answer(&ask, "allow once").await;
    let run_id = running_bash_run(w).await;
    w.api
        .post("/api/agent-cancel", json!({ "run_id": run_id }))
        .await;
    w.finish_run(sid, Agent::Ling, mark).await;
}

/// The run of the Bash call now on screen, read off its block's event.
async fn running_bash_run(w: &World) -> String {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        let found = w.events().into_iter().rev().find(|e| {
            e["kind"] == "content_block" && e["phase"] == "start" && e["data"]["tool"] == "Bash"
        });
        if let Some(run) = found.and_then(|e| e["run_id"].as_str().map(str::to_string)) {
            return run;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no Bash block with a run_id came: {:#?}", w.events());
}

/// The distinct runs of the saved rows, in order; every row a run wrote
/// must name one.
fn runs_of_rows(rows: &[Value]) -> Vec<String> {
    let mut runs: Vec<String> = Vec::new();
    for r in rows {
        let from = r["from_id"].as_str().unwrap_or("");
        if NOT_A_RUN.contains(&from) {
            assert!(
                r.get("run_id").is_none(),
                "a turn's opener names a run: {r}"
            );
            continue;
        }
        let run = r["run_id"]
            .as_str()
            .unwrap_or_else(|| panic!("a run's row has no run_id: {r}"));
        if runs.last().map(String::as_str) != Some(run) {
            runs.push(run.to_string());
        }
    }
    runs
}

/// A run's own event: a stream/turn kind, the agent's message, its
/// outcome, or the idle that ends it.
fn belongs_to_a_run(e: &Value) -> bool {
    let kind = e["kind"].as_str().unwrap_or("");
    match kind {
        k if RUN_KINDS.contains(&k) => true,
        "message" => e["data"]["from"] == "ling",
        "run" => e["phase"] == "outcome",
        "activity" => e["data"]["status"] == "idle",
        _ => false,
    }
}
