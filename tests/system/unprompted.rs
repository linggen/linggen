//! What an app tells her at night: quiet hours hold an unasked moment, except
//! one from the app the person is using — that one passes them, and the
//! hourly ceiling still counts and holds it (doc/yinyue-spec.md
//! § Guardrails).
//!
//! The engine runs on UTC (`support::process`), so quiet hours are a window
//! around the clock of the test: now is always inside them.

use crate::notes::{held, wait_for};
use crate::support::model::Agent;
use crate::support::{say, Setup, World};
use serde_json::{json, Value};
use std::time::Duration;

const LINE: &str = "That was a hard one. Breathe.";

/// An hour either side of now, as `[pet] quiet_hours` writes it.
pub(crate) fn quiet_around_now() -> String {
    let now = chrono::Utc::now();
    let edge = |hours: i64| {
        (now + chrono::Duration::hours(hours))
            .format("%H:%M")
            .to_string()
    };
    format!("{}-{}", edge(-1), edge(1))
}

pub(crate) async fn set_pet(w: &World, fields: Value) {
    let mut config = w.api.get("/api/config").await;
    for (key, value) in fields.as_object().unwrap() {
        config["pet"][key] = value.clone();
    }
    w.api.post("/api/config", config).await;
}

/// A page's beat: `app` in front (none: the main page), input `idle_s` ago.
async fn beat(w: &World, app: Option<&str>, idle_s: u64) {
    let mut body = json!({
        "surface": "main-ui", "focused": true, "typing": false, "idle_ms": idle_s * 1000
    });
    if let Some(app) = app {
        body["app"] = json!(app);
    }
    w.api.post("/api/presence", body).await;
}

async fn the_app_tells_her(w: &World) {
    let moment = json!({"app": "dice", "text": "The dice came up snake eyes.", "big": true});
    assert_eq!(w.api.post("/api/yinyue/event", moment).await, json!("ok"));
}

fn spoken(w: &World) -> Vec<String> {
    w.events()
        .into_iter()
        .filter(|e| e["kind"] == "pet_speak")
        .filter_map(|e| e["text"].as_str().map(String::from))
        .collect()
}

async fn night_world() -> World {
    let w = World::start(Setup::default().script(Agent::Yinyue, vec![say(LINE)])).await;
    set_pet(&w, json!({ "quiet_hours": quiet_around_now() })).await;
    w
}

/// Held at night, recorded once, and she is never woken.
async fn assert_held_by(w: &World, reason: &str) {
    wait_for("a hold record", || !held(w).is_empty()).await;
    tokio::time::sleep(Duration::from_secs(8)).await;
    let records = held(w);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        records[0]["object"],
        format!("app moments from dice ({reason})")
    );
    assert_eq!(records[0]["detail"]["source"], "moment");
    assert!(w.model.calls_of(Agent::Yinyue).is_empty(), "she was woken");
    assert!(spoken(w).is_empty(), "{:?}", spoken(w));
}

#[tokio::test]
async fn a_moment_from_the_app_in_use_passes_quiet_hours() {
    let w = night_world().await;
    let mark = w.mark();
    beat(&w, Some("dice"), 10).await;
    the_app_tells_her(&w).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    wait_for("her line", || !spoken(&w).is_empty()).await;
    assert_eq!(spoken(&w), [LINE]);
    assert!(held(&w).is_empty(), "{:?}", held(&w));
    w.assert_all_scripted();
}

#[tokio::test]
async fn a_moment_is_held_by_quiet_hours_when_another_page_is_in_front() {
    let w = night_world().await;
    beat(&w, None, 10).await;
    the_app_tells_her(&w).await;
    assert_held_by(&w, "quiet hours").await;
}

#[tokio::test]
async fn a_moment_is_held_by_quiet_hours_when_the_app_has_had_no_input_for_five_minutes() {
    let w = night_world().await;
    beat(&w, Some("dice"), 301).await;
    the_app_tells_her(&w).await;
    assert_held_by(&w, "quiet hours").await;
}

#[tokio::test]
async fn the_hourly_ceiling_still_holds_a_moment_from_the_app_in_use() {
    let w = night_world().await;
    set_pet(&w, json!({ "unprompted_per_hour": 0 })).await;
    beat(&w, Some("dice"), 10).await;
    the_app_tells_her(&w).await;
    assert_held_by(&w, "hourly ceiling").await;
}
