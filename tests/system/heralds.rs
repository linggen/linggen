//! What Yinyue tells a person about Ling's work (doc/yinyue-companion-spec.md
//! § Herald): one notice per thing, told by voice only while the person is
//! away, never a row of its own; the thing it is about carries a folded mark;
//! one telling per waiting session; a run another tool started is never told;
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
/// thread gets no row of hers for it — the reply's own row carries the
/// folded mark, live and on reload.
#[tokio::test]
async fn a_notice_is_never_a_row_of_its_own_and_marks_the_reply() {
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
    let told = w
        .events_when(|e| e.iter().any(|x| x["kind"] == "notice_told"))
        .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        w.events().iter().filter(|e| spoke(e)).count(),
        1,
        "said once"
    );
    assert!(
        !every_row(&w).contains("READY-LINE"),
        "the notice became a row"
    );
    let mark_event = told
        .iter()
        .find(|x| x["kind"] == "notice_told")
        .expect("a live mark");
    assert_eq!(mark_event["session_id"], sid.as_str(), "{mark_event}");
    assert_eq!(mark_event["data"]["line"], READY_LINE);
    // On reload: Ling's reply row carries the mark, once.
    let url = format!(
        "/api/workspace/state?session_id={sid}&project_root={}",
        urlencoding::encode(&w.work())
    );
    let page = w.api.get(&url).await;
    let marked: Vec<&Value> = page["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r[0].get("told_aloud_at").is_some())
        .collect();
    assert_eq!(marked.len(), 1, "{page}");
    assert_eq!(marked[0][0]["from"], "ling");
    assert_eq!(marked[0][0]["told_aloud_line"], READY_LINE);
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

/// Two permission prompts waiting in one session, 2 s apart, while the
/// person is away: one telling. The cue names the first question, so her
/// bubble carries its card; both cards carry the mark.
#[tokio::test]
async fn one_telling_per_waiting_session() {
    let setup = Setup::default().script(
        Agent::Yinyue,
        vec![
            text("I'm here."),
            crate::support::say("WAIT-LINE they need you."),
            crate::support::say("SECOND-TELLING"),
        ],
    );
    let w = crate::claude_bridge::world_with(setup).await;
    let sid = crate::claude_bridge::session_in(&w, &w.work()).await;
    meet_then_away(&w).await;
    w.api.chat(&sid, &w.work(), "asktwice").await;
    away(&w).await;
    let asks = || async {
        w.api
            .get("/api/pending-ask-user")
            .await
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let pending = loop {
        let now = asks().await;
        if now.len() == 2 {
            break now;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "two asks never waited; log:\n{}",
            w.engine.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let cue = |e: &Value| e["kind"] == "pet_speak" && e["text"] == "WAIT-LINE they need you.";
    until("her notice", &w, || w.events().iter().any(cue)).await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(
        her_wakes(&w, "waiting on the user"),
        1,
        "one waiting session, told more than once"
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
    for ask in asks().await {
        assert!(
            ask.get("told_aloud_at").is_some(),
            "the card carries no mark: {ask}"
        );
    }
    for ask in asks().await {
        w.api.answer(&ask, "Allow once").await;
    }
    // The wait is over: nothing about it is written late.
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        !every_row(&w).contains("WAIT-LINE"),
        "a row about the wait was written after it ended"
    );
}
