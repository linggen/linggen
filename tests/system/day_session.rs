//! One session a day (doc/one-ling-spec.md § One session a day): with
//! `agent.one_session_a_day` on, the day's session is found by the day —
//! a day begins at 04:00 local — made on first use and found again after a
//! restart.
//!
//! The engine's clock is the real one; the local hour is set by its time
//! zone (`TZ`), chosen so that it is half past three, or half past four,
//! right now.

use crate::support::model::Agent;
use crate::support::{text, Setup, World};
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

/// One message from a page: `page` is the app the page belongs to (none:
/// the main page).
pub(crate) async fn say(w: &World, sid: &str, page: Option<&str>, message: &str) -> Value {
    let mut body = json!({"session_id": sid, "project_root": w.work(), "message": message});
    if let Some(app) = page {
        body["skill_name"] = json!(app);
    }
    w.api.post("/api/chat", body).await
}

/// [`say`], waiting for `agent`'s run to finish.
pub(crate) async fn turn_at(w: &World, sid: &str, page: Option<&str>, message: &str, agent: Agent) {
    let mark = w.mark();
    let resp = say(w, sid, page, message).await;
    assert_eq!(resp["agent_id"], agent.id(), "routed elsewhere: {resp}");
    w.finish_run(sid, agent, mark).await;
}

/// The place each person's message in `sid` was kept in, in order.
pub(crate) fn user_places(w: &World, sid: &str) -> Vec<String> {
    w.raw_rows(sid)
        .iter()
        .filter(|r| r["from_id"] == "user" && r["is_observation"] == false)
        .map(|r| r["place"].as_str().unwrap_or("-").to_string())
        .collect()
}

/// The sessions on disk whose creator is `creator`.
fn sessions_made_by(w: &World, creator: &str) -> Vec<String> {
    let dir = w.home.linggen_home().join("sessions");
    let line = format!("creator: {creator}");
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|id| meta(w, id).lines().any(|l| l.trim() == line))
        .collect()
}

fn offers(call: &crate::support::model::Call, tool: &str) -> bool {
    call.tool_names().iter().any(|t| t == tool)
}

