//! A private app (`private-talk: true`) with one session a day on
//! (doc/one-ling-spec.md § One session a day): it keeps a session of its
//! own and joins the day by no way in; and a day is its own person's — what
//! another person's turn asks never reaches the builder's thread or voice.

use crate::day_session::{day_world_at, install_ledger, say};
use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use serde_json::{json, Value};

/// Every word the ledger's talk below carries.
fn holds_ledger(text: &str) -> bool {
    text.contains("LEDGER-ONLY") || text.contains("LEDGER-REPLY")
}

fn ids(listed: &Value) -> Vec<String> {
    listed["sessions"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s["id"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn day_text(w: &World, day: &str) -> String {
    serde_json::to_string(&w.raw_rows(day)).unwrap()
}

/// Wait until some session of the ledger holds `marker`; that session's id.
async fn ledger_session_with(w: &World, marker: &str) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let listed = w.api.get("/api/skill-sessions?skill=ledger").await;
        let found = ids(&listed).into_iter().find(|sid| {
            serde_json::to_string(&w.raw_rows(sid))
                .unwrap()
                .contains(marker)
        });
        if let Some(sid) = found {
            return sid;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no ledger session holds {marker}; log:\n{}",
            w.engine.log()
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// A private app's chat joins the day by none of the ways in: a new
/// session, its app's list, the day's lookup, a chat with no session, a
/// chat that names the day, her moment, and a message delivered to Ling in
/// that app. An app that does not resolve stays apart too. The day holds
/// nothing of it.
#[tokio::test]
async fn a_private_apps_chat_joins_the_day_by_no_way_in() {
    let setup = Setup::default()
        .script(
            Agent::Ling,
            vec![
                text("LEDGER-REPLY one."),
                text("LEDGER-REPLY two."),
                text("LEDGER-REPLY relayed."),
            ],
        )
        .script(
            Agent::Yinyue,
            vec![
                text("LEDGER-REPLY her line."),
                tool(
                    "agent_chat",
                    json!({"to": "ling", "message": "Count the shares.", "app": "ledger"}),
                ),
                text("Passed on."),
            ],
        );
    let (w, _) = day_world_at(12, setup).await;
    install_ledger(&w).await;
    let day = w.api.get("/api/sessions/day").await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let own = w
        .api
        .create_session(json!({"title": "Ledger", "skill": "ledger"}))
        .await;
    assert_ne!(own, day, "a new session of the private app is the day's");
    let listed = ids(&w.api.get("/api/skill-sessions?skill=ledger").await);
    assert!(
        !listed.contains(&day),
        "the private app's list holds the day: {listed:?}"
    );
    assert!(
        listed.contains(&own),
        "the private app's own session is not listed: {listed:?}"
    );
    let asked = w.api.get("/api/sessions/day?skill=ledger").await;
    assert_ne!(
        asked["id"],
        json!(day),
        "the day's lookup gave the private app the day: {asked}"
    );

    let mark = w.mark();
    let body =
        json!({"project_root": w.work(), "message": "LEDGER-ONLY first", "skill_name": "ledger"});
    let sent = w.api.post("/api/chat", body).await;
    let first = sent["session_id"].as_str().unwrap_or_default().to_string();
    assert_ne!(first, day, "a chat with no session joined the day");
    w.finish_run(&first, Agent::Ling, mark).await;

    let mark = w.mark();
    let sent = say(&w, &day, Some("ledger"), "LEDGER-ONLY second").await;
    let second = sent["session_id"].as_str().unwrap_or_default().to_string();
    assert_ne!(second, day, "a chat naming the day joined it");
    w.finish_run(&second, Agent::Ling, mark).await;

    let mark = w.mark();
    let moment = json!({"app": "ledger", "text": "LEDGER-ONLY the books balance.", "asked": true, "session": own});
    assert_eq!(w.api.post("/api/yinyue/event", moment).await, json!("ok"));
    w.finish_run(&own, Agent::Yinyue, mark).await;
    assert!(
        day_text(&w, &day).find("LEDGER-").is_none(),
        "her moment ran in the day"
    );

    let mark = w.mark();
    w.api
        .post("/api/yinyue/chat", json!({"text": "Ask Ling to count."}))
        .await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let relayed = ledger_session_with(&w, "LEDGER-REPLY relayed.").await;
    assert_ne!(relayed, day);

    let gone = w
        .api
        .create_session(json!({"title": "Gone", "skill": "zz-gone"}))
        .await;
    assert_ne!(gone, day, "an app that does not resolve joined the day");
    assert!(
        !holds_ledger(&day_text(&w, &day)),
        "the day holds the ledger's talk: {}",
        day_text(&w, &day)
    );
}

/// A question waiting in another person's day while the builder is away:
/// she is not woken with it in the builder's thread, and nothing is said
/// aloud on this Mac.
#[tokio::test]
async fn another_persons_question_reaches_neither_the_builders_thread_nor_her_voice() {
    let setup = Setup::default()
        .script(
            Agent::Ling,
            vec![
                tool("Bash", json!({"cmd": "sleep 3; echo moved", "run_in_background": true})),
                text("Started."),
                tool(
                    "AskUser",
                    json!({"questions": [{"question": "FRIEND-QUESTION: which box?", "header": "Box",
                        "options": [{"label": "Red"}, {"label": "Blue"}]}]}),
                ),
                text("Red it is."),
            ],
        )
        .script(Agent::Yinyue, vec![text("I'm here."), text("They need you back.")]);
    let (w, _) = day_world_at(12, setup).await;
    crate::day_session::install_app(&w, "porter", "Bash, AskUser", false).await;
    let mark = w.mark();
    w.api
        .post("/api/yinyue/chat", json!({"text": "Hello"}))
        .await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let mine = w.api.get("/api/sessions/day").await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let body = json!({"project_root": w.work(), "message": "Move the box", "skill_name": "porter", "user_id": "u-friend"});
    let sent = w.api.post("/api/chat", body).await;
    let theirs = sent["session_id"].as_str().unwrap_or_default().to_string();
    assert_ne!(theirs, mine, "{sent}");
    crate::day_session::grant_until(&w, "porter", 2).await;
    let away =
        json!({"surface": "user-turn", "focused": false, "typing": false, "idle_ms": 600_000});
    w.api.post("/api/presence", away).await;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let asked = loop {
        assert!(
            std::time::Instant::now() < deadline,
            "no question; log:\n{}",
            w.engine.log()
        );
        let asks = w.api.get("/api/pending-ask-user?skill=porter").await;
        if let Some(ask) = asks.as_array().and_then(|a| a.first()).cloned() {
            break ask;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    // Her herald, when there is one, comes at once: give it a few seconds.
    let quiet = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while w.model.calls_of(Agent::Yinyue).len() < 2 && std::time::Instant::now() < quiet {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let told: Vec<String> = w
        .model
        .calls_of(Agent::Yinyue)
        .iter()
        .skip(1)
        .map(|c| c.text())
        .collect();
    assert!(
        told.iter().all(|t| !t.contains("FRIEND-QUESTION")),
        "another person's question reached her: {told:?}"
    );
    let spoke: Vec<Value> = w
        .events()
        .into_iter()
        .filter(|e| e["kind"] == "pet_speak")
        .collect();
    assert!(
        spoke.is_empty(),
        "her voice spoke for another person's question: {spoke:?}"
    );
    assert!(
        !day_text(&w, &mine).contains("FRIEND-QUESTION"),
        "the builder's day holds it"
    );
    w.api.answer(&asked, "Red").await;
}
