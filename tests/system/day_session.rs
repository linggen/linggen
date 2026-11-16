//! One session a day (doc/one-ling-spec.md § One session a day): with
//! `agent.one_session_a_day` on, the day's session is found by the day —
//! a day begins at 04:00 local — made on first use and found again after a
//! restart.
//!
//! The engine's clock is the real one; the local hour is set by its time
//! zone (`TZ`), chosen so that it is half past three, or half past four,
//! right now.

use crate::support::{Setup, World};
use serde_json::{json, Value};

/// A POSIX `TZ` whose local time is now `hour:30`, and the local date then.
fn zone_at(hour: i64) -> (String, chrono::NaiveDate) {
    let utc = chrono::Utc::now();
    let utc_minutes = (utc.timestamp() / 60).rem_euclid(1440);
    // POSIX offsets are west of UTC: local = UTC − offset.
    let offset = (utc_minutes - (hour * 60 + 30)).rem_euclid(1440);
    let zone = format!("XXX+{:02}:{:02}", offset / 60, offset % 60);
    let local = utc - chrono::Duration::minutes(offset);
    (zone, local.date_naive())
}

pub(crate) async fn set_agent(w: &World, fields: Value) {
    let mut config = w.api.get("/api/config").await;
    for (key, value) in fields.as_object().unwrap() {
        config["agent"][key] = value.clone();
    }
    w.api.post("/api/config", config).await;
}

/// A world whose local time is `hour:30`, the setting on.
pub(crate) async fn day_world_at(hour: i64, setup: Setup) -> (World, chrono::NaiveDate) {
    let (zone, local_date) = zone_at(hour);
    let w = World::start(Setup {
        env: vec![("TZ", zone)],
        ..setup
    })
    .await;
    set_agent(&w, json!({ "one_session_a_day": true })).await;
    (w, local_date)
}

fn day_id(date: chrono::NaiveDate) -> String {
    format!("sess-day-{}", date.format("%Y-%m-%d"))
}

fn meta(w: &World, sid: &str) -> String {
    let path = w
        .home
        .linggen_home()
        .join("sessions")
        .join(sid)
        .join("session.yaml");
    std::fs::read_to_string(path).unwrap_or_default()
}

#[tokio::test]
async fn before_four_in_the_morning_it_is_still_yesterday_and_a_restart_finds_it() {
    let (mut w, local) = day_world_at(3, Setup::default()).await;
    let yesterday = local.pred_opt().unwrap();
    let day = w.api.get("/api/sessions/day").await;
    assert_eq!(day["on"], true, "{day}");
    assert_eq!(day["id"], day_id(yesterday), "{day}");
    let made = meta(&w, &day_id(yesterday));
    assert!(made.contains("id: ling"), "Ling leads the day: {made}");
    assert!(made.contains("Linggen · "), "{made}");

    w.engine.restart(&w.home).await;
    let again = w.api.get("/api/sessions/day").await;
    assert_eq!(again["id"], day["id"], "the same day after a restart");
    assert_eq!(meta(&w, &day_id(yesterday)), made, "found, not made again");
}

#[tokio::test]
async fn from_four_in_the_morning_it_is_a_new_day() {
    let (w, local) = day_world_at(4, Setup::default()).await;
    let day = w.api.get("/api/sessions/day").await;
    assert_eq!(day["id"], day_id(local), "{day}");
}

#[tokio::test]
async fn with_the_setting_off_there_is_no_day_session() {
    let w = World::start(Setup::default()).await;
    let day = w.api.get("/api/sessions/day").await;
    assert_eq!(day, json!({ "on": false, "id": null }));
    let sessions = w.home.linggen_home().join("sessions");
    let made: Vec<_> = std::fs::read_dir(sessions)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("sess-day-"))
        .collect();
    assert!(made.is_empty(), "no day session is made while off");
}
