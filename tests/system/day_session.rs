//! Linggen's daily session (doc/one-ling-spec.md § Linggen's daily session): with
//! `agent.daily_session` on, the day's session is found by the day —
//! a day begins at 04:00 local — made on first use and found again after a
//! restart.
//!
//! The engine's clock is the real one; the local hour is set by its time
//! zone (`TZ`), chosen so that it is half past three, or half past four,
//! right now.

use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
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
    set_agent(&w, json!({ "daily_session": true })).await;
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
    let w = World::start(Setup::default().setting_off()).await;
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
    // The day's first, then the app's other sessions: its one-off job, and
    // the chat it had before the day (the fixture's), which its page still
    // shows. Another app's sessions and a New chat are not this app's.
    assert_eq!(ids.first(), Some(&day.as_str()), "the day's first: {listed}");
    assert_eq!(listed["sessions"][0]["day"], json!(true), "the day's row says so: {listed}");
    assert!(ids.contains(&npc.as_str()), "its one-off job: {listed}");
    assert!(ids.contains(&EARLIER_DICE_CHAT), "its chat from before the day: {listed}");
    assert!(!ids.contains(&chat.as_str()), "a New chat is not the app's: {listed}");
    assert_eq!(ids.len(), 3, "{listed}");
}

/// The fixture's dice chat from before any day (`fixtures/`), also named in
/// `sessions.rs`.
const EARLIER_DICE_CHAT: &str = "sess-1790000001-5a1ed001";

#[tokio::test]
async fn an_apps_page_lists_earlier_chats_below_the_day() {
    let setup = Setup::default().script(Agent::Ling, vec![text("In the day.")]);
    let (w, _) = day_world_at(12, setup).await;
    set_agent(&w, json!({ "daily_session": false })).await;
    let older = w.api.create_session(json!({"title": "Earlier", "skill": "dice"})).await;
    let newer = w.api.create_session(json!({"title": "Later", "skill": "dice"})).await;
    w.api.create_session(json!({"title": "Other app", "skill": "quiet"})).await;
    // Give the fixtures distinct timestamps even when both were made in one second.
    for (id, created) in [(&older, 100), (&newer, 200)] {
        let path = w.home.linggen_home().join("sessions").join(id).join("session.yaml");
        let mut saved: Value = serde_norway::from_str(&meta(&w, id)).unwrap();
        saved["created_at"] = json!(created);
        saved["agents"] = json!([{"id": "ling", "model": "saved-model"}]);
        std::fs::write(path, serde_norway::to_string(&saved).unwrap()).unwrap();
        std::fs::remove_file(w.home.linggen_home().join("sessions").join(id).join("messages.jsonl")).unwrap();
    }

    set_agent(&w, json!({ "daily_session": true })).await;
    let day = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let listed = w.api.get("/api/skill-sessions?skill=dice").await;
    let rows = listed["sessions"].as_array().unwrap();
    let ids: Vec<_> = rows.iter().map(|row| row["id"].as_str().unwrap()).collect();
    // The day's first and marked; then the app's earlier chats — these two
    // and the fixture's — each with its own members and model, so one opened
    // from the page is not given the default model.
    assert_eq!(ids.first(), Some(&day.as_str()), "{listed}");
    assert_eq!(rows[0]["day"], json!(true), "{listed}");
    for earlier in [older.as_str(), newer.as_str(), EARLIER_DICE_CHAT] {
        assert!(ids[1..].contains(&earlier), "{earlier} is listed below the day: {listed}");
    }
    assert_eq!(ids.len(), 4, "another app's chat is not listed here: {listed}");
    for row in rows.iter().filter(|row| [older.as_str(), newer.as_str()].contains(&row["id"].as_str().unwrap())) {
        assert_eq!(row["agents"], json!([{"id": "ling", "model": "saved-model"}]), "{row}");
        assert!(row.get("day").is_none(), "only the day's row is marked: {row}");
    }

    let mark = w.mark();
    let sent = w
        .api
        .post("/api/chat", json!({"project_root": w.work(), "message": "Roll", "skill_name": "dice"}))
        .await;
    assert_eq!(sent["session_id"], day, "{sent}");
    w.finish_run(&day, Agent::Ling, mark).await;
    assert_eq!(user_places(&w, &day), ["dice"]);
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

fn history_text(v: &Value) -> String {
    v["messages"].to_string()
}

/// A made-up app that plans before it acts.
async fn install_planner(w: &World) {
    let dir = w.home.linggen_home().join("skills/planner");
    std::fs::create_dir_all(&dir).expect("planner dir");
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: planner\ndescription: A test planner.\n\
         allowed-tools: [EnterPlanMode, ExitPlanMode]\n---\n\nPlan first.\n",
    )
    .expect("SKILL.md");
    w.api.post("/api/skills/reload", json!({})).await;
}

