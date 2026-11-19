//! A person's question is never answered on the event path: a message typed
//! to her while a turn of hers the engine started is running waits for that
//! turn, then runs as its own turn — and its answer is shown as any
//! person's is.

use crate::missions::beat_present;
use crate::notes::{she_writes_the_note, the_clock_moves};
use crate::support::model::{slow, Agent};
use crate::support::{say, text, tool, Setup, World};
use serde_json::json;
use std::time::Duration;

/// Every row of every thread, as one text.
fn every_row(w: &World) -> String {
    let dir = w.home.linggen_home().join("sessions");
    let mut all = String::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path().join("messages.jsonl");
        all.push_str(&std::fs::read_to_string(path).unwrap_or_default());
    }
    all
}

/// Her woken note's turn is slow; the person asks her something while it
/// runs. The answer to the question is shown (a row and a spoken line),
/// whatever happens to the turn the engine started.
#[tokio::test]
async fn a_question_typed_during_a_held_turn_gets_a_shown_answer() {
    let setup = Setup::default().script(
        Agent::Yinyue,
        vec![
            tool("note_write", json!({"text": "the parcel arrives tomorrow", "look_again": "+40m"})),
            text("Noted."),
            slow(say("The parcel comes tomorrow.")),
            text("ANSWER-TO-THE-QUESTION it is Tuesday."),
        ],
    );
    let w = World::start(setup).await;
    she_writes_the_note(&w).await;
    beat_present(&w).await;
    let mark = w.mark();
    the_clock_moves(&w);
    // Her woken turn is running (slowly) when the person types to her.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while w.model.calls_of(Agent::Yinyue).len() < 3 {
        assert!(std::time::Instant::now() < deadline, "the woken turn never began");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let day = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let typed = json!({"session_id": day, "project_root": w.work(),
        "agent_id": "yinyue", "message": "What day is it?"});
    let sent = w.api.post("/api/chat", typed).await;
    assert_eq!(sent["status"], "queued", "typed during her turn: {sent}");
    w.wait_agent(Agent::Yinyue, mark).await;
    let events = w
        .events_when(|e| e.iter().any(|x| x["kind"] == "pet_speak" && x["text"].as_str().is_some_and(|t| t.contains("ANSWER-TO-THE-QUESTION"))))
        .await;
    let heard = events
        .iter()
        .any(|x| x["kind"] == "pet_speak" && x["text"].as_str().is_some_and(|t| t.contains("ANSWER-TO-THE-QUESTION")));
    assert!(heard, "her answer to the question was not spoken");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(every_row(&w).contains("ANSWER-TO-THE-QUESTION"), "her answer is no row of the thread");
}

/// The reasons she gave for staying silent, and the malformed lines, with a
/// marker each: none of them may reach a thread, an event or the voice.
fn unspoken() -> [(&'static str, &'static str); 3] {
    [
        ("bare prose", "BARE-PROSE their reply is ready."),
        ("a reason for silence", "他不在，先不说。 WHY-SILENT"),
        ("an unclosed envelope", "<say>UNCLOSED-LINE their reply"),
    ]
}

fn marker(reply: &str) -> &str {
    ["BARE-PROSE", "WHY-SILENT", "UNCLOSED-LINE"]
        .into_iter()
        .find(|m| reply.contains(m))
        .expect("a marker")
}

/// The person is away; Ling finishes a reply in the day's session. The herald
/// is her reply to the kickoff: finalized, then published — or nothing.
async fn ling_finishes_while_away(her_reply: crate::support::model::Reply) -> World {
    ling_finishes_while_away_after(her_reply, "Hello").await
}

