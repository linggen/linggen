//! A line an app's page sends with "this was spoken" (`heard: true`, from
//! `shared/chat-bridge.js` `send(text, {heard})`) is kept as an ordinary row
//! of the person with the row's `heard` mark, whether it was handled at once
//! or waited behind a running turn (`queue: after-turn`). The mark is a
//! person's alone, and a mark and nothing else: no extra turn, reply or
//! permission follows from it. Rows are read back from the thread on disk.

use crate::support::engine::RunEnd;
use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use serde_json::{json, Value};

/// A made-up app whose page holds the microphone and whose chat keeps a
/// message sent mid-turn until the turn is over.
async fn install_speaker(w: &World) {
    let dir = w.home.linggen_home().join("skills/speaker");
    std::fs::create_dir_all(&dir).expect("speaker dir");
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: speaker\ndescription: A test app that is talked to.\n\
         allowed-tools: [AskUser]\nqueue: after-turn\nmembers: [ling, yinyue]\n---\n\nAnswer in one line.\n",
    )
    .expect("SKILL.md");
    w.api.post("/api/skills/reload", json!({})).await;
}

/// One line from the app's page, as the chat sends it.
async fn send(w: &World, sid: &str, line: &str, extra: Value) -> Value {
    let mut body = json!({
        "session_id": sid, "project_root": w.work(), "message": line, "skill_name": "speaker",
    });
    for (key, value) in extra.as_object().cloned().unwrap_or_default() {
        body[key] = value;
    }
    w.api.post("/api/chat", body).await
}

/// The person's rows of `sid` (not tool rows), as the thread holds them.
fn person_rows(w: &World, sid: &str) -> Vec<Value> {
    w.raw_rows(sid)
        .into_iter()
        .filter(|r| r["from_id"] == "user" && r["is_observation"] == false)
        .collect()
}

/// The mark of the person's row that says `line`: none when the row has no
/// `heard` field (it is left out when false).
fn mark_of(w: &World, sid: &str, line: &str) -> Option<Value> {
    let rows = person_rows(w, sid);
    let row = rows.iter().find(|r| r["content"] == line);
    let row = row.unwrap_or_else(|| panic!("no row of the person says {line:?}: {rows:#?}"));
    row.get("heard").cloned()
}

/// Wait until Ling's line `reply` is in `sid`'s thread (its turn is over).
async fn wait_for_reply(w: &World, sid: &str, reply: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !serde_json::to_string(&w.raw_rows(sid))
        .unwrap()
        .contains(reply)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "no {reply:?} in the thread:\n{}",
            w.engine.log()
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn the_session(w: &World) -> String {
    install_speaker(w).await;
    w.api
        .create_session(json!({"title": "Speaker", "skill": "speaker"}))
        .await
}

/// A line handled at once keeps the mark; a typed line next to it is as ever.
#[tokio::test]
async fn a_spoken_line_is_kept_as_the_persons_row_marked_heard() {
    let w =
        World::start(Setup::default().script(Agent::Ling, vec![text("First."), text("Second.")]))
            .await;
    let sid = the_session(&w).await;
    let mark = w.mark();
    let sent = send(
        &w,
        &sid,
        "Bonjour, je m'appelle Alex",
        json!({"heard": true}),
    )
    .await;
    assert_eq!(sent["agent_id"], "ling", "{sent}");
    w.finish_run(&sid, Agent::Ling, mark).await;
    let mark = w.mark();
    send(&w, &sid, "Typed this one", json!({})).await;
    w.finish_run(&sid, Agent::Ling, mark).await;

    assert_eq!(
        mark_of(&w, &sid, "Bonjour, je m'appelle Alex"),
        Some(json!(true))
    );
    assert_eq!(
        mark_of(&w, &sid, "Typed this one"),
        None,
        "a request without the flag is as before"
    );

    // The chat's history view tells the page which rows were spoken, so its
    // mic mark is there after a reload; a typed row says nothing of it.
    let history = w.api.history(&sid, &w.work()).await;
    let shown = history["messages"].as_array().cloned().unwrap_or_default();
    let meta_of = |line: &str| {
        shown
            .iter()
            .find(|r| r[1] == line)
            .map(|r| r[0].clone())
            .unwrap_or_else(|| panic!("{line}: {history}"))
    };
    assert_eq!(meta_of("Bonjour, je m'appelle Alex")["heard"], json!(true));
    assert!(meta_of("Typed this one").get("heard").is_none());
    w.assert_all_scripted();
}

/// A line that waited behind the running turn is kept, when its turn comes,
/// with the mark it was sent with; the line before it, typed, has none.
#[tokio::test]
async fn a_spoken_line_that_waited_in_the_queue_keeps_the_mark() {
    let ask = json!({"questions": [{"question": "Which box?", "header": "Box",
        "options": [{"label": "Red"}, {"label": "Blue"}]}]});
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![tool("AskUser", ask), text("Red."), text("Heard you.")],
    ))
    .await;
    let sid = the_session(&w).await;
    let mark = w.mark();
    send(&w, &sid, "Pick a box", json!({})).await;
    let RunEnd::Asked(asked) = w.wait_run(&sid, Agent::Ling, mark).await else {
        panic!("no question");
    };
    let queued = send(&w, &sid, "Je voudrais un café", json!({"heard": true})).await;
    assert_eq!(queued["status"], "queued", "sent mid-turn: {queued}");
    assert!(
        person_rows(&w, &sid)
            .iter()
            .all(|r| r["content"] != "Je voudrais un café"),
        "a queued line is kept when it is taken, not before"
    );
    w.api.answer(&asked, "Red").await;
    wait_for_reply(&w, &sid, "Heard you.").await;

    assert_eq!(mark_of(&w, &sid, "Je voudrais un café"), Some(json!(true)));
    assert_eq!(mark_of(&w, &sid, "Pick a box"), None);
    w.assert_all_scripted();
}

/// The flag is a person's: a line sent as an agent is kept as that agent's,
/// unmarked, and the request is otherwise answered as ever.
#[tokio::test]
async fn the_flag_is_ignored_for_a_line_that_is_not_the_persons() {
    let w = World::start(Setup::default().script(Agent::Ling, vec![text("Noted.")])).await;
    let sid = the_session(&w).await;
    let mark = w.mark();
    send(
        &w,
        &sid,
        "Relayed words",
        json!({"heard": true, "sender": "yinyue"}),
    )
    .await;
    w.finish_run(&sid, Agent::Ling, mark).await;

    let rows = w.raw_rows(&sid);
    let row = rows
        .iter()
        .find(|r| r["content"] == "Relayed words")
        .expect("the relayed row");
    assert_eq!(row["from_id"], "yinyue", "{row}");
    assert!(
        row.get("heard").is_none(),
        "an agent's line was marked spoken: {row}"
    );
    w.assert_all_scripted();
}
