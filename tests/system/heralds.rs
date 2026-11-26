//! What Yinyue tells a person about Ling's work (doc/yinyue-spec.md
//! § Herald): one notice per thing, told by voice only while the person is
//! away, never a row of its own; the thing it is about carries a folded mark;
//! one telling per waiting turn; a run another tool started is never told;
//! and a turn the engine started for her that ends without words leaves
//! nothing on any page.

use crate::notes::{she_writes_the_note, the_clock_moves};
use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use serde_json::Value;
use std::time::Duration;

/// The events of hers a thread page would draw: anything she sends that is
/// not a cue of her stage (`pet_*`), from `from` on.
fn her_thread_events(w: &World, from: usize) -> Vec<Value> {
    w.events()
        .into_iter()
        .skip(from)
        .filter(|e| e["agent_id"] == "yinyue")
        .filter(|e| !e["kind"].as_str().unwrap_or("").starts_with("pet_"))
        .collect()
}

/// A turn the engine started for her (a note come due) that ends without
/// words sends nothing a thread draws: no status, no run, no token count.
#[tokio::test]
async fn a_silent_turn_of_hers_shows_nothing_on_any_page() {
    let w = World::start(crate::notes::script(text("He is away. Nothing to say."))).await;
    she_writes_the_note(&w).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let from = w.events().len();
    let mark = w.mark();
    the_clock_moves(&w);
    w.wait_agent(Agent::Yinyue, mark).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let shown = her_thread_events(&w, from);
    let kinds: Vec<String> = shown
        .iter()
        .map(|e| format!("{} {}", e["kind"], e["phase"]))
        .collect();
    assert!(
        shown.is_empty(),
        "a silent turn of hers reached a page: {kinds:?}"
    );
}

/// Her thread as a page loads it: the rows she wrote, each `[meta, text]`.
async fn her_rows_on_load(w: &World) -> Vec<Value> {
    let sid = w
        .events()
        .iter()
        .rev()
        .filter(|e| e["agent_id"] == "yinyue")
        .find_map(|e| e["session_id"].as_str().map(str::to_string))
        .expect("her session");
    let url = format!(
        "/api/workspace/state?session_id={sid}&project_root={}",
        urlencoding::encode(&w.work())
    );
    let page = w.api.get(&url).await;
    page["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r[0]["from"] == "yinyue")
        .collect()
}

/// After a reload too: a held turn of hers that used a tool and said
/// nothing leaves no row — not its call, not an ended run.
#[tokio::test]
async fn a_silent_turn_that_used_a_tool_shows_nothing_after_a_reload() {
    let setup = Setup::default().script(
        Agent::Yinyue,
        vec![
            tool(
                "note_write",
                serde_json::json!({"text": "the parcel arrives tomorrow", "look_again": "+40m"}),
            ),
            text("Noted."),
            tool("recent_activity", serde_json::json!({})),
            text("Nothing new. No line."),
        ],
    );
    let w = World::start(setup).await;
    she_writes_the_note(&w).await;
    let before = her_rows_on_load(&w).await;
    let mark = w.mark();
    the_clock_moves(&w);
    w.wait_agent(Agent::Yinyue, mark).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let after = her_rows_on_load(&w).await;
    assert_eq!(after, before, "a silent turn of hers left a row on reload");
}

/// Every row of every session's thread, as one text.
fn every_row(w: &World) -> String {
    let dir = w.home.linggen_home().join("sessions");
    let mut all = String::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        all.push_str(
            &std::fs::read_to_string(entry.path().join("messages.jsonl")).unwrap_or_default(),
        );
    }
    all
}

/// She is met (a word with her), then the person steps away: the only
/// surface that beat is long idle and unfocused.
async fn meet_then_away(w: &World) {
    let mark = w.mark();
    w.api
        .post("/api/yinyue/chat", serde_json::json!({"text": "Hello"}))
        .await;
    w.wait_agent(Agent::Yinyue, mark).await;
    // The fixture's voice is off; a notice is marked only when said aloud.
    assert_eq!(
        w.api
            .post("/api/yinyue/chat", serde_json::json!({"text": "/unmute"}))
            .await,
        serde_json::json!("ok")
    );
    away(w).await;
}

async fn away(w: &World) {
    let away = serde_json::json!({"surface": "user-turn", "focused": false, "typing": false, "idle_ms": 600_000});
    w.api.post("/api/presence", away).await;
}

/// Her calls after the greeting whose kickoff says `about`.
fn her_wakes(w: &World, about: &str) -> usize {
    w.model
        .calls_of(Agent::Yinyue)
        .iter()
        .skip(1)
        .filter(|c| {
            c.thread()
                .last()
                .is_some_and(|(_, said)| said.contains(about))
        })
        .count()
}