#[tokio::test]
async fn an_apps_page_finds_the_day_and_its_one_off_jobs_stay_apart() {
    let (w, _) = day_world_at(12, Setup::default()).await;
    let day = w.api.get("/api/sessions/day").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let dice = w
        .api
        .create_session(json!({"title": "Dice", "skill": "dice"}))
        .await;
    let quiet = w
        .api
        .create_session(json!({"title": "Quiet", "skill": "quiet"}))
        .await;
    assert_eq!(
        (dice.as_str(), quiet.as_str()),
        (day.as_str(), day.as_str())
    );

    let npc = json!({"title": "NPC", "skill": "dice", "withheld_tools": ["mcp__memory"]});
    let npc = w.api.create_session(npc).await;
    let chat = w.api.create_session(json!({"title": "Chat"})).await;
    assert_ne!(npc, day, "an app's one-off job keeps its own session");
    assert_ne!(chat, day, "a New chat stays apart");

    let listed = w.api.get("/api/skill-sessions?skill=dice").await;
    let ids: Vec<&str> = listed["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["id"].as_str())
        .collect();
    assert_eq!(
        ids,
        vec![day.as_str(), npc.as_str()],
        "the day's first: {listed}"
    );
}

/// The first scenario: a day that moves between two apps and the main page,
/// Ling and Yinyue both there — one thread, each turn with its own place's
/// tools and nothing of the other's.
#[tokio::test]
async fn a_day_between_two_apps_and_the_main_page_is_one_thread() {
    let setup = Setup::default()
        .script(
            Agent::Ling,
            vec![
                text("Rolled a four."),
                text("Just talking."),
                text("Home again."),
            ],
        )
        .script(Agent::Yinyue, vec![text("I saw the four.")]);
    let (w, _) = day_world_at(12, setup).await;
    let day = w
        .api
        .create_session(json!({"title": "Dice", "skill": "dice"}))
        .await;

    turn_at(&w, &day, Some("dice"), "Roll for me", Agent::Ling).await;
    turn_at(&w, &day, Some("quiet"), "Just talk", Agent::Ling).await;
    let mark = w.mark();
    w.api
        .post("/api/yinyue/chat", json!({"text": "Did you see?"}))
        .await;
    w.wait_agent(Agent::Yinyue, mark).await;
    turn_at(&w, &day, None, "Back on the main page", Agent::Ling).await;

    let ling = w.model.calls_of(Agent::Ling);
    assert_eq!(ling.len(), 3, "one call a turn");
    assert!(
        offers(&ling[0], "Look") && offers(&ling[0], "Roll"),
        "{:?}",
        ling[0].tool_names()
    );
    for (i, call) in ling.iter().enumerate().skip(1) {
        for dice in ["Look", "Roll", "Peek"] {
            assert!(!offers(call, dice), "turn {i} was offered dice's {dice}");
        }
    }
    assert!(ling[0].text().contains("You run the dice table"));
    assert!(!ling[1].text().contains("You run the dice table"));
    assert!(ling[1].text().contains("You only talk"));
    assert!(!ling[2].text().contains("You only talk"));

    // One thread: she reads the day's talk, and Ling reads hers.
    let hers = w.model.calls_of(Agent::Yinyue);
    assert_eq!(hers.len(), 1);
    assert!(
        hers[0].text().contains("Rolled a four."),
        "her thread holds the dice turn"
    );
    assert!(
        ling[2].text().contains("I saw the four."),
        "Ling's holds her line"
    );

    // Her kickoff is kept as a person's row, at home like her turn.
    assert_eq!(
        user_places(&w, &day),
        vec!["dice", "quiet", ":home", ":home"]
    );
    let seated = meta(&w, &day);
    assert!(
        seated.contains("id: ling") && seated.contains("id: yinyue"),
        "{seated}"
    );
    assert!(
        w.raw_rows(&day).iter().any(|r| r["agent_id"] == "yinyue"),
        "her turn is a row of the day"
    );
}

/// With the setting on, a mission run and a delegated run keep sessions of
/// their own.
#[tokio::test]
async fn a_mission_and_a_delegated_run_stay_outside_the_day() {
    let setup = Setup::default().script(Agent::Ling, vec![text("Last roll: 3."), text("Done.")]);
    let (w, _) = day_world_at(12, setup).await;
    let day = w.api.get("/api/sessions/day").await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let mark = w.mark();
    let fired = w
        .api
        .post("/api/missions/table-check/trigger", json!({}))
        .await;
    let mission = fired["session_id"]
        .as_str()
        .expect("a mission session")
        .to_string();
    w.finish_run(&mission, Agent::Ling, mark).await;
    assert_ne!(mission, day);

    let call = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "agent_run", "arguments": {"prompt": "Say done."}}
    });
    let ran = w.api.post("/mcp", call).await;
    assert_eq!(ran["result"]["content"][0]["text"], "Done.", "{ran}");
    assert!(
        sessions_made_by(&w, "external").len() == 1,
        "a delegated run runs in a session of its own"
    );
    assert!(
        w.raw_rows(&day).is_empty(),
        "nothing of either reached the day's thread"
    );
}

/// What the person told the private app — none of it may reach another place.
const SECRET: &str = "LEDGER-ONLY 120 shares of ACME";
const LEDGER_REPLY: &str = "LEDGER-REPLY: holding steady at 120.";

/// A made-up private app (`private-talk: true`), written into the world's
/// home after start and reloaded, as `missions.rs` installs its beacon.
pub(crate) async fn install_ledger(w: &World) {
    let dir = w.home.linggen_home().join("skills/ledger");
    std::fs::create_dir_all(&dir).expect("ledger dir");
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: ledger\ndescription: A private test ledger.\nallowed-tools: []\n\
         private-talk: true\nmembers: [ling, yinyue]\n---\n\nYou keep the ledger. Answer in one line.\n",
    )
    .expect("SKILL.md");
    w.api.post("/api/skills/reload", json!({})).await;
}