/// [`ling_finishes_while_away`], the person's first words being `hello`.
async fn ling_finishes_while_away_after(her_reply: crate::support::model::Reply, hello: &str) -> World {
    let setup = Setup::default()
        .script(Agent::Ling, vec![slow(text("Done for you."))])
        .script(Agent::Yinyue, vec![text("I'm here."), her_reply]);
    let w = World::start(setup).await;
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": hello})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let day = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let mark = w.mark();
    // The person's last written line is the language sample: said in the
    // language of their first words.
    let task = if hello.is_ascii() { "Do the thing" } else { "帮我做这件事" };
    w.api.chat(&day, &w.work(), task).await;
    // Away while the reply is being written: the typed turn's own beat, too.
    for surface in ["user-turn", "main-ui"] {
        let away = json!({"surface": surface, "focused": false, "typing": false, "idle_ms": 900_000});
        w.api.post("/api/presence", away).await;
    }
    w.wait_run(&day, Agent::Ling, mark).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while w.model.calls_of(Agent::Yinyue).len() < 2 {
        assert!(std::time::Instant::now() < deadline, "no herald was asked for; log:\n{}", w.engine.log());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    w
}

#[tokio::test]
async fn a_reply_ready_herald_without_the_envelope_says_nothing() {
    for (why, reply) in unspoken() {
        let w = ling_finishes_while_away(text(reply)).await;
        let m = marker(reply);
        assert!(!every_row(&w).contains(m), "{why}: reached a thread");
        let events = serde_json::to_string(&w.events()).unwrap();
        assert!(!events.contains(m), "{why}: reached a live event");
    }
}

#[tokio::test]
async fn a_reply_ready_herald_in_an_envelope_is_one_row_and_one_line() {
    let w = ling_finishes_while_away(say("HERALD-LINE your reply is ready.")).await;
    let said: Vec<_> = w
        .events()
        .into_iter()
        .filter(|e| e["kind"] == "pet_speak" && e["text"].as_str().is_some_and(|t| t.contains("HERALD-LINE")))
        .collect();
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(every_row(&w).matches("HERALD-LINE").count(), 1, "one row");
}

/// A question waits in a turn of Ling's (holding the day's session) while the
/// person is away; her herald reply is `her_reply`, slow when `late` — so
/// the question can be answered before her line is ready. Returns the world
/// and the day's id.
async fn a_question_waits_while_away(
    app: &str,
    private: bool,
    her_reply: crate::support::model::Reply,
    answer_first: bool,
) -> World {
    use crate::day_session::{day_world_at, grant_until, install_app, say as say_in};
    let question = "QUESTION-TEXT: which box?";
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
        .script(Agent::Yinyue, vec![text("I'm here."), her_reply, say("Your reply is ready.")]);
    let (w, _) = day_world_at(12, setup).await;
    install_app(&w, app, "Bash, AskUser", private).await;
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "Hello"})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let day = w.api.create_session(json!({"title": "App", "skill": app})).await;
    say_in(&w, &day, Some(app), "Move the box").await;
    grant_until(&w, app, 2).await;
    let away = json!({"surface": "user-turn", "focused": false, "typing": false, "idle_ms": 600_000});
    w.api.post("/api/presence", away).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let asked = loop {
        assert!(std::time::Instant::now() < deadline, "no question; log:\n{}", w.engine.log());
        let asks = w.api.get(&format!("/api/pending-ask-user?skill={app}")).await;
        if let Some(ask) = asks.as_array().and_then(|a| a.first()).cloned() {
            break ask;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    if answer_first {
        // Answered while her line is still being written.
        tokio::time::sleep(Duration::from_millis(500)).await;
        w.api.answer(&asked, "Red").await;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while w.model.calls_of(Agent::Yinyue).len() < 2 {
        assert!(std::time::Instant::now() < deadline, "she was never asked; log:\n{}", w.engine.log());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_secs(4)).await;
    if !answer_first {
        w.api.answer(&asked, "Red").await;
    }
    w
}

fn nothing_of(w: &World, why: &str, reply: &str) {
    let m = marker(reply);
    assert!(!every_row(w).contains(m), "{why}: reached a thread");
    let events = serde_json::to_string(&w.events()).unwrap();
    assert!(!events.contains(m), "{why}: reached a live event");
}

#[tokio::test]
async fn an_ask_herald_without_the_envelope_says_nothing_public_or_private() {
    for (app, private) in [("porter", false), ("vault", true)] {
        for (why, reply) in unspoken() {
            let w = a_question_waits_while_away(app, private, text(reply), false).await;
            nothing_of(&w, why, reply);
        }
    }
}

/// The permission or question herald, her line ready only after the question
/// was answered: nothing is shown, written or spoken.
#[tokio::test]
async fn a_question_answered_before_her_line_was_ready_is_not_heralded() {
    let w = a_question_waits_while_away("porter", false, slow(say("LATE-HERALD they need you.")), true).await;
    let events = serde_json::to_string(&w.events()).unwrap();
    assert!(!events.contains("LATE-HERALD"), "a stale herald reached a live event");
    assert!(!every_row(&w).contains("LATE-HERALD"), "a stale herald reached a thread");
}

const IN_CHINESE: &str = "Reply in Simplified Chinese.";

/// The person writes Chinese; the herald for a finished reply (a full turn
/// of hers) is asked in their language.
#[tokio::test]
async fn a_full_turn_herald_is_asked_in_the_persons_language() {
    let w = ling_finishes_while_away_after(say("好了。"), "你好，帮我看看这个文件").await;
    let herald = w.model.calls_of(Agent::Yinyue).into_iter().nth(1).expect("her herald call");
    assert!(herald.text().contains(IN_CHINESE), "no language on the full path");
}

/// The same on the busy-day light path: a question waits inside Ling's turn,
/// so her line is a light turn — and it carries the language too.
#[tokio::test]
async fn a_light_turn_herald_is_asked_in_the_persons_language() {
    use crate::day_session::{day_world_at, grant_until, install_app, say as say_in};
    let setup = Setup::default()
        .script(
            Agent::Ling,
            vec![
                tool("Bash", json!({"cmd": "sleep 3; echo moved", "run_in_background": true})),
                text("Started."),
                tool("AskUser", json!({"questions": [{"question": "Which box?", "header": "Box",
                    "options": [{"label": "Red"}, {"label": "Blue"}]}]})),
                text("Red it is."),
            ],
        )
        .script(Agent::Yinyue, vec![text("我在。"), say("他们需要你。"), say("好了。")]);
    let (w, _) = day_world_at(12, setup).await;
    install_app(&w, "porter", "Bash, AskUser", false).await;
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "你好，帮我看看这个文件"})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let day = w.api.create_session(json!({"title": "App", "skill": "porter"})).await;
    say_in(&w, &day, Some("porter"), "把箱子挪一下").await;
    grant_until(&w, "porter", 2).await;
    let away = json!({"surface": "user-turn", "focused": false, "typing": false, "idle_ms": 600_000});
    w.api.post("/api/presence", away).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let asked = loop {
        assert!(std::time::Instant::now() < deadline, "no question; log:\n{}", w.engine.log());
        let asks = w.api.get("/api/pending-ask-user?skill=porter").await;
        if let Some(ask) = asks.as_array().and_then(|a| a.first()).cloned() {
            break ask;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let notice = loop {
        let calls = w.model.calls_of(Agent::Yinyue);
        let found = calls.into_iter().skip(1).find(|c| {
            c.thread().last().is_some_and(|(_, said)| said.contains("the user to answer"))
        });
        if let Some(call) = found {
            break call;
        }
        assert!(std::time::Instant::now() < deadline, "she never heralded");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    w.api.answer(&asked, "Red").await;
    assert!(notice.text().contains(IN_CHINESE), "no language on the light path");
}

/// Ling hands her a message with `agent_chat`: she answers it as an event.
async fn ling_messages_her(her_reply: crate::support::model::Reply) -> World {
    use crate::day_session::{day_world_at, turn_at};
    let setup = Setup::default()
        .script(Agent::Yinyue, vec![text("I'm here."), her_reply])
        .script(
            Agent::Ling,
            vec![tool("agent_chat", json!({"to": "yinyue", "message": "The dice are back."})), text("Told her.")],
        );
    let (w, _) = day_world_at(12, setup).await;
    let day = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "Hello"})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let mark = w.mark();
    turn_at(&w, &day, None, "Tell her the dice are back", Agent::Ling).await;
    w.finish_run(&day, Agent::Yinyue, mark).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    w
}

#[tokio::test]
async fn an_agent_chat_answered_without_the_envelope_says_nothing() {
    for (why, reply) in unspoken() {
        let w = ling_messages_her(text(reply)).await;
        nothing_of(&w, why, reply);
    }
}

#[tokio::test]
async fn an_agent_chat_answered_in_an_envelope_is_one_row_and_one_line() {
    let w = ling_messages_her(say("CHAT-LINE the dice are back.")).await;
    let said = w
        .events()
        .into_iter()
        .filter(|e| e["kind"] == "pet_speak" && e["text"].as_str().is_some_and(|t| t.contains("CHAT-LINE")))
        .count();
    assert_eq!(said, 1);
    assert_eq!(every_row(&w).matches("CHAT-LINE").count(), 1);
}

/// A quiet glance: the cadence is shortened for the test; her reply to the
/// "quiet moment" kickoff is the one that matters.
async fn a_quiet_glance(her_reply: crate::support::model::Reply) -> World {
    let setup = Setup {
        env: vec![("LINGGEN_AMBIENT_SECS", "3".to_string())],
        ..Setup::default()
    }
    .script(Agent::Yinyue, vec![text("Hello."), her_reply]);
    let w = World::start(setup).await;
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "Hello"})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let mark = w.mark();
    w.wait_agent(Agent::Yinyue, mark).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    w
}

