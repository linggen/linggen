//! Her own notes and their look-again (doc/yinyue-companion-spec.md § Notes):
//! she writes a note with a time, the time comes, she is woken with the note
//! in front of her — and says a line, or nothing, or the hour's ceiling holds
//! the wake.
//!
//! The engine is its own process with the real clock, so "the clock moves"
//! here is the note's look-again time moved behind it in her notes file, as
//! forty minutes passing would leave it.

use crate::support::model::{Agent, Call};
use crate::support::{text, tool, Setup, World};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// What the engine's own side of the woken turn may use.
const QUIET: [&str; 4] = ["note_done", "note_write", "recent_activity", "sense"];

fn script(woken: &str) -> Setup {
    Setup::default().script(
        Agent::Yinyue,
        vec![
            tool(
                "note_write",
                json!({"text": "the parcel arrives tomorrow", "look_again": "+40m"}),
            ),
            text("Noted."),
            text(woken),
        ],
    )
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn notes_file(w: &World) -> std::path::PathBuf {
    w.home.linggen_home().join("notes/yinyue.json")
}

fn her_notes(w: &World) -> Vec<Value> {
    let text = std::fs::read_to_string(notes_file(w)).expect("her notes file");
    let book: Value = serde_json::from_str(&text).expect("notes json");
    book["notes"].as_array().cloned().unwrap_or_default()
}

/// She is asked to remember; her note is kept with its time, forty minutes on.
async fn she_writes_the_note(w: &World) {
    let mark = w.mark();
    w.api
        .post(
            "/api/yinyue/chat",
            json!({"text": "Keep the parcel in mind."}),
        )
        .await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let notes = her_notes(w);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0]["id"], "n1");
    assert_eq!(notes[0]["text"], "the parcel arrives tomorrow");
    let at = notes[0]["look_again_at"]
        .as_u64()
        .expect("a look-again time");
    let ahead = at.saturating_sub(now());
    assert!((2280..=2400).contains(&ahead), "+40m is {ahead}s ahead");
}

/// Forty minutes pass: the look-again time is now a minute behind.
fn the_clock_moves(w: &World) {
    let path = notes_file(w);
    let mut book: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    book["notes"][0]["look_again_at"] = json!(now() - 60);
    std::fs::write(&path, serde_json::to_vec_pretty(&book).unwrap()).unwrap();
}

/// Her woken turn: the note named as the reason, her notes in front of her,
/// the quiet tools and nothing else.
fn assert_woken_with_the_note(call: &Call) {
    let thread = call.thread();
    // The turn's own words, the last user row (its world block follows it).
    let (_, kickoff) = thread
        .iter()
        .rev()
        .find(|(role, _)| role == "user")
        .expect("a kickoff");
    assert!(kickoff.contains("look again at your note n1"), "{kickoff}");
    assert!(kickoff.contains("the parcel arrives tomorrow"), "{kickoff}");
    let notes = thread
        .iter()
        .find(|(_, t)| t.starts_with("# Your notes"))
        .unwrap_or_else(|| panic!("no notes block: {thread:?}"));
    assert!(
        notes.1.contains("- n1 · written") && notes.1.contains(": the parcel arrives tomorrow"),
        "{}",
        notes.1
    );
    let mut offered = call.tool_names();
    offered.sort();
    assert_eq!(offered, QUIET, "a woken note may speak and ask, never act");
}

fn spoken(w: &World) -> Vec<String> {
    w.events()
        .into_iter()
        .filter(|e| e["kind"] == "pet_speak")
        .filter_map(|e| e["text"].as_str().map(String::from))
        .collect()
}

/// The note after its wake: still open, without a time.
fn assert_open_without_a_time(w: &World) {
    let notes = her_notes(w);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].get("look_again_at").is_none(), "{notes:?}");
}

#[tokio::test]
async fn a_woken_note_is_in_front_of_her_and_she_may_stay_silent() {
    let w = World::start(script("SILENT")).await;
    she_writes_the_note(&w).await;
    let mark = w.mark();
    the_clock_moves(&w);
    w.wait_agent(Agent::Yinyue, mark).await;
    let calls = w.model.calls_of(Agent::Yinyue);
    assert_woken_with_the_note(calls.last().unwrap());
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(spoken(&w), ["Noted."], "SILENT says nothing");
    assert_open_without_a_time(&w);
    w.assert_all_scripted();
}