fn holds_secret(text: &str) -> bool {
    text.contains("LEDGER-ONLY") || text.contains("LEDGER-REPLY")
}

fn history_text(v: &Value) -> String {
    v["messages"].to_string()
}

/// The second scenario: a private app's talk never reaches another place's
/// turn, nor her outside it — in the bytes each is sent — nor a page
/// outside it; in its own place it is all there.
#[tokio::test]
async fn a_private_apps_talk_reaches_no_other_place_nor_her() {
    let setup = Setup::default()
        .script(
            Agent::Ling,
            vec![
                text(LEDGER_REPLY),
                text("Rolled a two."),
                text("Home."),
                text("Still 120."),
            ],
        )
        .script(Agent::Yinyue, vec![text("I'm here.")]);
    let (w, _) = day_world_at(12, setup).await;
    install_ledger(&w).await;
    let day = w
        .api
        .create_session(json!({"title": "Ledger", "skill": "ledger"}))
        .await;

    turn_at(&w, &day, Some("ledger"), SECRET, Agent::Ling).await;
    turn_at(&w, &day, Some("dice"), "Roll for me", Agent::Ling).await;
    turn_at(&w, &day, None, "Hello from the main page", Agent::Ling).await;
    let mark = w.mark();
    w.api
        .post("/api/yinyue/chat", json!({"text": "Anything new?"}))
        .await;
    w.wait_agent(Agent::Yinyue, mark).await;
    turn_at(&w, &day, Some("ledger"), "And now?", Agent::Ling).await;

    let ling = w.model.calls_of(Agent::Ling);
    assert_eq!(ling.len(), 4);
    assert!(
        !holds_secret(&ling[1].text()),
        "the dice turn was given the ledger's talk"
    );
    assert!(
        !holds_secret(&ling[2].text()),
        "the main page's turn was given it"
    );
    let hers = w.model.calls_of(Agent::Yinyue);
    assert!(
        !holds_secret(&hers[0].text()),
        "she was given the ledger's talk"
    );
    // Its own place is given all of it, its own reply included.
    assert!(ling[3].text().contains("LEDGER-ONLY") && ling[3].text().contains("LEDGER-REPLY"));
    // The other places' talk is everyone's, the ledger's included.
    assert!(ling[3].text().contains("Rolled a two."));

    // Shown on its own page only.
    let main = w.api.history(&day, &w.work()).await;
    assert!(
        !holds_secret(&history_text(&main)),
        "the main page shows it"
    );
    assert!(history_text(&main).contains("Rolled a two."));
    let page = |app: &str| format!("/api/skill-sessions/state?skill={app}&session_id={day}");
    assert!(!holds_secret(&history_text(
        &w.api.get(&page("dice")).await
    )));
    assert!(holds_secret(&history_text(
        &w.api.get(&page("ledger")).await
    )));
    // Each row of the day names its place, for the page to fold.
    let places: Vec<String> = main["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m[0]["place"].as_str().map(String::from))
        .collect();
    assert!(places.contains(&"dice".to_string()) && !places.contains(&"ledger".to_string()));
}

/// A summary made outside the private app is made without its talk.
#[tokio::test]
async fn a_summary_made_at_home_leaves_the_private_apps_talk_out() {
    let filler: String = (0..900)
        .map(|i| format!("filler-{i:05} the quick brown fox jumps over the lazy dog."))
        .collect::<Vec<_>>()
        .join(" ");
    let setup = Setup::default()
        .script(
            Agent::Ling,
            vec![text(LEDGER_REPLY), text("OK."), text("Fine.")],
        )
        .extra("context compaction assistant", "SUMMARY: filler was sent.");
    let (w, _) = day_world_at(12, setup).await;
    install_ledger(&w).await;
    let day = w
        .api
        .create_session(json!({"title": "Ledger", "skill": "ledger"}))
        .await;
    turn_at(&w, &day, Some("ledger"), SECRET, Agent::Ling).await;
    turn_at(
        &w,
        &day,
        None,
        &format!("Ignore this.\n{filler}"),
        Agent::Ling,
    )
    .await;
    turn_at(&w, &day, None, "Short one.", Agent::Ling).await;

    let resp = w
        .api
        .post(
            "/api/chat/compact",
            json!({"session_id": day, "project_root": w.work()}),
        )
        .await;
    assert_eq!(resp["compacted"], true, "{resp}");
    let asked: Vec<String> = w
        .model
        .calls()
        .iter()
        .map(|c| c.text())
        .filter(|t| t.contains("ENTRIES TO SUMMARIZE"))
        .collect();
    assert_eq!(asked.len(), 1);
    assert!(
        asked[0].contains("filler-00001"),
        "the summary is of the day's talk"
    );
    assert!(
        !holds_secret(&asked[0]),
        "the summarizer was given the ledger's talk"
    );
}

/// With no private app's talk, and the setting off, a session's history and
/// rows are as before: no place is shown on them.
#[tokio::test]
async fn with_the_setting_off_an_apps_session_is_its_own() {
    let w = World::start(Setup::default().script(Agent::Ling, vec![text("Rolled.")])).await;
    let sid = w
        .api
        .create_session(json!({"title": "Dice", "skill": "dice"}))
        .await;
    assert!(!sid.starts_with("sess-day-"), "{sid}");
    turn_at(&w, &sid, Some("dice"), "Roll", Agent::Ling).await;
    let page = format!("/api/skill-sessions/state?skill=dice&session_id={sid}");
    let shown = w.api.get(&page).await;
    assert!(shown["messages"]
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m[0].get("place").is_none()));
}