/// With the setting off, a session's history and rows are as before: no
/// place is shown on them.
#[tokio::test]
async fn with_the_setting_off_an_apps_session_is_its_own() {
    let setup = Setup::default().setting_off().script(Agent::Ling, vec![text("Rolled.")]);
    let w = World::start(setup).await;
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
    set_agent(&w, json!({ "daily_session": false })).await;
    let old = w
        .api
        .create_session(json!({"title": "table", "skill": "dice"}))
        .await;
    set_agent(&w, json!({ "daily_session": true })).await;
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

/// The phone's side: a hand-off finds the day's session the way the phone
/// looks for an app's session (the list's first), runs in that app's place
/// when it names the app (`skill_name`), and the open notes read back as
/// they are, read-only.
#[tokio::test]
async fn a_phone_hand_off_lands_in_the_day_in_its_apps_place() {
    let setup = Setup::default()
        .script(Agent::Ling, vec![text("Rolled a six for you.")])
        .script(
            Agent::Yinyue,
            vec![
                tool("note_write", json!({"text": "ask how the game went"})),
                text("Noted."),
            ],
        );
    let (w, _) = day_world_at(12, setup).await;
    let day = w.api.get("/api/sessions/day").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let listed = w.api.get("/api/skill-sessions?skill=dice").await;
    assert_eq!(listed["sessions"][0]["id"], day.as_str(), "{listed}");

    let mark = w.mark();
    let body = json!({
        "session_id": day, "project_root": w.work(), "message": "Roll a die for me",
        "skill_name": "dice", "via": "iphone", "via_note": "Let me roll it on the Mac."
    });
    let resp = w.api.post("/api/chat", body).await;
    assert_eq!(resp["agent_id"], "ling", "{resp}");
    w.finish_run(&day, Agent::Ling, mark).await;
    let call = w.model.calls_of(Agent::Ling).pop().unwrap();
    assert!(
        offers(&call, "Roll"),
        "the hand-off stands in dice: {:?}",
        call.tool_names()
    );
    let handed: Vec<Value> = w
        .raw_rows(&day)
        .into_iter()
        .filter(|r| r["via"] == "iphone")
        .collect();
    assert_eq!(handed.len(), 1);
    assert_eq!(handed[0]["place"], "dice");

    let mark = w.mark();
    w.api
        .post(
            "/api/yinyue/chat",
            json!({"text": "Remember to ask me about the game."}),
        )
        .await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let notes = w.api.get("/api/notes?agent=yinyue").await;
    assert_eq!(
        notes["notes"][0]["text"], "ask how the game went",
        "{notes}"
    );
    assert_eq!(w.api.get("/api/notes?agent=../x").await["notes"], json!([]));
}

/// The third scenario: late at night, before 04:00, it is still the day
/// that began the morning before; a restart in the middle of it changes
/// nothing — the page finds the same session and the thread goes on.
#[tokio::test]
async fn a_restart_late_at_night_keeps_the_days_thread() {
    let setup = Setup::default().script(
        Agent::Ling,
        vec![
            text("Rolled a five before midnight."),
            text("Still the same night."),
        ],
    );
    let (mut w, local) = day_world_at(3, setup).await;
    let day = w
        .api
        .create_session(json!({"title": "Dice", "skill": "dice"}))
        .await;
    assert_eq!(
        day,
        day_id(local.pred_opt().unwrap()),
        "half past three is yesterday's day"
    );
    turn_at(&w, &day, Some("dice"), "Roll once more", Agent::Ling).await;

    w.engine.restart(&w.home).await;
    let again = w
        .api
        .create_session(json!({"title": "Dice", "skill": "dice"}))
        .await;
    assert_eq!(again, day, "the same day after the restart");
    turn_at(&w, &again, Some("dice"), "Are we still on?", Agent::Ling).await;
    let after = w.model.calls_of(Agent::Ling).pop().unwrap();
    assert!(
        after.text().contains("Rolled a five before midnight."),
        "the thread goes on"
    );
    assert_eq!(user_places(&w, &day), vec!["dice", "dice"]);
}

/// Switching it off goes back to a session per app, and loses nothing: the
/// day's session still reads, the app's own chat is its own; on again, the
/// same day is found and the app's chat is still there.
#[tokio::test]
async fn switching_off_and_on_again_loses_no_session() {
    let setup =
        Setup::default().script(Agent::Ling, vec![text("In the day."), text("On its own.")]);
    let (w, _) = day_world_at(12, setup).await;
    let day = w
        .api
        .create_session(json!({"title": "Dice", "skill": "dice"}))
        .await;
    turn_at(&w, &day, Some("dice"), "Roll", Agent::Ling).await;

    set_agent(&w, json!({ "daily_session": false })).await;
    let own = w
        .api
        .create_session(json!({"title": "Dice", "skill": "dice"}))
        .await;
    assert_ne!(own, day, "off: the app makes its own session");
    let listed = w
        .api
        .get("/api/skill-sessions?skill=dice")
        .await
        .to_string();
    assert!(
        !listed.contains(&day),
        "off: the app's list has no day: {listed}"
    );
    turn_at(&w, &own, Some("dice"), "Roll again", Agent::Ling).await;
    assert_eq!(w.api.get("/api/sessions/day").await["on"], false);
    let still = w.api.history(&day, &w.work()).await;
    assert!(
        history_text(&still).contains("In the day."),
        "the day still reads"
    );
    // A page cannot remove the day's session as if it were its own.
    let refused = w
        .api
        .delete_status(
            "/api/skill-sessions",
            json!({"skill": "dice", "session_id": day}),
        )
        .await;
    assert_eq!(refused, 403);
    assert!(w.rows_opt(&day).is_some());

    set_agent(&w, json!({ "daily_session": true })).await;
    let again = w
        .api
        .create_session(json!({"title": "Dice", "skill": "dice"}))
        .await;
    assert_eq!(again, day, "on again: the same day");
    assert!(
        w.raw_rows(&own)
            .iter()
            .any(|r| r["content"] == "On its own."),
        "the app's chat stays"
    );
}

/// A made-up app whose agent runs shell commands in its own folder.
async fn install_worker(w: &World) {
    let dir = w.home.linggen_home().join("skills/worker");
    std::fs::create_dir_all(&dir).expect("worker dir");
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: worker\ndescription: A test worker.\nallowed-tools: [Bash]\n\
         cwd: ~/.linggen/skills/worker\n---\n\nRun what you are asked.\n",
    )
    .expect("SKILL.md");
    w.api.post("/api/skills/reload", json!({})).await;
}

