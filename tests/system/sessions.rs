//! Prewritten sessions (tests/fixtures/home/sessions): a shared table, a
//! compacted chat, a run cut off before its reply — read back the way the
//! engine and the chat page read them.

use crate::support::model::Agent;
use crate::support::{snap, snap_text, text, Setup, World};

const SHARED: &str = "sess-1790000001-5a1ed001";
const COMPACTED: &str = "sess-1790000002-c0ac7ed2";
const INTERRUPTED: &str = "sess-1790000003-1e7e5ed3";

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
}
