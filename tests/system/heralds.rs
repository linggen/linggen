//! What Yinyue tells a person about Ling's work (doc/yinyue-companion-spec.md
//! § Herald): one notice per thing, told by voice only while the person is
//! away, never a row of its own; the thing it is about carries a folded mark;
//! one telling per waiting session; a run another tool started is never told;
//! and a turn the engine started for her that ends without words leaves
//! nothing on any page.

use crate::notes::{she_writes_the_note, the_clock_moves};
use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use serde_json::Value;
use std::time::Duration;

/// The events of hers a thread page would draw: anything she sends that is
/// not a cue of her stage (`pet_*`), from `from` on.
fn her_thread_events(w: &World, from: usize) -> Vec<Value> {
    w.events()
        .into_iter()
        .skip(from)
        .filter(|e| e["agent_id"] == "yinyue")
        .filter(|e| !e["kind"].as_str().unwrap_or("").starts_with("pet_"))
        .collect()
}

/// A turn the engine started for her (a note come due) that ends without
/// words sends nothing a thread draws: no status, no run, no token count.
#[tokio::test]
async fn a_silent_turn_of_hers_shows_nothing_on_any_page() {
    let w = World::start(crate::notes::script(text("He is away. Nothing to say."))).await;
    she_writes_the_note(&w).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let from = w.events().len();
    let mark = w.mark();
    the_clock_moves(&w);
    w.wait_agent(Agent::Yinyue, mark).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let shown = her_thread_events(&w, from);
    let kinds: Vec<String> = shown.iter().map(|e| format!("{} {}", e["kind"], e["phase"])).collect();
    assert!(shown.is_empty(), "a silent turn of hers reached a page: {kinds:?}");
}

/// Her thread as a page loads it: the rows she wrote, each `[meta, text]`.
async fn her_rows_on_load(w: &World) -> Vec<Value> {
    let sid = w
        .events()
        .iter()
        .rev()
        .filter(|e| e["agent_id"] == "yinyue")
        .find_map(|e| e["session_id"].as_str().map(str::to_string))
        .expect("her session");
    let url = format!("/api/workspace/state?session_id={sid}&project_root={}", urlencoding::encode(&w.work()));
    let page = w.api.get(&url).await;
    page["messages"].as_array().cloned().unwrap_or_default().into_iter().filter(|r| r[0]["from"] == "yinyue").collect()
}

/// After a reload too: a held turn of hers that used a tool and said
/// nothing leaves no row — not its call, not an ended run.
#[tokio::test]
async fn a_silent_turn_that_used_a_tool_shows_nothing_after_a_reload() {
    let setup = Setup::default().script(
        Agent::Yinyue,
        vec![
            tool("note_write", serde_json::json!({"text": "the parcel arrives tomorrow", "look_again": "+40m"})),
            text("Noted."),
            tool("recent_activity", serde_json::json!({})),
            text("Nothing new. No line."),
        ],
    );
    let w = World::start(setup).await;
    she_writes_the_note(&w).await;
    let before = her_rows_on_load(&w).await;
    let mark = w.mark();
    the_clock_moves(&w);
    w.wait_agent(Agent::Yinyue, mark).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let after = her_rows_on_load(&w).await;
    assert_eq!(after, before, "a silent turn of hers left a row on reload");
}
