//! Her words are spoken aloud only when the person cannot see them
//! (doc/yinyue-companion-spec.md § Heard or read): a line shows on the page of
//! the place it was said in — an app's page, or the main page — and while
//! the person is at that page it goes out as words only (`voice: false`, no
//! clauses to play). Anywhere else, or with nobody at a screen, it is spoken.

use crate::day_session::{day_world_at, say};
use crate::support::model::Agent;
use crate::support::{text, Setup, World};
use serde_json::{json, Value};

/// The fixture's voice is off; these worlds need it on.
async fn unmute(w: &World) {
    assert_eq!(w.api.post("/api/yinyue/chat", json!({"text": "/unmute"})).await, json!("ok"));
}

/// A page's beat: `app` in front (none: the main page), read 30 s ago.
async fn at_page(w: &World, app: Option<&str>) {
    let mut body = json!({"surface": "page", "focused": true, "typing": false, "idle_ms": 30_000});
    if let Some(app) = app {
        body["app"] = json!(app);
    }
    w.api.post("/api/presence", body).await;
}

/// The same page, hidden behind another window.
async fn page_hidden(w: &World) {
    let body = json!({"surface": "page", "focused": false, "typing": false, "idle_ms": 0});
    w.api.post("/api/presence", body).await;
}

/// Her one line on the stage, once it is out.
async fn her_line(w: &World, line: &str) -> Value {
    let events = w
        .events_when(|e| e.iter().any(|x| x["kind"] == "pet_speak" && x["text"] == line))
        .await;
    let said = events
        .into_iter()
        .find(|x| x["kind"] == "pet_speak" && x["text"] == line);
    said.unwrap_or_else(|| panic!("her line never reached the stage; log:\n{}", w.engine.log()))
}

fn spoken(cue: &Value) -> bool {
    let voice = cue["data"]["voice"].as_bool().expect("a voice flag");
    let clauses = cue["data"]["chunks"].as_array().map_or(0, Vec::len);
    assert_eq!(voice, clauses > 0, "clauses only for a spoken line: {cue}");
    voice
}

/// The person asks her something from a page, here the dice app's, while the
/// day's session is on.
async fn ask_from_dice(w: &World, day: &str) {
    let mark = w.mark();
    let moment = json!({"app": "dice", "text": "The player asks what the sky says.", "asked": true, "session": day});
    assert_eq!(w.api.post("/api/yinyue/event", moment).await, json!("ok"));
    w.finish_run(day, Agent::Yinyue, mark).await;
}

async fn the_day(w: &World) -> String {
    w.api.get("/api/sessions/day").await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Use case 1: typed to her on the main page, read there — text only.
#[tokio::test]
async fn an_answer_on_the_main_page_she_is_read_on_is_text_only() {
    let w = World::start(Setup::default().setting_off().script(Agent::Yinyue, vec![text("Right here.")])).await;
    unmute(&w).await;
    at_page(&w, None).await;
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "Are you there?"})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    assert!(!spoken(&her_line(&w, "Right here.").await));
}

/// Use case 1, in an app: addressed in the app's chat while its page is in
/// front — text only.
#[tokio::test]
async fn an_answer_in_the_app_page_in_front_is_text_only() {
    let setup = Setup::default().script(Agent::Yinyue, vec![text("Six, of course.")]);
    let (w, _) = day_world_at(12, setup).await;
    unmute(&w).await;
    let day = the_day(&w).await;
    at_page(&w, Some("dice")).await;
    let mark = w.mark();
    say(&w, &day, Some("dice"), "@Yinyue what will I roll?").await;
    w.finish_run(&day, Agent::Yinyue, mark).await;
    assert!(!spoken(&her_line(&w, "Six, of course.").await));
}

/// Use case 1, from an app's moment: her answer shows on the app page in
/// front — text only.
#[tokio::test]
async fn a_moment_answered_on_the_app_page_in_front_is_text_only() {
    let setup = Setup::default().script(Agent::Yinyue, vec![text("Clouds over the harbor.")]);
    let (w, _) = day_world_at(12, setup).await;
    unmute(&w).await;
    let day = the_day(&w).await;
    at_page(&w, Some("dice")).await;
    ask_from_dice(&w, &day).await;
    assert!(!spoken(&her_line(&w, "Clouds over the harbor.").await));
}

/// Use case 2: at the Mac, but Linggen's page is hidden — spoken.
#[tokio::test]
async fn a_line_while_the_page_is_hidden_is_spoken() {
    let setup = Setup::default().script(Agent::Yinyue, vec![text("Clouds over the harbor.")]);
    let (w, _) = day_world_at(12, setup).await;
    unmute(&w).await;
    let day = the_day(&w).await;
    at_page(&w, Some("dice")).await;
    page_hidden(&w).await;
    ask_from_dice(&w, &day).await;
    assert!(spoken(&her_line(&w, "Clouds over the harbor.").await));
}

/// Use case 2, nobody at any screen: still spoken, as before.
#[tokio::test]
async fn a_line_with_nobody_at_a_screen_is_spoken() {
    let setup = Setup::default().script(Agent::Yinyue, vec![text("Clouds over the harbor.")]);
    let (w, _) = day_world_at(12, setup).await;
    unmute(&w).await;
    let day = the_day(&w).await;
    ask_from_dice(&w, &day).await;
    assert!(spoken(&her_line(&w, "Clouds over the harbor.").await));
}

/// Use case 3: on another app's page, where this line shows only folded in
/// the day's conversation — spoken.
#[tokio::test]
async fn a_line_folded_on_another_apps_page_is_spoken() {
    let setup = Setup::default().script(Agent::Yinyue, vec![text("Clouds over the harbor.")]);
    let (w, _) = day_world_at(12, setup).await;
    unmute(&w).await;
    let day = the_day(&w).await;
    at_page(&w, Some("tiles")).await;
    ask_from_dice(&w, &day).await;
    assert!(spoken(&her_line(&w, "Clouds over the harbor.").await));
}

/// Muted, a line is words only wherever the person is.
#[tokio::test]
async fn muted_she_is_words_only_even_unseen() {
    let w = World::start(Setup::default().setting_off().script(Agent::Yinyue, vec![text("Quietly, then.")])).await;
    w.api.post("/api/yinyue/chat", json!({"text": "/mute"})).await;
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "Still there?"})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    assert!(!spoken(&her_line(&w, "Quietly, then.").await));
}