#[tokio::test]
async fn a_quiet_glance_without_the_envelope_says_nothing() {
    for (why, reply) in unspoken() {
        let w = a_quiet_glance(text(reply)).await;
        nothing_of(&w, why, reply);
    }
}

#[tokio::test]
async fn a_quiet_glance_in_an_envelope_is_one_row_and_one_line() {
    let w = a_quiet_glance(say("GLANCE-LINE the evening is quiet.")).await;
    let events = w
        .events_when(|e| e.iter().any(|x| x["kind"] == "pet_speak" && x["text"].as_str().is_some_and(|t| t.contains("GLANCE-LINE"))))
        .await;
    let said = events
        .iter()
        .filter(|e| e["kind"] == "pet_speak" && e["text"].as_str().is_some_and(|t| t.contains("GLANCE-LINE")))
        .count();
    assert_eq!(said, 1);
    assert_eq!(every_row(&w).matches("GLANCE-LINE").count(), 1);
}

/// A restart left her session's entry: her turn was running, begun as
/// `origin` (none: a person's). The new engine continues it; `her_reply` is
/// what she says to the continue turn.
async fn a_restart_continues_her_turn(origin: Option<&str>, her_reply: crate::support::model::Reply) -> World {
    use crate::day_session::day_world_at;
    let setup = Setup::default().script(Agent::Yinyue, vec![text("I'm here."), her_reply]);
    let (mut w, _) = day_world_at(12, setup).await;
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "Hello"})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let day = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();
    let interrupted = origin != Some("task_end");
    let mut entry = json!({"session_id": day, "agent_id": "yinyue", "caller": false,
        "turn_interrupted": interrupted, "tasks": []});
    if let Some(origin) = origin {
        entry["origin"] = json!(origin);
    }
    crate::restart::leave_plan(&w, json!([entry]));
    let mark = w.mark();
    w.engine.restart(&w.home).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    w
}