/// With the setting on there is no app session for her to sit in: an app's
/// moment wakes her in the day's session, standing in that app — its tools
/// for her (`place.yinyue.tools`), her line one row there and nowhere else —
/// even when the moment names an app chat from before.
#[tokio::test]
async fn an_apps_moment_wakes_her_in_the_day_in_that_apps_place() {
    let setup = Setup::default().script(Agent::Yinyue, vec![text("Clouds over the harbor.")]);
    let (w, _) = day_world_at(12, setup).await;
    // A dice chat from before the setting, held by a page that did not move.
    set_agent(&w, json!({ "one_session_a_day": false })).await;
    let old = w
        .api
        .create_session(json!({"title": "table", "skill": "dice"}))
        .await;
    set_agent(&w, json!({ "one_session_a_day": true })).await;
    let day = w.api.get("/api/sessions/day").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(old, day);

    let mark = w.mark();
    let moment = json!({"app": "dice", "text": "The player looks at the sky.", "asked": true, "session": old});
    assert_eq!(w.api.post("/api/yinyue/event", moment).await, json!("ok"));
    w.finish_run(&day, Agent::Yinyue, mark).await;

    let rows = w.raw_rows(&day);
    let hers: Vec<&Value> = rows
        .iter()
        .filter(|r| r["from_id"] == "yinyue" && r["is_observation"] == false)
        .collect();
    assert_eq!(hers.len(), 1, "{rows:?}");
    assert_eq!(hers[0]["content"], "Clouds over the harbor.");
    assert_eq!(hers[0]["place"], "dice");
    assert!(
        w.rows_opt(&old).unwrap_or_default().is_empty(),
        "her line landed in the old chat"
    );
    let call = w.model.calls_of(Agent::Yinyue).pop().expect("her call");
    assert!(
        offers(&call, "Peek"),
        "dice's tool for her: {:?}",
        call.tool_names()
    );
    w.assert_all_scripted();
}
