//! A private app (`private-talk: true`) with the daily session on
//! (doc/one-ling-spec.md § Linggen's daily session): it keeps a session of its
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
                // The local owner opens this person's day and starts the
                // question directly. Task ends must use the person's own
                // policy, regardless of who opened their day.
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
        let asks = w.api.get("/api/pending-ask-user?skill=porter&user_id=u-friend").await;
        if let Some(ask) = asks.as_array().and_then(|a| a.first()).cloned() {
            break ask;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    // Her herald, when there is one, comes at once: give it a few seconds.
    // Her greeting above is her one line until then.
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
        .filter(|e| e["kind"] == "pet_speak" && e["text"] != "I'm here.")
        .collect();
    assert!(
        spoke.is_empty(),
        "her voice spoke for another person's question: {spoke:?}"
    );
    assert!(
        !day_text(&w, &mine).contains("FRIEND-QUESTION"),
        "the builder's day holds it"
    );
    w.api.answer_as(&asked, "Red", Some("u-friend")).await;
}

/// A subagent of a private app's turn keeps its talk there too: its
/// message to another agent is refused with a reason it reads, and nothing
/// reaches her.
#[tokio::test]
async fn a_private_apps_subagent_carries_no_words_out() {
    let setup = Setup::default().script(
        Agent::Ling,
        vec![
            tool(
                "Task",
                json!({"target_agent_id": "ling", "task": "Tell Yinyue the shares"}),
            ),
            tool(
                "agent_chat",
                json!({"to": "yinyue", "message": "LEDGER-ONLY news"}),
            ),
            text("Kept it here."),
            text("Done."),
        ],
    );
    let (w, _) = day_world_at(12, setup).await;
    crate::day_session::install_app(&w, "relay", "Task, agent_chat", true).await;
    let own = w
        .api
        .create_session(json!({"title": "Relay", "skill": "relay"}))
        .await;
    let mark = w.mark();
    say(&w, &own, Some("relay"), "Hand it off").await;
    w.finish_run(&own, Agent::Ling, mark).await;
    let calls = w.model.calls_of(Agent::Ling);
    assert!(
        calls[2]
            .text()
            .contains("refused: relay keeps what is said in it to itself"),
        "{}",
        calls[2].text()
    );
    assert!(
        w.model.calls_of(Agent::Yinyue).is_empty(),
        "she was reached"
    );
}

/// The words of the ledger's own SKILL.md.
const LEDGER_RULES: &str = "You keep the ledger.";

/// In the day, a person's `/ledger …` is carried to the ledger's own chat,
/// as a chat that names the day from the ledger's page is: its turn, its
/// rows and its reply stand there, and the day holds none of it.
#[tokio::test]
async fn a_private_apps_slash_command_in_the_day_is_carried_to_its_own_chat() {
    let setup = Setup::default().script(
        Agent::Ling,
        vec![text("LEDGER-REPLY carried."), text("LEDGER-REPLY addressed.")],
    );
    let (w, _) = day_world_at(12, setup).await;
    install_ledger(&w).await;
    let day = w.api.get("/api/sessions/day").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let mark = w.mark();
    let sent = say(&w, &day, None, "/ledger LEDGER-ONLY count the shares").await;
    let carried = sent["session_id"].as_str().unwrap_or_default().to_string();
    assert_ne!(carried, day, "the slash command ran in the day: {sent}");
    w.finish_run(&carried, Agent::Ling, mark).await;
    assert!(
        day_text(&w, &carried).contains("LEDGER-REPLY carried."),
        "the ledger's chat holds no reply"
    );
    let calls = w.model.calls_of(Agent::Ling);
    assert!(calls[0].text().contains(LEDGER_RULES), "the ledger's turn ran without its rules");
    // Addressed to Ling, the same command is carried too.
    let mark = w.mark();
    let sent = say(&w, &day, None, "@ling //ledger LEDGER-ONLY once more").await;
    let addressed = sent["session_id"].as_str().unwrap_or_default().to_string();
    assert_ne!(addressed, day, "an addressed slash command ran in the day: {sent}");
    w.finish_run(&addressed, Agent::Ling, mark).await;
    assert!(
        !holds_ledger(&day_text(&w, &day)),
        "the day holds the ledger's talk: {}",
        day_text(&w, &day)
    );
}

/// In the day, Ling's `Skill` tool is refused a private app with a reason
/// she reads: its SKILL.md never reaches the day, nor its rows.
#[tokio::test]
async fn the_skill_tool_is_refused_a_private_app_in_the_day() {
    let setup = Setup::default().script(
        Agent::Ling,
        vec![tool("Skill", json!({"skill": "ledger"})), text("Not here.")],
    );
    let (w, _) = day_world_at(12, setup).await;
    install_ledger(&w).await;
    let day = w.api.get("/api/sessions/day").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let mark = w.mark();
    say(&w, &day, None, "Open the books").await;
    w.finish_run(&day, Agent::Ling, mark).await;
    let calls = w.model.calls_of(Agent::Ling);
    let after = calls[1].text();
    assert!(
        after.contains("refused: ledger keeps a session of its own"),
        "no refusal the model reads: {after}"
    );
    assert!(!after.contains(LEDGER_RULES), "the ledger's SKILL.md reached the day's turn");
    assert!(
        !day_text(&w, &day).contains(LEDGER_RULES),
        "the day holds the ledger's SKILL.md"
    );
}