async fn until(what: &str, w: &World, ok: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(40);
    while !ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "{what} never came; log:\n{}",
            w.engine.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

const READY_LINE: &str = "READY-LINE your answer is waiting.";

/// A reply finished while the person was away: she says so aloud, and the
/// thread gets no row of hers for it. No surface holds her here, so nobody
/// heard it: nothing is marked, live or on reload (a page that holds her
/// sees the mark — `tests/e2e/specs/heralds.spec.ts`).
#[tokio::test]
async fn a_notice_no_surface_heard_is_no_row_and_no_mark() {
    let setup = Setup::default()
        .script(
            Agent::Ling,
            vec![crate::support::model::slow(text(
                "Here is the plan for the shelf.",
            ))],
        )
        .script(
            Agent::Yinyue,
            vec![text("I'm here."), crate::support::say(READY_LINE)],
        );
    let w = World::start(setup).await;
    meet_then_away(&w).await;
    let sid = w
        .api
        .create_session(serde_json::json!({"title": "Shelf"}))
        .await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "Plan the shelf").await;
    away(&w).await;
    w.finish_run(&sid, Agent::Ling, mark).await;
    let spoke = |e: &Value| e["kind"] == "pet_speak" && e["text"] == READY_LINE;
    until("her notice", &w, || w.events().iter().any(spoke)).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let cue = w.events().into_iter().find(|e| spoke(e)).unwrap();
    assert_eq!(cue["data"]["voice"], true, "said aloud: {cue}");
    assert_eq!(
        w.events().iter().filter(|e| spoke(e)).count(),
        1,
        "said once"
    );
    assert!(
        !every_row(&w).contains("READY-LINE"),
        "the notice became a row"
    );
    assert!(
        !w.events().iter().any(|e| e["kind"] == "notice_told"),
        "marked though no surface heard it"
    );
    let url = format!(
        "/api/workspace/state?session_id={sid}&project_root={}",
        urlencoding::encode(&w.work())
    );
    let page = w.api.get(&url).await;
    assert!(!page.to_string().contains("told_aloud_at"), "{page}");
}

/// Her voice muted: the notice is words only, and a notice never said aloud
/// leaves no mark.
#[tokio::test]
async fn a_notice_not_said_aloud_leaves_no_mark() {
    let setup = Setup::default()
        .script(
            Agent::Ling,
            vec![crate::support::model::slow(text(
                "Here is the plan for the shelf.",
            ))],
        )
        .script(
            Agent::Yinyue,
            vec![text("I'm here."), crate::support::say(READY_LINE)],
        );
    let w = World::start(setup).await;
    meet_then_away(&w).await;
    assert_eq!(
        w.api
            .post("/api/yinyue/chat", serde_json::json!({"text": "/mute"}))
            .await,
        serde_json::json!("ok")
    );
    let sid = w
        .api
        .create_session(serde_json::json!({"title": "Shelf"}))
        .await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "Plan the shelf").await;
    away(&w).await;
    w.finish_run(&sid, Agent::Ling, mark).await;
    let spoke = |e: &Value| e["kind"] == "pet_speak" && e["text"] == READY_LINE;
    until("her notice", &w, || w.events().iter().any(spoke)).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        !w.events().iter().any(|e| e["kind"] == "notice_told"),
        "a mark for a line not said aloud"
    );
    let url = format!(
        "/api/workspace/state?session_id={sid}&project_root={}",
        urlencoding::encode(&w.work())
    );
    let page = w.api.get(&url).await;
    assert!(!page.to_string().contains("told_aloud_at"), "{page}");
    assert!(
        !every_row(&w).contains("READY-LINE"),
        "the notice became a row"
    );
}

/// A run in a session another tool started (`agent_run`) is never told —
/// not its failure, not its reply — also when a person types into it.
#[tokio::test]
async fn an_external_run_is_never_told() {
    let setup = Setup::default()
        .script(
            Agent::Ling,
            vec![
                text("Done."),
                crate::support::error(400, "the provider refused"),
                crate::support::model::slow(text("Again, from the top.")),
            ],
        )
        .script(
            Agent::Yinyue,
            vec![
                text("I'm here."),
                crate::support::say("EXTERNAL-LINE"),
                crate::support::say("EXTERNAL-LINE"),
            ],
        );
    let w = World::start(setup).await;
    meet_then_away(&w).await;
    let call = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "agent_run", "arguments": {"prompt": "Say done."}}});
    let done = w.api.post("/mcp", call).await;
    assert!(done.to_string().contains("Done."), "{done}");
    let sid = crate::delegate::delegate_session(&w);
    // Typed into it: the run fails, then one finishes while the person is away.
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "Try it").await;
    away(&w).await;
    w.finish_run(&sid, Agent::Ling, mark).await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "Once more").await;
    away(&w).await;
    w.finish_run(&sid, Agent::Ling, mark).await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    let woken = w.model.calls_of(Agent::Yinyue).len() - 1;
    assert_eq!(
        woken,
        0,
        "an external run was told to the person; log:\n{}",
        w.engine.log()
    );
    assert!(!w
        .events()
        .iter()
        .any(|e| e["kind"] == "pet_speak" && e["text"] == "EXTERNAL-LINE"));
}