#[tokio::test]
async fn a_woken_note_may_have_her_speak() {
    let line = "The parcel comes tomorrow. Don't miss it.";
    let w = World::start(script(line)).await;
    she_writes_the_note(&w).await;
    let mark = w.mark();
    the_clock_moves(&w);
    w.wait_agent(Agent::Yinyue, mark).await;
    assert_woken_with_the_note(w.model.calls_of(Agent::Yinyue).last().unwrap());
    let said = spoken(&w);
    assert_eq!(said.last().map(String::as_str), Some(line), "{said:?}");
    assert_open_without_a_time(&w);
    w.assert_all_scripted();
}

async fn set_ceiling(w: &World, per_hour: u32) {
    let mut config = w.api.get("/api/config").await;
    config["pet"]["unprompted_per_hour"] = json!(per_hour);
    w.api.post("/api/config", config).await;
}

/// The activity log's `hold` records.
fn held(w: &World) -> Vec<Value> {
    let dir = w.home.linggen_home().join("activity");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .flat_map(|e| {
            let text = std::fs::read_to_string(e.path()).unwrap_or_default();
            text.lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .collect::<Vec<_>>()
        })
        .filter(|a| a["verb"] == "hold")
        .collect()
}

async fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(40) {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("{what} did not happen in 40s");
}

#[tokio::test]
async fn the_hourly_ceiling_holds_the_wake_once_and_lets_it_go_after() {
    let w = World::start(script("SILENT")).await;
    she_writes_the_note(&w).await;
    set_ceiling(&w, 0).await;
    the_clock_moves(&w);
    wait_for("a hold record", || !held(&w).is_empty()).await;
    // Several ticks later: still one record, no turn, the time unspent.
    tokio::time::sleep(Duration::from_secs(12)).await;
    let records = held(&w);
    assert_eq!(records.len(), 1, "one record per held wake: {records:?}");
    assert_eq!(records[0]["by"], "yinyue");
    assert_eq!(records[0]["object"], "a look-again on n1 (hourly ceiling)");
    assert_eq!(records[0]["detail"]["source"], "note");
    assert_eq!(
        w.model.calls_of(Agent::Yinyue).len(),
        2,
        "she was not woken"
    );
    assert!(
        her_notes(&w)[0]["look_again_at"].is_u64(),
        "held, not used up"
    );

    // The ceiling lifted: the held wake comes.
    let mark = w.mark();
    set_ceiling(&w, 6).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    assert_woken_with_the_note(w.model.calls_of(Agent::Yinyue).last().unwrap());
    assert_open_without_a_time(&w);
    w.assert_all_scripted();
}

/// At an app's table her notes follow her: shown, and hers to finish there,
/// whatever tools the skill names for her (the dice table names one).
#[tokio::test]
async fn at_an_apps_table_her_notes_are_in_front_of_her_and_hers_to_finish() {
    let w = World::start(Setup::default().script(
        Agent::Yinyue,
        vec![
            tool("note_write", json!({"text": "the parcel arrives tomorrow"})),
            text("Noted."),
            tool("note_done", json!({"id": "n1"})),
            text("Clouds over the harbor."),
        ],
    ))
    .await;
    let mark = w.mark();
    w.api
        .post(
            "/api/yinyue/chat",
            json!({"text": "Keep the parcel in mind."}),
        )
        .await;
    w.wait_agent(Agent::Yinyue, mark).await;
    assert_eq!(her_notes(&w).len(), 1);

    let sid = w
        .api
        .create_session(json!({"title": "table", "skill": "dice"}))
        .await;
    let mark = w.mark();
    let moment = json!({"app": "dice", "text": "The player looks at the sky.", "asked": true, "session": sid});
    assert_eq!(w.api.post("/api/yinyue/event", moment).await, json!("ok"));
    w.finish_run(&sid, Agent::Yinyue, mark).await;

    let calls = w.model.calls_of(Agent::Yinyue);
    let at_table = &calls[2];
    let thread = at_table.thread();
    assert!(
        thread.iter().any(
            |(_, t)| t.starts_with("# Your notes") && t.contains("the parcel arrives tomorrow")
        ),
        "{thread:?}"
    );
    let offered = at_table.tool_names();
    assert!(offered.iter().any(|t| t == "note_done"), "{offered:?}");
    assert!(her_notes(&w).is_empty(), "finished at the table");
    w.assert_all_scripted();
}
