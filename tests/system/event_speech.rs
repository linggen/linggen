//! A turn the engine started for her shows only what she chose to say: one
//! `<say>…</say>` that is the last thing she wrote. Anything else — bare
//! prose, the reason she is silent, a broken envelope — reaches no thread,
//! no live event and no voice. Driven through three wakes that run end to
//! end: a look-again note, an unasked app moment, and a mission's fault.

use crate::missions::{beat_present, failing_run_script, install_beacon, pet_lines, run_once, set_exit};
use crate::notes::{she_writes_the_note, spoken, the_clock_moves};
use crate::support::model::{Agent, Reply};
use crate::support::{say, text, Setup, World};
use serde_json::json;
use std::time::Duration;

/// What she writes in the turn, and why none of it may reach anyone.
fn unspoken() -> [(&'static str, &'static str); 4] {
    [
        ("bare prose", "BARE-PROSE the parcel comes tomorrow."),
        ("a reason for silence", "他不在，先不说。 WHY-SILENT"),
        ("an unclosed envelope", "<say>UNCLOSED-LINE the parcel"),
        ("text after the envelope", "<say>AFTER-LINE</say> and then more words"),
    ]
}

/// Every row of every session's thread, as one text.
fn every_row(w: &World) -> String {
    let dir = w.home.linggen_home().join("sessions");
    let mut all = String::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path().join("messages.jsonl");
        all.push_str(&std::fs::read_to_string(path).unwrap_or_default());
    }
    all
}

fn marker(reply: &str) -> &str {
    ["BARE-PROSE", "WHY-SILENT", "UNCLOSED-LINE", "AFTER-LINE"]
        .into_iter()
        .find(|m| reply.contains(m))
        .expect("a marker")
}

fn nothing_shown(w: &World, why: &str, reply: &str) {
    let m = marker(reply);
    assert!(!every_row(w).contains(m), "{why}: reached a thread");
    let events = serde_json::to_string(&w.events()).unwrap();
    assert!(!events.contains(m), "{why}: reached a live event");
}

#[tokio::test]
async fn a_woken_note_that_does_not_use_the_envelope_says_nothing() {
    for (why, reply) in unspoken() {
        let w = World::start(crate::notes::script(text(reply))).await;
        she_writes_the_note(&w).await;
        let mark = w.mark();
        the_clock_moves(&w);
        w.wait_agent(Agent::Yinyue, mark).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        nothing_shown(&w, why, reply);
        assert_eq!(spoken(&w), ["Noted."], "{why}: a line was spoken");
    }
}

#[tokio::test]
async fn thinking_then_one_final_envelope_speaks_exactly_that_line_once() {
    let reply = "He is away. Let me think. <say>The parcel comes tomorrow.</say>";
    let w = World::start(crate::notes::script(text(reply))).await;
    she_writes_the_note(&w).await;
    let mark = w.mark();
    the_clock_moves(&w);
    w.wait_agent(Agent::Yinyue, mark).await;
    w.events_when(|e| e.iter().any(|x| x["kind"] == "pet_speak" && x["text"] == "The parcel comes tomorrow.")).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let said = spoken(&w);
    let line = "The parcel comes tomorrow.";
    assert_eq!(said.iter().filter(|s| *s == line).count(), 1, "{said:?}");
    let rows = every_row(&w);
    assert_eq!(rows.matches(line).count(), 1, "one row for the line: {rows}");
    assert!(!rows.contains("Let me think"), "her thinking reached a thread");
}

async fn dice_in_front(w: &World) {
    let body = json!({"surface": "main-ui", "focused": true, "typing": false, "idle_ms": 10_000, "app": "dice"});
    w.api.post("/api/presence", body).await;
}

#[tokio::test]
async fn an_app_moment_answered_without_the_envelope_says_nothing() {
    for (why, reply) in unspoken() {
        let setup = Setup::default().script(Agent::Yinyue, vec![text(reply)]);
        let w = World::start(setup).await;
        let mark = w.mark();
        dice_in_front(&w).await;
        let moment = json!({"app": "dice", "text": "The dice came up snake eyes.", "big": true});
        w.api.post("/api/yinyue/event", moment).await;
        w.wait_agent(Agent::Yinyue, mark).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        nothing_shown(&w, why, reply);
        assert!(spoken(&w).is_empty(), "{why}: {:?}", spoken(&w));
    }
}

#[tokio::test]
async fn a_fault_answered_without_the_envelope_says_nothing() {
    for (why, reply) in unspoken() {
        let setup = failing_run_script_with(text(reply));
        let w = World::start(setup).await;
        install_beacon(&w.home.linggen_home().join("skills"));
        w.api.post("/api/skills/reload", json!({})).await;
        set_exit(&w, 3);
        beat_present(&w).await;
        let mark = w.mark();
        run_once(&w).await;
        w.wait_agent(Agent::Yinyue, mark).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        nothing_shown(&w, why, reply);
        assert!(pet_lines(&w).is_empty(), "{why}: {:?}", pet_lines(&w));
    }
}

/// The mission script, with her reply to the fault chosen.
fn failing_run_script_with(reply: Reply) -> Setup {
    let _ = failing_run_script;
    Setup::default()
        .script(
            Agent::Ling,
            vec![
                crate::support::tool("Ping", json!({})),
                text("The lamp check did not complete. DONE"),
            ],
        )
        .script(Agent::Yinyue, vec![reply])
}

#[tokio::test]
async fn a_person_started_turn_is_shown_as_it_always_was() {
    let w = World::start(Setup::default().script(Agent::Yinyue, vec![text("Here, with you.")])).await;
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "Are you there?"})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    w.events_when(|e| e.iter().any(|x| x["kind"] == "pet_speak")).await;
    assert_eq!(spoken(&w), ["Here, with you."]);
    let _ = say("unused");
}