/// Answer every question the worker's page is shown until Ling has made
/// `n` calls.
async fn calls_granting_on_its_page(w: &World, n: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while w.model.calls_of(Agent::Ling).len() < n {
        assert!(
            std::time::Instant::now() < deadline,
            "only {} calls",
            w.model.calls_of(Agent::Ling).len()
        );
        let asks = w.api.get("/api/pending-ask-user?skill=worker").await;
        for ask in asks.as_array().cloned().unwrap_or_default() {
            w.api.answer(&ask, "grant:all").await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// An app's background command ends after its turn: its end — the
/// output's tail — is told in that app, a row of its place. Every row of the
/// day names its place.
#[tokio::test]
async fn an_apps_task_end_is_told_in_that_app() {
    let setup = Setup::default().script(
        Agent::Ling,
        vec![
            tool(
                "Bash",
                json!({"cmd": "sleep 1; echo WORKER-OUT-7", "run_in_background": true}),
            ),
            text("Started it."),
            text("It finished."),
        ],
    );
    let (w, _) = day_world_at(12, setup).await;
    install_worker(&w).await;
    let day = w
        .api
        .create_session(json!({"title": "Worker", "skill": "worker"}))
        .await;
    say(&w, &day, Some("worker"), "Run it in the background").await;
    calls_granting_on_its_page(&w, 3).await;
    let told = w.model.calls_of(Agent::Ling)[2].text();
    assert!(
        told.contains("WORKER-OUT-7"),
        "the end is told with its output: {told}"
    );
    let rows = w.raw_rows(&day);
    let notice = rows
        .iter()
        .find(|r| {
            r["from_id"] == "system"
                && r["content"]
                    .as_str()
                    .is_some_and(|c| c.contains("WORKER-OUT-7"))
        })
        .expect("the end notice row");
    assert_eq!(notice["place"], "worker");
    let unplaced: Vec<&Value> = rows.iter().filter(|r| r.get("place").is_none()).collect();
    assert!(
        unplaced.is_empty(),
        "rows of the day with no place: {unplaced:?}"
    );
}

/// A made-up app written into the world's home and loaded: `private` keeps
/// its talk to itself.
pub(crate) async fn install_app(w: &World, name: &str, tools: &str, private: bool) {
    let dir = w.home.linggen_home().join(format!("skills/{name}"));
    std::fs::create_dir_all(&dir).expect("app dir");
    let body = format!(
        "---\nname: {name}\ndescription: A test app.\nallowed-tools: [{tools}]\n\
         private-talk: {private}\ncwd: ~/.linggen/skills/{name}\n---\n\nDo what you are asked.\n"
    );
    std::fs::write(dir.join("SKILL.md"), body).expect("SKILL.md");
    w.api.post("/api/skills/reload", json!({})).await;
}

/// Each person has a day of their own: neither is handed, shown or lets
/// write into the other's — by the day's lookup, an app's list, a chat that
/// names the other's day, or a page's history.
#[tokio::test]
async fn each_person_sees_and_writes_only_their_own_day() {
    let setup = Setup::default().script(
        Agent::Ling,
        vec![text("For the builder."), text("For the friend."), text("Friend again.")],
    );
    let (w, _) = day_world_at(12, setup).await;
    let friend = "u-friend";
    let mine = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let theirs = w.api.get(&format!("/api/sessions/day?user_id={friend}")).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(mine, theirs);

    let chat = |message: &str, user: Option<&str>, sid: Option<&str>| {
        let mut body = json!({"project_root": w.work(), "message": message, "skill_name": "dice"});
        if let Some(user) = user {
            body["user_id"] = json!(user);
        }
        if let Some(sid) = sid {
            body["session_id"] = json!(sid);
        }
        body
    };
    let mark = w.mark();
    let sent = w.api.post("/api/chat", chat("BUILDER-WORDS", None, None)).await;
    assert_eq!(sent["session_id"], mine.as_str());
    w.finish_run(&mine, Agent::Ling, mark).await;
    let mark = w.mark();
    let sent = w.api.post("/api/chat", chat("FRIEND-WORDS", Some(friend), None)).await;
    assert_eq!(sent["session_id"], theirs.as_str());
    w.finish_run(&theirs, Agent::Ling, mark).await;
    // Naming the builder's day, the friend writes into their own.
    let mark = w.mark();
    let sent = w.api.post("/api/chat", chat("FRIEND-AGAIN", Some(friend), Some(&mine))).await;
    assert_eq!(sent["session_id"], theirs.as_str(), "{sent}");
    w.finish_run(&theirs, Agent::Ling, mark).await;

    let text_of = |sid: &str| serde_json::to_string(&w.raw_rows(sid)).unwrap();
    assert!(!text_of(&mine).contains("FRIEND-"), "the friend wrote into the builder's day");
    assert!(!text_of(&theirs).contains("BUILDER-WORDS"));
    let builders_save = w.api.create_session(json!({"title": "Builder's save", "skill": "dice", "withheld_tools": ["Bash"]})).await;
    let list = w.api.get(&format!("/api/skill-sessions?skill=dice&user_id={friend}")).await;
    assert!(list["sessions"].as_array().unwrap().iter().all(|row| row["id"] != builders_save.as_str()));
    assert_eq!(list["sessions"][0]["id"], theirs.as_str());
    assert!(list["sessions"].as_array().unwrap().iter().all(|row| row["id"] != mine.as_str()));
    let page = format!("/api/skill-sessions/state?skill=dice&session_id={mine}&user_id={friend}");
    assert!(!history_text(&w.api.get(&page).await).contains("BUILDER-WORDS"));
    let main = format!(
        "/api/workspace/state?session_id={mine}&project_root={}&user_id={friend}",
        urlencoding::encode(&w.work())
    );
    assert!(!history_text(&w.api.get(&main).await).contains("BUILDER-WORDS"));
    let own = format!("/api/skill-sessions/state?skill=dice&session_id={theirs}&user_id={friend}");
    assert!(history_text(&w.api.get(&own).await).contains("FRIEND-WORDS"));
}

/// Answer the permission prompts `app`'s page is shown until Ling has made
/// `n` calls.
pub(crate) async fn grant_until(w: &World, app: &str, n: usize) {
    grant_until_as(w, app, n, None).await
}

/// [`grant_until`], answered as the person `user` (`None`: the builder) —
/// a day's questions are its own person's.
pub(crate) async fn grant_until_as(w: &World, app: &str, n: usize, user: Option<&str>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let asker = user.map(|u| format!("&user_id={u}")).unwrap_or_default();
    while w.model.calls_of(Agent::Ling).len() < n {
        assert!(std::time::Instant::now() < deadline, "only {} calls", w.model.calls_of(Agent::Ling).len());
        let asks = w.api.get(&format!("/api/pending-ask-user?skill={app}{asker}")).await;
        for ask in asks.as_array().cloned().unwrap_or_default() {
            w.api.answer_as(&ask, "grant:all", user).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// A question asked while the person is away, in `app`'s chat — its place
/// of the day, or a private app's own session: she calls them back before
/// it is answered, and what she was told is returned. The turn that asks it
/// holds its session while it waits.
async fn herald_while_away(app: &str, private: bool, question: &str) -> String {
    let setup = Setup::default()
        .script(
            Agent::Ling,
            vec![
                tool("Bash", json!({"cmd": "sleep 3; echo moved", "run_in_background": true})),
                text("Started."),
                tool(
                    "AskUser",
                    json!({"questions": [{"question": question, "header": "Box",
                        "options": [{"label": "Red"}, {"label": "Blue"}]}]}),
                ),
                text("Red it is."),
            ],
        )
        .script(Agent::Yinyue, vec![text("I'm here."), text("They need you back.")]);
    let (w, _) = day_world_at(12, setup).await;
    install_app(&w, app, "Bash, AskUser", private).await;
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "Hello"})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let day = w.api.create_session(json!({"title": "App", "skill": app})).await;
    say(&w, &day, Some(app), "Move the box").await;
    grant_until(&w, app, 2).await;
    // Away: the only surface that beat is the turn's own.
    let away = json!({"surface": "user-turn", "focused": false, "typing": false, "idle_ms": 600_000});
    w.api.post("/api/presence", away).await;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let asked = loop {
        assert!(std::time::Instant::now() < deadline, "no question; log:\n{}", w.engine.log());
        let asks = w.api.get(&format!("/api/pending-ask-user?skill={app}")).await;
        if let Some(ask) = asks.as_array().and_then(|a| a.first()).cloned() {
            break ask;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    while w.model.calls_of(Agent::Yinyue).len() < 2 {
        assert!(std::time::Instant::now() < deadline, "she never heralded; log:\n{}", w.engine.log());
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let told = w.model.calls_of(Agent::Yinyue)[1].text();
    let spoke = w.events_when(|e| e.iter().any(|x| x["kind"] == "pet_speak")).await;
    assert!(spoke.iter().any(|x| x["kind"] == "pet_speak"), "her line was not spoken");
    w.api.answer(&asked, "Red").await;
    told
}

/// The person is away while a turn waits on its question: she calls them
/// back before it is answered, told what is asked.
#[tokio::test]
async fn she_calls_the_person_back_to_a_waiting_question_in_the_day() {
    let told = herald_while_away("porter", false, "PORTER-QUESTION: which box?").await;
    assert!(told.contains("PORTER-QUESTION"), "{told}");
}

/// In a private app she still calls them back, told only that the app waits.
#[tokio::test]
async fn from_a_private_app_she_is_told_only_that_it_waits() {
    let told = herald_while_away("vault", true, "VAULT-QUESTION: which box?").await;
    assert!(!told.contains("VAULT-QUESTION"), "she was told the question: {told}");
    assert!(told.contains("vault"), "{told}");
}

/// From a private app a message to another agent is refused, with a reason
/// the model reads; nothing reaches her.
#[tokio::test]
async fn a_private_apps_message_to_another_agent_is_refused() {
    let setup = Setup::default().script(
        Agent::Ling,
        vec![
            tool("agent_chat", json!({"to": "yinyue", "message": "LEDGER-ONLY news"})),
            text("Kept it here."),
        ],
    );
    let (w, _) = day_world_at(12, setup).await;
    install_app(&w, "quiet", "agent_chat", true).await;
    let day = w.api.create_session(json!({"title": "Quiet", "skill": "quiet"})).await;
    turn_at(&w, &day, Some("quiet"), "Tell her", Agent::Ling).await;
    let after = w.model.calls_of(Agent::Ling)[1].text();
    assert!(after.contains("refused: quiet keeps what is said in it to itself"), "{after}");
    assert!(w.model.calls_of(Agent::Yinyue).is_empty(), "she was reached");
}

/// With plans waiting in two places, approving runs the plan being decided
/// in its own place: the home plan's rows and its run are home's, the app's
/// plan row its app's.
#[tokio::test]
async fn an_approved_plan_runs_in_its_own_place() {
    let setup = Setup::default().script(
        Agent::Ling,
        vec![
            tool("EnterPlanMode", json!({"reason": "plan it"})),
            tool("ExitPlanMode", json!({"plan_text": "## Plan\n\n1. PLANNER-SECRET step"})),
            tool("EnterPlanMode", json!({"reason": "plan it"})),
            tool("ExitPlanMode", json!({"plan_text": "## Plan\n\n1. HOME-PLAN step"})),
            text("HOME-RUN done."),
        ],
    );
    let (w, _) = day_world_at(12, setup).await;
    install_planner(&w).await;
    let day = w.api.create_session(json!({"title": "Planner", "skill": "planner"})).await;
    let has = |w: &World, marker: &str| {
        w.raw_rows(&day).iter().any(|r| r["content"].as_str().is_some_and(|c| c.contains(marker)))
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    say(&w, &day, Some("planner"), "Plan it").await;
    while !has(&w, "PLANNER-SECRET") {
        assert!(std::time::Instant::now() < deadline, "log:\n{}", w.engine.log());
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    say(&w, &day, None, "Plan at home").await;
    while !has(&w, "HOME-PLAN") {
        assert!(std::time::Instant::now() < deadline, "log:\n{}", w.engine.log());
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let action = json!({"project_root": w.work(), "agent_id": "ling", "session_id": day});
    w.api.post("/api/plan/approve", action).await;
    while !has(&w, "HOME-RUN") {
        assert!(std::time::Instant::now() < deadline, "log:\n{}", w.engine.log());
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let rows = w.raw_rows(&day);
    let with = |marker: &str| -> Vec<&Value> {
        rows.iter().filter(|r| r["content"].as_str().is_some_and(|c| c.contains(marker))).collect()
    };
    assert!(with("PLANNER-SECRET").iter().all(|r| r["place"] == "planner"), "{rows:#?}");
    let home = with("HOME-PLAN");
    assert!(home.len() >= 2, "made and approved: {rows:#?}");
    assert!(home.iter().all(|r| r["place"] == ":home"), "the home plan stood in the app: {rows:#?}");
    assert!(with("HOME-RUN").iter().all(|r| r["place"] == ":home"), "it ran in the app: {rows:#?}");
    let main = w.api.history(&day, &w.work()).await;
    assert!(history_text(&main).contains("HOME-RUN"));
}

/// Every MCP tool that takes a session id refuses a day's — the builder's
/// and another person's alike: a host neither reads, continues, stops,
/// answers for nor closes a day. The table is the front door's own list.
#[tokio::test]
async fn every_mcp_tool_taking_a_session_refuses_a_day() {
    let (w, _) = day_world_at(12, Setup::default()).await;
    let mine = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let theirs = w.api.get("/api/sessions/day?user_id=u-friend").await["id"].as_str().unwrap().to_string();
    assert!(theirs.contains("-p-"), "{theirs}");
    let list = w.api.post("/mcp", json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).await;
    let takes = calls_taking_a_session(&list);
    assert!(takes.len() >= 5, "the calls taking a session: {takes:?}");
    let mut wrong = Vec::new();
    for day in [&mine, &theirs] {
        for (name, op, fields) in &takes {
            let mut args = json!({});
            for field in fields {
                args[field.as_str()] = match field.as_str() {
                    "session_id" => json!(day),
                    "prompt" => json!("DAY-PROMPT"),
                    "request_id" => json!("r1"),
                    "decision" => json!("deny"),
                    other => panic!("no value for {other}"),
                };
            }
            if let Some(op) = op {
                args["op"] = json!(op);
            }
            let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": {"name": name, "arguments": args}});
            let res = w.api.post("/mcp", call).await;
            let text = res["result"]["content"][0]["text"].as_str().unwrap_or_default();
            if res["result"]["isError"] != json!(true) || !text.contains("is the day's session") {
                wrong.push(format!("{name} {op:?} on {day}: {res}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    assert!(w.raw_rows(&mine).is_empty() && w.raw_rows(&theirs).is_empty(), "a day was written");
    assert!(w.model.calls().is_empty(), "a model was called");
}

/// Every call of the door's list that takes a session — each tool, and
/// for a tool with ops each op whose form names `session_id` — with the
/// fields it needs (its required ones and the session, read from the
/// published form).
fn calls_taking_a_session(list: &Value) -> Vec<(String, Option<String>, Vec<String>)> {
    let mut calls = Vec::new();
    for tool in list["result"]["tools"].as_array().expect("the tool list") {
        let name = tool["name"].as_str().unwrap().to_string();
        let schema = &tool["inputSchema"];
        if schema["properties"].get("session_id").is_none() {
            continue;
        }
        let Some(index) = schema["properties"]["op"]["description"].as_str() else {
            let required = schema["required"].as_array().cloned().unwrap_or_default();
            let mut fields: Vec<String> = required.iter().filter_map(|f| f.as_str().map(str::to_string)).collect();
            if !fields.iter().any(|f| f == "session_id") {
                fields.push("session_id".into());
            }
            calls.push((name, None, fields));
            continue;
        };
        for form in index.split(" · ") {
            let Some((op, fields)) = form.split_once(": ") else { continue };
            let fields: Vec<String> = fields
                .split(", ")
                .filter(|f| !f.ends_with('?') && *f != "no fields")
                .map(str::to_string)
                .collect();
            if fields.iter().any(|f| f == "session_id") {
                calls.push((name.clone(), Some(op.to_string()), fields));
            }
        }
    }
    calls
}

// What an app's own session gives a person, held in the day: each test below
// is the twin of one with the setting off (named in its comment), its turn
// sent as an app's page sends it — naming its app (`skill_name`).

/// The `- Workspace:` line of a call's system prompt.
fn workspace_line(call: &crate::support::model::Call) -> String {
    let body = &call.body;
    let system = body["messages"][0]["content"].as_str().unwrap_or_default();
    system
        .lines()
        .find(|l| l.starts_with("- Workspace: "))
        .unwrap_or_else(|| panic!("no Workspace line: {system}"))
        .to_string()
}

/// A turn standing in an app works in that app's folder — its declared
/// `cwd:` (dice), else the skill's own folder (quiet) — and a home turn after
/// them in home's (`workspace::a_skill_session_keeps_its_declared_cwd`,
/// `…_with_no_declared_cwd_starts_in_the_skills_folder`).
#[tokio::test]
async fn a_turn_in_an_app_works_in_that_apps_folder() {
    let setup = Setup::default().script(
        Agent::Ling,
        vec![text("Rolled."), text("Talked."), text("Home.")],
    );
    let (w, _) = day_world_at(12, setup).await;
    let day = w.api.create_session(json!({"title": "Dice", "skill": "dice"})).await;
    turn_at(&w, &day, Some("dice"), "Roll", Agent::Ling).await;
    turn_at(&w, &day, Some("quiet"), "Talk", Agent::Ling).await;
    turn_at(&w, &day, None, "Back home", Agent::Ling).await;
    let calls = w.model.calls_of(Agent::Ling);
    let home = w.home.linggen_home();
    let want = [home.join("skills/dice"), home.join("skills/quiet"), home.join("workspace")];
    for (call, folder) in calls.iter().zip(want) {
        assert_eq!(workspace_line(call), format!("- Workspace: {}", folder.display()));
    }
}

/// A `!` command typed at an app's page in the day starts in that app's
/// folder, as it does in the app's own session — the page names its app as
/// a chat does (`workspace::a_skill_session_keeps_its_declared_cwd`).
#[tokio::test]
async fn a_bang_command_at_an_apps_page_in_the_day_starts_in_its_folder() {
    let (w, _) = day_world_at(12, Setup::default()).await;
    let day = w.api.create_session(json!({"title": "Dice", "skill": "dice"})).await;
    let body = json!({"session_id": day, "project_root": w.work(), "command": "pwd", "skill_name": "dice"});
    let ran = w.api.post("/api/bash", body).await;
    let want = w.home.linggen_home().join("skills/dice");
    assert_eq!(ran["stdout"].as_str().unwrap_or_default().trim(), want.display().to_string(), "{ran}");
}

/// The `pwd` of a `!` command sent to session `sid` from the page of
/// `skill` (`None`: a page of no app).
async fn bang_pwd(w: &World, sid: &str, skill: Option<&str>, command: &str) -> String {
    let mut body = json!({"session_id": sid, "project_root": w.work(), "command": command});
    if let Some(skill) = skill {
        body["skill_name"] = json!(skill);
    }
    let ran = w.api.post("/api/bash", body).await;
    ran["stdout"].as_str().unwrap_or_default().trim().to_string()
}

/// A `cd` at an app's page in the day stays that place's: home's next `!`
/// starts where home's last one ended, and the day's own folder is not
/// moved to the app's.
#[tokio::test]
async fn a_bang_cd_at_an_apps_page_never_moves_homes() {
    let (w, _) = day_world_at(12, Setup::default()).await;
    let day = w.api.create_session(json!({"title": "Dice", "skill": "dice"})).await;
    let home = bang_pwd(&w, &day, None, "pwd").await;
    let cwd = |w: &World| meta(w, &day).lines().find(|l| l.starts_with("cwd:")).map(str::to_string);
    let before = cwd(&w);
    let tmp = w.home.linggen_home().join("skills");
    let moved = bang_pwd(&w, &day, Some("dice"), &format!("cd {} && pwd", tmp.display())).await;
    assert_eq!(moved, tmp.display().to_string());
    assert_eq!(bang_pwd(&w, &day, None, "pwd").await, home, "home's `!` moved");
    assert_eq!(bang_pwd(&w, &day, Some("dice"), "pwd").await, moved, "the app's `cd` was lost");
    assert!(before.is_some(), "no cwd line: {}", meta(&w, &day));
    assert_eq!(cwd(&w), before, "the day's folder moved: {}", meta(&w, &day));
}

/// A private app is never a place in the day: a `!` sent from its page
/// with the day's id stands at home's workspace, never in the app's folder.
#[tokio::test]
async fn a_bang_command_naming_a_private_app_in_the_day_gets_no_folder_of_its() {
    let (w, _) = day_world_at(12, Setup::default()).await;
    install_ledger(&w).await;
    let day = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let ran = bang_pwd(&w, &day, Some("ledger"), "pwd").await;
    assert_ne!(ran, w.home.linggen_home().join("skills/ledger").display().to_string());
    assert_eq!(ran, w.home.linggen_home().join("workspace").display().to_string());
}

/// A skill's `tier: edit` tool, from its page in the day, runs under the
/// grant the skill declares on its own folder — no prompt — and writes
/// there (`skills::skill_edit_tool_runs_under_its_own_grant`).
#[tokio::test]
async fn an_apps_edit_tool_runs_under_its_own_grant_in_the_day() {
    let setup = Setup::default().script(
        Agent::Ling,
        vec![tool("Roll", json!({"faces": 9})), text("Five.")],
    );
    let (w, _) = day_world_at(12, setup).await;
    let day = w.api.create_session(json!({"title": "table", "skill": "dice"})).await;
    turn_at(&w, &day, Some("dice"), "Roll a nine-sided die.", Agent::Ling).await;
    assert_eq!(crate::support::rows::calls_by(&w.rows(&day), "ling"), ["Roll"]);
    let table = std::fs::read_to_string(w.home.linggen_home().join("skills/dice/data/table.json"))
        .expect("table");
    assert!(
        table.contains(r#""last_roll":{"faces":9,"value":5}"#),
        "Roll did not write the table: {table}"
    );
    w.assert_all_scripted();
}

/// A zero-tool app (`allowed-tools: []`), from its page in the day, offers
/// only the agent's own notes (`skills::zero_tool_skill_offers_no_tools`).
#[tokio::test]
async fn a_zero_tool_app_offers_no_tools_in_the_day() {
    let setup = Setup::default().script(Agent::Ling, vec![text("Hello.")]);
    let (w, _) = day_world_at(12, setup).await;
    let day = w.api.create_session(json!({"title": "quiet", "skill": "quiet"})).await;
    turn_at(&w, &day, Some("quiet"), "Hi.", Agent::Ling).await;
    let mut offered = w.model.calls_of(Agent::Ling).pop().expect("his call").tool_names();
    offered.sort();
    assert_eq!(offered, ["note_done", "note_write"], "the call offered {offered:?}");
    w.assert_all_scripted();
}

/// Not signed in, on a machine the picture lane refuses: a turn at an app
/// whose tools never had GenerateImage lists nothing as not available
/// (`prompt::a_tool_this_machine_cannot_run_is_named_with_what_it_needs`).
#[tokio::test]
async fn at_an_app_in_the_day_a_tool_it_never_had_is_not_listed() {
    let setup = Setup {
        signed_out: true,
        machine_memory_gb: Some(8),
        ..Setup::default()
    }
    .script(Agent::Ling, vec![text("Rolled.")]);
    let (w, _) = day_world_at(12, setup).await;
    let day = w.api.create_session(json!({"title": "table", "skill": "dice"})).await;
    turn_at(&w, &day, Some("dice"), "Roll", Agent::Ling).await;
    let call = w.model.calls_of(Agent::Ling).pop().expect("his call");
    assert!(!call.text().contains("# Not available here"), "a tool it never had: {}", call.text());
}

/// At an app's place in the day each member is handed what the app's own
/// table gave them: Ling the skill's tools and not her pet tool; Yinyue,
/// called by name from the page, her place's tool only, reading his tool
/// call as labelled facts and never his guide — and she is seated
/// (`shared_table::a_shared_table_she_answers_from_his_look`,
/// `prompt::export_per_member_at_a_skill_table`).
#[tokio::test]
async fn at_an_apps_place_in_the_day_she_answers_from_his_look() {
    let setup = Setup::default()
        .script(
            Agent::Ling,
            vec![tool("Look", json!({})), text("Harbor Table."), text("Mm.")],
        )
        .script(Agent::Yinyue, vec![text("Alex.")]);
    let (w, _) = day_world_at(12, setup).await;
    let day = w.api.create_session(json!({"title": "table", "skill": "dice"})).await;
    turn_at(&w, &day, Some("dice"), "Look at the table.", Agent::Ling).await;
    let his = w.model.calls_of(Agent::Ling).remove(0);
    assert!(offers(&his, "Look") && offers(&his, "Roll"), "{:?}", his.tool_names());
    assert!(!offers(&his, "Peek"), "Ling is offered her pet tool: {:?}", his.tool_names());

    let before = w.rows(&day).len();
    turn_at(&w, &day, Some("dice"), "@银月 what is the player called?", Agent::Yinyue).await;
    let rows = w.rows(&day);
    let hers = &rows[before..];
    assert!(crate::support::rows::calls_by(hers, "yinyue").is_empty(), "{hers:?}");
    assert_eq!(crate::support::rows::last_said(hers, "yinyue"), Some("Alex."));
    let read = w.model.calls_of(Agent::Yinyue).pop().expect("her call");
    let seen = read.text();
    assert!(seen.contains("Ling used Look"), "his call is not labelled text for her");
    assert!(seen.contains("Harbor Table") && seen.contains("Alex"), "the declared facts are missing");
    assert!(!seen.contains("Never mention this guide"), "she read Ling's guide");
    assert!(offers(&read, "Peek"), "her place's tool is missing: {:?}", read.tool_names());
    for not in ["Look", "Roll", "AskUser"] {
        assert!(!offers(&read, not), "she is offered {not}: {:?}", read.tool_names());
    }
    let seated = meta(&w, &day);
    assert!(seated.contains("id: yinyue"), "{seated}");

    turn_at(&w, &day, Some("dice"), "Thanks.", Agent::Ling).await;
    w.assert_all_scripted();
}

/// An app page's prompt export (Copy system prompt), in the day, shows the
/// prompt and tools of the app's place — the page names its app as a chat
/// does (`prompt::export_per_member_at_a_skill_table`).
#[tokio::test]
async fn an_apps_prompt_export_in_the_day_is_its_places() {
    let (w, _) = day_world_at(12, Setup::default()).await;
    let day = w.api.create_session(json!({"title": "table", "skill": "dice"})).await;
    let q = format!("/api/chat/system-prompt?project_root=&agent_id=ling&session_id={day}&skill_name=dice");
    let e = w.api.get(&q).await;
    let names: Vec<&str> = e["tools"]
        .as_array()
        .map(|all| all.iter().filter_map(|t| t["function"]["name"].as_str().or(t["name"].as_str())).collect())
        .unwrap_or_default();
    assert!(names.contains(&"Look") && names.contains(&"Roll"), "{names:?}");
    let prompt = e["system_prompt"].as_str().unwrap_or("");
    assert!(prompt.contains("You run the dice table"), "{prompt}");
}

/// A private app is never a place in the day: the export asked from its
/// page with the day's id shows none of its prompt or tools.
#[tokio::test]
async fn a_private_apps_prompt_export_in_the_day_shows_none_of_it() {
    let (w, _) = day_world_at(12, Setup::default()).await;
    install_ledger(&w).await;
    let day = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let q = format!("/api/chat/system-prompt?project_root=&agent_id=ling&session_id={day}&skill_name=ledger");
    let e = w.api.get(&q).await;
    let prompt = e["system_prompt"].as_str().unwrap_or("");
    assert!(!prompt.is_empty(), "{e}");
    assert!(!prompt.contains("You keep the ledger"), "{prompt}");
}

/// Her moment's hidden kickoff in the day is hers alone: no other member
/// holds it (`shared_table::f_app_moment_lands_in_the_named_session`; where
/// it lands: `an_apps_moment_wakes_her_in_the_day_in_that_apps_place`).
#[tokio::test]
async fn her_moments_hidden_kickoff_is_hers_alone_in_the_day() {
    let setup = Setup::default().script(Agent::Yinyue, vec![text("Clouds over the harbor.")]);
    let (w, _) = day_world_at(12, setup).await;
    let day = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let mark = w.mark();
    let moment = json!({"app": "dice", "text": "The player looks at the sky.", "asked": true, "session": day});
    assert_eq!(w.api.post("/api/yinyue/event", moment).await, json!("ok"));
    w.finish_run(&day, Agent::Yinyue, mark).await;
    let rows = w.rows(&day);
    assert_eq!(crate::support::rows::last_said(&rows, "yinyue"), Some("Clouds over the harbor."));
    let hidden: Vec<_> = rows.iter().filter(|r| r.content.contains("[HIDDEN]")).collect();
    assert!(!hidden.is_empty(), "no hidden kickoff row");
    assert!(hidden.iter().all(|r| r.agent_id == "yinyue"), "{hidden:?}");
    w.assert_all_scripted();
}

/// Her own talk in the day reads the room as her own thread does: the
/// Right now block, the most present surface winning
/// (`presence::active_and_idle_focused_surfaces_read_present`).
#[tokio::test]
async fn in_the_day_her_call_reads_the_room() {
    let setup = Setup::default().script(Agent::Yinyue, vec![text("Here.")]);
    let (w, _) = day_world_at(12, setup).await;
    let beat = |surface: &'static str, idle_ms: u64| {
        json!({"surface": surface, "focused": true, "typing": false, "idle_ms": idle_ms})
    };
    w.api.post("/api/presence", beat("main-ui", 30_000)).await;
    w.api.post("/api/presence", beat("skill-page", 600_000)).await;
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "Are you there?"})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let call = w.model.calls_of(Agent::Yinyue).pop().expect("her call");
    let them = call
        .thread()
        .into_iter()
        .find_map(|(_, t)| t.lines().find(|l| l.starts_with("- Them: ")).map(str::to_string));
    let them = them.unwrap_or_else(|| panic!("no Right now block in her call: {:?}", call.thread()));
    assert!(them.starts_with("- Them: present_reading"), "{them}");
}

/// Her own talk in the day is offered her own tools — her senses, not
/// Ling's hands — as her own thread offers them (`agents/yinyue.md`).
#[tokio::test]
async fn in_the_day_her_own_talk_offers_her_own_tools() {
    let setup = Setup::default().script(Agent::Yinyue, vec![text("Here.")]);
    let (w, _) = day_world_at(12, setup).await;
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "Are you there?"})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let call = w.model.calls_of(Agent::Yinyue).pop().expect("her call");
    let mut names = call.tool_names();
    names.sort();
    assert!(offers(&call, "sense"), "{names:?}");
    for not in ["Bash", "Write", "Edit"] {
        assert!(!offers(&call, not), "she is offered Ling's {not}: {names:?}");
    }
}

/// Her prompt export at home in the day shows the tools her talk there is
/// offered: her own, not Ling's.
#[tokio::test]
async fn in_the_day_her_prompt_export_at_home_shows_her_own_tools() {
    let setup = Setup::default().script(Agent::Yinyue, vec![text("Here.")]);
    let (w, _) = day_world_at(12, setup).await;
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "Are you there?"})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let day = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let q = format!("/api/chat/system-prompt?project_root=&agent_id=yinyue&session_id={day}");
    let e = w.api.get(&q).await;
    let names: Vec<&str> = e["tools"]
        .as_array()
        .map(|all| all.iter().filter_map(|t| t["function"]["name"].as_str().or(t["name"].as_str())).collect())
        .unwrap_or_default();
    assert!(names.contains(&"sense"), "{names:?}");
    for not in ["Bash", "Write", "Edit"] {
        assert!(!names.contains(&not), "her export shows Ling's {not}: {names:?}");
    }
}

/// The compaction summary is the day's, no place's: Clear on the main page
/// keeps it, a page never folds it, and the next turn starts at it.
#[tokio::test]
async fn clear_at_home_keeps_the_days_summary() {
    use crate::shared_table::{filler, opening};
    let setup = Setup::default()
        .script(Agent::Ling, vec![text("OK."), text("Fine."), text("Still here.")])
        .extra("context compaction assistant", "SUMMARY: Alex sent filler; Ling said OK.");
    let (w, _) = day_world_at(12, setup).await;
    let day = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    turn_at(&w, &day, None, &format!("Ignore this filler.\n{}", filler()), Agent::Ling).await;
    turn_at(&w, &day, None, "Short one.", Agent::Ling).await;
    let resp = w.api.post("/api/chat/compact", json!({"session_id": day, "project_root": w.work()})).await;
    assert_eq!(resp["compacted"], true, "not compacted: {resp}");

    w.api.post("/api/chat/clear", json!({"session_id": day})).await;
    let summaries = w.rows(&day).iter().filter(|r| r.from_id == "compaction").count();
    assert_eq!(summaries, 1, "Clear at home took the day's summary");
    let history = w.api.history(&day, &w.work()).await;
    let shown = history["messages"].as_array().cloned().unwrap_or_default();
    let summary = shown.iter().find(|r| r[1].as_str().is_some_and(|t| t.contains("SUMMARY: Alex")));
    let summary = summary.unwrap_or_else(|| panic!("no summary in {history:#}"));
    assert!(summary[0].get("place").is_none(), "the summary shows a place: {summary}");

    turn_at(&w, &day, None, "Still there?", Agent::Ling).await;
    let call = w.model.calls_of(Agent::Ling).pop().expect("his call");
    assert!(opening(&call).contains("SUMMARY: Alex sent filler"), "the thread lost the summary");
    assert!(!call.text().contains("filler-00001"), "the rows before the summary came back");
}

/// One model a member a day: only the picker keeps one. A model a page
/// sends with its message is for that turn; the main page stays on the
/// picker's.
#[tokio::test]
async fn in_the_day_a_pages_model_is_for_its_turn_only() {
    let setup = Setup::default().script(Agent::Ling, vec![text("Rolled."), text("Home.")]);
    let (w, _) = day_world_at(12, setup).await;
    let day = w.api.create_session(json!({"title": "Dice", "skill": "dice"})).await;
    w.api
        .patch("/api/sessions", json!({"session_id": day, "agent_id": "ling", "model_id": "fake-sol"}))
        .await;
    let mark = w.mark();
    let body = json!({"session_id": day, "project_root": w.work(), "message": "Roll",
        "skill_name": "dice", "model_id": "fake-eye"});
    w.api.post("/api/chat", body).await;
    w.finish_run(&day, Agent::Ling, mark).await;
    turn_at(&w, &day, None, "Back on the main page", Agent::Ling).await;

    let models: Vec<String> = w.model.calls_of(Agent::Ling).iter().map(|c| c.model().to_string()).collect();
    assert_eq!(models, ["fake-eye-1", "fake-sol-1"], "the page's model was kept for the day");
    let kept = meta(&w, &day);
    assert!(kept.contains("fake-sol") && !kept.contains("fake-eye"), "{kept}");
}

/// In the day a page's compaction settings set nothing: the call answers
/// success, the day keeps no focus, and a summary is not written for it.
#[tokio::test]
async fn in_the_day_a_pages_compact_config_sets_nothing() {
    use crate::shared_table::filler;
    let setup = Setup::default()
        .script(Agent::Ling, vec![text("OK."), text("Fine.")])
        .extra("context compaction assistant", "SUMMARY: the day so far.");
    let (w, _) = day_world_at(12, setup).await;
    let day = w.api.create_session(json!({"title": "Dice", "skill": "dice"})).await;
    turn_at(&w, &day, Some("dice"), &format!("Ignore this filler.\n{}", filler()), Agent::Ling).await;
    turn_at(&w, &day, Some("dice"), "Short one.", Agent::Ling).await;

    let set = json!({"session_id": day, "threshold": 0.3, "focus": "Keep every dice roll."});
    let resp = w.api.post("/api/chat/compact_config", set).await;
    assert_eq!(resp["ok"], true, "{resp}");
    let kept = meta(&w, &day);
    assert!(!kept.contains("Keep every dice roll") && !kept.contains("compact_threshold"), "{kept}");

    let resp = w.api.post("/api/chat/compact", json!({"session_id": day, "project_root": w.work()})).await;
    assert_eq!(resp["compacted"], true, "not compacted: {resp}");
    let summarizer = w.model.calls().into_iter().find(|c| c.text().contains("context compaction assistant"));
    let summarizer = summarizer.expect("the summarizer's call");
    assert!(!summarizer.text().contains("Keep every dice roll"), "the day's summary followed the page's focus");
}
