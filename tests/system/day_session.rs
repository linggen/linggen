//! One session a day (doc/one-ling-spec.md § One session a day): with
//! `agent.one_session_a_day` on, the day's session is found by the day —
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

    set_agent(&w, json!({ "one_session_a_day": false })).await;
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

    set_agent(&w, json!({ "one_session_a_day": true })).await;
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
    let list = w.api.get(&format!("/api/skill-sessions?skill=dice&user_id={friend}")).await;
    assert_eq!(list["sessions"][0]["id"], theirs.as_str());
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
