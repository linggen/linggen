//! Plan mode: only ExitPlanMode submits a plan. A reply written in plan mode
//! without it is an answer — shown and saved once, never a plan to approve
//! (2026-10-22: a coffee-machine shortlist came back as a plan).

use crate::support::model::Agent;
use crate::support::rows::Row;
use crate::support::{text, tool, Setup, World};
use serde_json::json;
use std::time::{Duration, Instant};

const SID: &str = "sess-1790000002-c0ac7ed2";

fn is_plan(row: &Row) -> bool {
    row.content.starts_with("{\"type\":\"plan\"")
}

/// Rows after `before`, once `done` holds for them (the plan loop is a
/// second run inside the turn, so one `run/finish` is not the end).
async fn rows_when(w: &World, before: usize, done: impl Fn(&[Row]) -> bool) -> Vec<Row> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
        let rows = w.rows_opt(SID).unwrap_or_default();
        let new = rows.get(before..).unwrap_or_default().to_vec();
        if done(&new) {
            tokio::time::sleep(Duration::from_millis(500)).await;
            return w.rows(SID).split_off(before);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("rows never settled; log:\n{}", w.engine.log());
}

#[tokio::test]
async fn a_reply_in_plan_mode_without_exit_is_an_answer() {
    let answer = "Three picks: Profitec GO, Lelit Elizabeth, Ascaso Steel Duo.";
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![
            tool("EnterPlanMode", json!({"reason": "research first"})),
            text(answer),
        ],
    ))
    .await;
    let before = w.rows(SID).len();
    w.api
        .chat(SID, &w.work(), "Give me more espresso machine options.")
        .await;
    let rows = rows_when(&w, before, |r| {
        r.iter().any(|x| x.said_by("ling") == Some(answer))
    })
    .await;
    let said: Vec<_> = rows.iter().filter_map(|r| r.said_by("ling")).collect();
    assert_eq!(
        said,
        [answer],
        "the answer is not saved exactly once: {rows:#?}"
    );
    assert!(
        !rows.iter().any(is_plan),
        "the answer became a plan: {rows:#?}"
    );
    w.assert_all_scripted();
}

#[tokio::test]
async fn exit_plan_mode_still_submits_a_plan() {
    let plan = "## Plan\n\n1. Add a --verbose flag to run.sh\n2. Test it";
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![
            tool("EnterPlanMode", json!({"reason": "plan the flag"})),
            tool("ExitPlanMode", json!({"plan_text": plan})),
        ],
    ))
    .await;
    let before = w.rows(SID).len();
    w.api
        .chat(
            SID,
            &w.work(),
            "Add a --verbose flag to run.sh, plan it first.",
        )
        .await;
    let rows = rows_when(&w, before, |r| r.iter().any(is_plan)).await;
    let row = rows.iter().find(|r| is_plan(r)).expect("plan row");
    assert!(row.content.contains("--verbose"), "{}", row.content);
    w.assert_all_scripted();
}