/// A question waiting in a private app's own chat is never framed into her
/// turn at home — at most that the app waits — and she cannot answer it
/// from there: it stays open in the app.
#[tokio::test]
async fn a_private_apps_question_is_not_framed_into_her_turn_nor_answered_from_home() {
    let ask = json!({"questions": [{"question": "VAULT-QUESTION which drawer?", "header": "Drawer",
        "options": [{"label": "Red"}, {"label": "Blue"}]}]});
    let setup = Setup::default()
        .script(Agent::Ling, vec![tool("AskUser", ask), text("Red drawer.")])
        .script(
            Agent::Yinyue,
            vec![tool("answer_prompt", json!({"answer": "Red"})), text("Noted.")],
        );
    let (w, _) = day_world_at(12, setup).await;
    crate::day_session::install_app(&w, "vault", "AskUser", true).await;
    let own = w.api.create_session(json!({"title": "Vault", "skill": "vault"})).await;
    let mark = w.mark();
    say(&w, &own, Some("vault"), "Open a drawer").await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while w.api.get("/api/pending-ask-user?skill=vault").await.as_array().is_none_or(|a| a.is_empty()) {
        assert!(std::time::Instant::now() < deadline, "no question; log:\n{}", w.engine.log_since(mark));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "Red"})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let mut wrong = Vec::new();
    let her = w.model.calls_of(Agent::Yinyue);
    if her.iter().any(|c| c.text().contains("VAULT-QUESTION")) {
        wrong.push("the question was framed into her turn".to_string());
    }
    let day = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    if day_text(&w, &day).contains("VAULT-QUESTION") {
        wrong.push("the day holds the question".into());
    }
    let open = w.api.get("/api/pending-ask-user?skill=vault").await;
    if open.as_array().is_none_or(|a| a.is_empty()) {
        wrong.push("she answered the private app's question from home".into());
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A private app's trigger said in the day — typed into the day's chat, or
/// said to her — is carried, words and all, to the app's own chat: the day
/// keeps no row of it.
#[tokio::test]
async fn a_private_apps_trigger_in_the_day_leaves_no_row_there() {
    let setup = Setup::default()
        .script(Agent::Ling, vec![text("VAULT-REPLY opened.")])
        .script(Agent::Yinyue, vec![text("VAULT-REPLY hers.")]);
    let (w, _) = day_world_at(12, setup).await;
    let dir = w.home.linggen_home().join("skills/vault");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: vault\ndescription: A private test vault.\nallowed-tools: []\n\
         private-talk: true\ntrigger: \"vault:\"\nmembers: [ling, yinyue]\n---\n\nYou keep the vault.\n",
    )
    .unwrap();
    w.api.post("/api/skills/reload", json!({})).await;
    let day = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let mark = w.mark();
    let sent = say(&w, &day, None, "vault: VAULT-WORDS open it").await;
    let carried = sent["session_id"].as_str().unwrap_or_default().to_string();
    assert_ne!(carried, day, "the trigger ran in the day: {sent}");
    w.finish_run(&carried, Agent::Ling, mark).await;
    assert!(day_text(&w, &carried).contains("VAULT-WORDS"), "the words did not go with the turn");

    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "vault: VAULT-SAID to her"})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let held = ledger_like_session_with(&w, "vault", "VAULT-SAID").await;
    assert_ne!(held, day);
    assert!(!day_text(&w, &day).contains("VAULT-"), "the day holds the trigger's words: {}", day_text(&w, &day));
}

/// Rule 3, the private app's exit: a line SPOKEN to her that names a private
/// app is not carried to its chat — a spoken line would be heard back at a
/// place whose talk stays there, and that chat has no way to hear it. Her door
/// says so in one line; nothing of it is kept or run anywhere. The typed twin
/// is carried exactly as before.
#[tokio::test]
async fn a_spoken_line_naming_a_private_app_is_refused_at_her_door_and_a_typed_one_is_carried() {
    let setup = Setup::default()
        .script(Agent::Ling, vec![text("VAULT-REPLY opened.")])
        .script(Agent::Yinyue, vec![text("VAULT-REPLY hers.")]);
    let (w, _) = day_world_at(12, setup).await;
    let dir = w.home.linggen_home().join("skills/vault");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: vault\ndescription: A private test vault.\nallowed-tools: []\n\
         private-talk: true\ntrigger: \"vault:\"\nmembers: [ling, yinyue]\n---\n\nYou keep the vault.\n",
    )
    .unwrap();
    w.api.post("/api/skills/reload", json!({})).await;
    let day = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();

    let spoken = json!({"text": "vault: VAULT-SPOKEN open it", "asked_aloud": true});
    let (status, body) = w.api.post_with_status("/api/yinyue/chat", spoken).await;
    assert_eq!((status, body["error"].as_str()), (422, Some("not_by_voice")), "{body}");
    let reason = body["reason"].as_str().unwrap_or_default();
    assert!(!reason.is_empty() && reason.split_whitespace().count() <= 20, "{body}");
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let listed = w.api.get("/api/skill-sessions?skill=vault").await;
    for sid in ids(&listed).into_iter().chain([day.clone()]) {
        assert!(!day_text(&w, &sid).contains("VAULT-SPOKEN"), "the spoken line was kept in {sid}");
    }

    let mark = w.mark();
    let typed = json!({"text": "vault: VAULT-TYPED open it", "asked_aloud": false});
    assert_eq!(w.api.post("/api/yinyue/chat", typed).await, json!("ok"));
    w.wait_agent(Agent::Yinyue, mark).await;
    let held = ledger_like_session_with(&w, "vault", "VAULT-TYPED").await;
    assert_ne!(held, day);
}

/// Wait until some session of `app` holds `marker`; that session's id.
async fn ledger_like_session_with(w: &World, app: &str, marker: &str) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let listed = w.api.get(&format!("/api/skill-sessions?skill={app}")).await;
        let found = ids(&listed).into_iter().find(|sid| day_text(w, sid).contains(marker));
        if let Some(sid) = found {
            return sid;
        }
        assert!(std::time::Instant::now() < deadline, "no {app} session holds {marker}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