/// A turn of hers a PERSON started, interrupted by a restart and continued:
/// it stays on the answer path — her plain answer is shown, as ever.
#[tokio::test]
async fn a_persons_turn_continued_after_a_restart_is_still_answered_in_the_open() {
    let w = a_restart_continues_her_turn(None, text("CONTINUED-ANSWER here is where we were.")).await;
    assert!(every_row(&w).contains("CONTINUED-ANSWER"), "a person's answer was dropped");
}

/// One an engine-started wake began: the continue turn speaks only in the
/// envelope — and a bare reply says nothing.
#[tokio::test]
async fn an_event_turn_continued_after_a_restart_speaks_only_in_the_envelope() {
    let w = a_restart_continues_her_turn(Some("herald"), text("BARE-PROSE continued thinking.")).await;
    assert!(!every_row(&w).contains("BARE-PROSE"), "bare prose reached a thread");
    let w = a_restart_continues_her_turn(Some("herald"), say("RESTART-LINE back on the new build.")).await;
    assert_eq!(every_row(&w).matches("RESTART-LINE").count(), 1);
}

/// A notice told to her with no turn of hers behind it (a task's end, a
/// restart's note): she speaks only in the envelope, and the contract is
/// attached — a bare reply says nothing, an envelope is one row, one line.
#[tokio::test]
async fn a_task_end_told_to_her_speaks_only_in_the_envelope() {
    let w = a_restart_continues_her_turn(Some("task_end"), text("BARE-PROSE the job finished.")).await;
    assert!(!every_row(&w).contains("BARE-PROSE"), "bare prose reached a thread");
    let w = a_restart_continues_her_turn(Some("task_end"), say("TASK-LINE the job finished.")).await;
    assert_eq!(every_row(&w).matches("TASK-LINE").count(), 1);
    let told = w.model.calls_of(Agent::Yinyue).pop().expect("her call");
    assert!(told.text().contains("<say>"), "the speaking contract was not attached to the notice");
}