/// A run waiting on a permission in a session another tool started is not
/// told either: its caller answers its own prompts.
#[tokio::test]
async fn an_external_runs_wait_is_never_told() {
    let setup = Setup::default()
        .script(
            Agent::Ling,
            vec![
                text("Done."),
                tool(
                    "Write",
                    serde_json::json!({"path": "~/outside.txt", "content": "x"}),
                ),
                text("Could not."),
            ],
        )
        .script(
            Agent::Yinyue,
            vec![text("I'm here."), crate::support::say("EXTERNAL-WAIT")],
        );
    let w = World::start(setup).await;
    meet_then_away(&w).await;
    let inner = format!("{}/inner", w.work());
    std::fs::create_dir_all(&inner).unwrap();
    let call = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "agent_run", "arguments": {"prompt": "Say done.", "write_dir": inner}}});
    let done = w.api.post("/mcp", call).await;
    assert!(done.to_string().contains("Done."), "{done}");
    let sid = crate::delegate::delegate_session(&w);
    w.api.chat(&sid, &inner, "Write outside").await;
    away(&w).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let ask = loop {
        if let Some(ask) = w.api.pending(&sid, None).await.into_iter().next() {
            break ask;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no wait in the external session; log:\n{}",
            w.engine.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(
        w.model.calls_of(Agent::Yinyue).len() - 1,
        0,
        "an external wait was told"
    );
    assert!(ask.get("told_aloud_at").is_none(), "{ask}");
    w.api.answer(&ask, "Deny").await;
}

/// Ling's script for a turn tree with two questions waiting at once: the
/// chat's first call starts a subagent beside itself (`Task`, in the
/// background), then the chat and the subagent each ask to write outside
/// the folder. The fake model answers by call order, so which of the two
/// takes which `Write` is not told — both are questions of the one turn.
/// The calls after those are what each says once its question is answered.
fn two_questions_of_one_turn() -> Vec<crate::support::model::Reply> {
    vec![
        tool(
            "Task",
            serde_json::json!({"target_agent_id": "ling", "run_in_background": true,
                "task": "Write outside the folder"}),
        ),
        tool(
            "Write",
            serde_json::json!({"path": "~/outside-1.txt", "content": "x"}),
        ),
        tool(
            "Write",
            serde_json::json!({"path": "~/outside-2.txt", "content": "x"}),
        ),
        text("Could not."),
        text("Could not either."),
        text("The helper could not."),
    ]
}

/// Every question waiting now, in any session.
async fn every_ask(w: &World) -> Vec<Value> {
    w.api
        .get("/api/pending-ask-user")
        .await
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Wait until `n` questions wait.
async fn until_asks(w: &World, n: usize) -> Vec<Value> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let now = every_ask(w).await;
        if now.len() == n {
            return now;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{n} asks never waited, {} did; log:\n{}",
            now.len(),
            w.engine.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The question ids her wake-up kickoffs named, in order. A kickoff for a
/// wait carries the id her `answer_prompt` is to relay to.
fn questions_she_was_woken_for(w: &World, asks: &[Value]) -> Vec<String> {
    w.model
        .calls_of(Agent::Yinyue)
        .iter()
        .skip(1)
        .filter_map(|c| c.thread().last().map(|(_, said)| said.clone()))
        .filter(|said| said.contains("waiting on the user"))
        .map(|said| {
            asks.iter()
                .map(|a| a["question_id"].as_str().unwrap_or_default().to_string())
                .find(|id| said.contains(id.as_str()))
                .unwrap_or_default()
        })
        .collect()
}

async fn answer_all(w: &World) {
    for ask in every_ask(w).await {
        w.api.answer(&ask, "Deny").await;
    }
}

/// A subagent's question waits beside its parent's, the person away: one
/// telling for the turn (the key is the turn tree's root run, not the
/// question nor the session). The cue names one of the waiting questions;
/// both cards carry the mark.
#[tokio::test]
async fn one_telling_per_waiting_turn() {
    let setup = Setup::default()
        .script(Agent::Ling, two_questions_of_one_turn())
        .script(
            Agent::Yinyue,
            vec![
                text("I'm here."),
                crate::support::say("WAIT-LINE they need you."),
                crate::support::say("SECOND-TELLING"),
            ],
        );
    let w = World::start(setup).await;
    meet_then_away(&w).await;
    let sid = w
        .api
        .create_session(serde_json::json!({"title": "Outside"}))
        .await;
    w.api.chat(&sid, &w.work(), "Write outside, twice").await;
    away(&w).await;
    let pending = until_asks(&w, 2).await;
    // The prop is what it claims: a second run, the first one's child.
    assert!(
        w.engine.log().contains("parent_run_id=Some("),
        "no subagent run began; log:\n{}",
        w.engine.log()
    );
    let cue = |e: &Value| e["kind"] == "pet_speak" && e["text"] == "WAIT-LINE they need you.";
    until("her notice", &w, || w.events().iter().any(cue)).await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(
        her_wakes(&w, "waiting on the user"),
        1,
        "one waiting turn, told more than once"
    );
    assert!(!w.events().iter().any(|e| e["text"] == "SECOND-TELLING"));
    let said = w.events().into_iter().find(|e| cue(e)).unwrap();
    let ids: Vec<&Value> = pending.iter().map(|a| &a["question_id"]).collect();
    assert!(
        ids.contains(&&said["data"]["question_id"]),
        "the cue names a waiting question: {said}"
    );
    assert!(
        !every_row(&w).contains("WAIT-LINE"),
        "the notice became a row"
    );
    answer_all(&w).await;
}

/// Two questions of one turn wait; the first telling comes to nothing (she
/// writes no line): the other, still waiting, is told — the one not tried.
#[tokio::test]
async fn a_question_folded_into_a_telling_that_came_to_nothing_is_told() {
    let setup = Setup::default()
        .script(Agent::Ling, two_questions_of_one_turn())
        .script(
            Agent::Yinyue,
            vec![
                text("I'm here."),
                // Slow: the second question comes while this telling still runs.
                crate::support::model::slow(text("He is away, so no line from me now.")),
                crate::support::say("SECOND-TOLD they need you."),
            ],
        );
    let w = World::start(setup).await;
    meet_then_away(&w).await;
    let sid = w
        .api
        .create_session(serde_json::json!({"title": "Outside"}))
        .await;
    w.api.chat(&sid, &w.work(), "Write outside, twice").await;
    away(&w).await;
    let pending = until_asks(&w, 2).await;
    let cue = |e: &Value| e["kind"] == "pet_speak" && e["text"] == "SECOND-TOLD they need you.";
    until("the folded question's telling", &w, || {
        w.events().iter().any(cue)
    })
    .await;
    let said = w.events().into_iter().find(|e| cue(e)).unwrap();
    assert_eq!(her_wakes(&w, "waiting on the user"), 2);
    let woken = questions_she_was_woken_for(&w, &pending);
    assert_eq!(woken.len(), 2, "{woken:?}");
    assert_ne!(woken[0], woken[1], "the same question told twice");
    assert_eq!(said["data"]["question_id"], woken[1], "{said}");
    answer_all(&w).await;
}

/// Two turns of two different sessions wait at once: two waits, each told.
#[tokio::test]
async fn waits_of_two_sessions_are_each_told() {
    let setup = Setup::default()
        .script(
            Agent::Ling,
            vec![
                tool(
                    "Write",
                    serde_json::json!({"path": "~/outside-1.txt", "content": "x"}),
                ),
                tool(
                    "Write",
                    serde_json::json!({"path": "~/outside-2.txt", "content": "x"}),
                ),
                text("Could not."),
                text("Could not either."),
            ],
        )
        .script(
            Agent::Yinyue,
            vec![
                text("I'm here."),
                crate::support::say("FIRST-WAIT they need you."),
                crate::support::say("OTHER-WAIT they need you."),
            ],
        );
    let w = World::start(setup).await;
    meet_then_away(&w).await;
    for title in ["Shelf", "Lamp"] {
        let sid = w
            .api
            .create_session(serde_json::json!({"title": title}))
            .await;
        w.api.chat(&sid, &w.work(), "Write outside").await;
    }
    away(&w).await;
    let pending = until_asks(&w, 2).await;
    assert_ne!(pending[0]["session_id"], pending[1]["session_id"]);
    let told = |events: &[Value], line: &str| {
        events
            .iter()
            .any(|e| e["kind"] == "pet_speak" && e["text"] == line)
    };
    until("both waits told", &w, || {
        let events = w.events();
        told(&events, "FIRST-WAIT they need you.") && told(&events, "OTHER-WAIT they need you.")
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(her_wakes(&w, "waiting on the user"), 2);
    let woken = questions_she_was_woken_for(&w, &pending);
    assert_ne!(woken[0], woken[1], "one wait told twice: {woken:?}");
    answer_all(&w).await;
}

/// The wait is answered while her line is being made: nothing of it is
/// said, marked or written — then or later.
#[tokio::test]
async fn a_wait_answered_while_her_line_is_made_leaves_nothing() {
    let setup = Setup::default()
        .script(
            Agent::Ling,
            vec![
                tool(
                    "Write",
                    serde_json::json!({"path": "~/outside.txt", "content": "x"}),
                ),
                text("Could not."),
            ],
        )
        .script(
            Agent::Yinyue,
            vec![
                text("I'm here."),
                crate::support::model::slow(crate::support::say("LATE-WAIT they need you.")),
            ],
        );
    let w = World::start(setup).await;
    meet_then_away(&w).await;
    let sid = w
        .api
        .create_session(serde_json::json!({"title": "Outside"}))
        .await;
    w.api.chat(&sid, &w.work(), "Write outside").await;
    away(&w).await;
    until("her notice's turn", &w, || {
        her_wakes(&w, "waiting on the user") == 1
    })
    .await;
    let ask = w
        .api
        .pending(&sid, None)
        .await
        .into_iter()
        .next()
        .expect("the wait");
    w.api.answer(&ask, "Deny").await;
    // Past her slow line, and a while after.
    tokio::time::sleep(Duration::from_secs(10)).await;
    let late = |e: &Value| e.to_string().contains("LATE-WAIT");
    assert!(
        !w.events()
            .iter()
            .any(|e| e["kind"] == "pet_speak" && late(e)),
        "a stale notice was said"
    );
    assert!(
        !w.events().iter().any(|e| e["kind"] == "notice_told"),
        "a stale notice was marked"
    );
    assert!(
        !every_row(&w).contains("LATE-WAIT"),
        "a row about the wait was written after it ended"
    );
}

/// A fact of hers held for the person (her own failed turn) does not ride a
/// notice, which leaves no row: it waits, and her next woken turn that is
/// kept (a mission's fault) carries it.
#[tokio::test]
async fn her_held_fact_is_not_used_up_by_a_notice() {
    let setup = Setup::default()
        .script(
            Agent::Ling,
            vec![
                crate::support::model::slow(text("Here is the plan for the shelf.")),
                tool("Ping", serde_json::json!({})),
                text("The lamp check did not complete. DONE"),
            ],
        )
        .script(
            Agent::Yinyue,
            vec![
                text("I'm here."),
                crate::support::error(400, "her model refused"),
                crate::support::say(READY_LINE),
                crate::support::say("FAULT-LINE the beacon failed."),
            ],
        );
    let w = World::start(setup).await;
    crate::missions::install_beacon(&w.home.linggen_home().join("skills"));
    w.api
        .post("/api/skills/reload", serde_json::json!({}))
        .await;
    meet_then_away(&w).await;
    // Her own turn fails: the fact is held for her.
    let mark = w.mark();
    w.api
        .post(
            "/api/yinyue/chat",
            serde_json::json!({"text": "Still there?"}),
        )
        .await;
    w.wait_agent(Agent::Yinyue, mark).await;
    away(&w).await;
    let sid = w
        .api
        .create_session(serde_json::json!({"title": "Shelf"}))
        .await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "Plan the shelf").await;
    away(&w).await;
    w.finish_run(&sid, Agent::Ling, mark).await;
    until("her notice", &w, || {
        w.events()
            .iter()
            .any(|e| e["kind"] == "pet_speak" && e["text"] == READY_LINE)
    })
    .await;
    let calls = w.model.calls_of(Agent::Yinyue);
    let notice = calls.last().unwrap().text();
    assert!(notice.contains("just finished a reply"), "{notice}");
    assert!(
        !notice.contains("one of your own turns"),
        "her held fact rode a notice: {notice}"
    );
    // A mission's fault, someone back: the held fact rides that turn.
    crate::missions::set_exit(&w, 3);
    crate::missions::beat_present(&w).await;
    let mark = w.mark();
    crate::missions::run_once(&w).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let fault = w.model.calls_of(Agent::Yinyue).last().unwrap().text();
    assert!(
        fault.contains("one of your own turns"),
        "her held fact was lost: {fault}"
    );
}
