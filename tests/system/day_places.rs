//! What a page is told of the place the day's work stands in
//! (doc/one-ling-spec.md § Linggen's daily session): with the daily session
//! on, a page at one app sees work from another, so the engine names the
//! place on what it sends — the running turn's events, a waiting message —
//! and the page shows it. Anywhere else nothing names a place.

use crate::day_session::{day_world_at, install_ledger, say};
use crate::support::model::Agent;
use crate::support::engine::RunEnd;
use crate::support::{text, tool, Setup, World};
use serde_json::{json, Value};

/// A turn at the dice page held open by its question, and a message sent
/// from quiet's page while it runs: the session, and the events up to the
/// queue's (read before the question is answered, while the turn runs).
async fn dice_running_quiet_waiting(w: &World, day: &str) -> Vec<Value> {
    let mark = w.mark();
    say(w, day, Some("dice"), "Pick a box").await;
    let RunEnd::Asked(asked) = w.wait_run(day, Agent::Ling, mark).await else {
        panic!("no question");
    };
    let sent = say(w, day, Some("quiet"), "QUIET words").await;
    assert_eq!(sent["status"], "queued", "{sent}");
    let events = w.events_when(|evs| !queue_items(evs, day).is_empty()).await;
    let mark = w.mark();
    w.api.answer(&asked, "Red").await;
    w.finish_run(day, Agent::Ling, mark).await;
    events
}

fn held_setup() -> Setup {
    let ask = json!({"questions": [{"question": "Which box?", "header": "Box",
        "options": [{"label": "Red"}, {"label": "Blue"}]}]});
    Setup::default().script(
        Agent::Ling,
        vec![tool("AskUser", ask), text("Red."), text("Quiet answered.")],
    )
}

/// The items of every queue event of `sid`.
fn queue_items(events: &[Value], sid: &str) -> Vec<Value> {
    events
        .iter()
        .filter(|e| e["kind"] == "queue" && e["session_id"] == sid)
        .flat_map(|e| e["data"]["items"].as_array().cloned().unwrap_or_default())
        .collect()
}

/// The events of `sid` of one of `kinds`.
fn of_kinds<'a>(events: &'a [Value], sid: &str, kinds: &[&str]) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|e| e["session_id"] == sid && kinds.iter().any(|k| e["kind"] == *k))
        .collect()
}

async fn the_day(w: &World) -> String {
    w.api.create_session(json!({"title": "Dice", "skill": "dice"})).await
}

/// While a turn from the dice page runs, its status events name the place
/// it stands in, so another page can say so beside its Stop.
#[tokio::test]
async fn in_the_day_a_running_turns_status_names_its_place() {
    let (w, _) = day_world_at(12, held_setup()).await;
    let day = the_day(&w).await;
    let events = dice_running_quiet_waiting(&w, &day).await;
    let status = of_kinds(&events, &day, &["activity"]);
    let running: Vec<_> = status.iter().filter(|e| e["phase"] != "done").collect();
    assert!(!running.is_empty(), "no status event: {events:#?}");
    for e in running {
        assert_eq!(e["place"], "dice", "a status event names no place: {e}");
    }
}

/// With the setting off, nothing the engine sends names a place.
#[tokio::test]
async fn with_the_setting_off_nothing_names_a_place() {
    let w = World::start(held_setup().setting_off()).await;
    let sid = the_day(&w).await;
    assert!(!sid.starts_with("sess-day-"), "{sid}");
    dice_running_quiet_waiting(&w, &sid).await;
    let named: Vec<_> = w.events().into_iter().filter(names_a_place).collect();
    assert!(named.is_empty(), "named a place: {named:#?}");
}

fn names_a_place(e: &Value) -> bool {
    let items = e["data"]["items"].as_array().cloned().unwrap_or_default();
    e.get("place").is_some() || items.iter().any(|i| i.get("place").is_some())
}

/// A private app is never a place in the day: its page naming the day
/// stands in its own chat, and nothing sent names it.
#[tokio::test]
async fn a_private_app_is_never_a_told_place() {
    let setup = Setup::default().script(Agent::Ling, vec![text("LEDGER-REPLY kept.")]);
    let (w, _) = day_world_at(12, setup).await;
    install_ledger(&w).await;
    let day = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let mark = w.mark();
    let sent = say(&w, &day, Some("ledger"), "LEDGER-ONLY words").await;
    let own = sent["session_id"].as_str().unwrap_or_default().to_string();
    assert_ne!(own, day, "{sent}");
    w.finish_run(&own, Agent::Ling, mark).await;
    let events = w.events_when(|evs| evs.iter().any(|e| e["kind"] == "message" && e["session_id"] == own.as_str())).await;
    let named: Vec<_> = events.iter().filter(|e| names_a_place(e)).collect();
    assert!(named.is_empty(), "named a place: {named:#?}");
}
