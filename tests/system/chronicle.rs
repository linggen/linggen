//! The chronicle (Memory V2, `doc/memory-spec.md`): the built-in monthly
//! mission writes `chronicle/<yyyy>/<yyyy-mm>.md` (and the year's file)
//! through the one note writer, from sources the engine only reads; with v2
//! off, or nothing waiting, the run ends with no model call. And Yinyue's
//! prompt has the day's dates under v2, which no other prompt has. Notes are
//! made-up fixtures in the world's own home.

use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const V2: Option<&str> = Some("\n[memory]\nlinggen = \"v2\"\n");

/// A person note made long ago: the one month (2024-03) and year (2024) waiting.
const OLD_NOTE: &str = "---\ntopic: Work\nscope: person\n---\n\n## Chose Rust for the engine\n<!-- id: c000000001 · kind: decision · voice: user · created: 2024-03-05 · updated: 2024-03-05 · source: linggen -->\nWhy: one binary.\n";

fn seed_old(home: &crate::support::home::Home) {
    // The fixture home's own sessions are not this test's months.
    let _ = std::fs::remove_dir_all(home.linggen_home().join("sessions"));
    let path = home.linggen_home().join("memory/person/work.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, OLD_NOTE).unwrap();
}

fn line(topic: &str, summary: &str) -> Value {
    json!({"op": "write", "scope": "chronicle", "topic": topic, "kind": "chronicle", "summary": summary, "body": "Source: note c000000001"})
}

async fn start_run(w: &World) -> String {
    let resp = w.api.post("/api/missions/chronicle/trigger", json!({})).await;
    resp["session_id"].as_str().unwrap_or_else(|| panic!("no session: {resp}")).to_string()
}

/// The run in `sid`, once it is on record as finished.
async fn finished_run(w: &World, sid: &str) -> Value {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        let runs = w.api.get("/api/missions/chronicle/runs").await;
        let newest = runs["runs"][0].clone();
        if newest["session_id"] == sid && newest["status"] != "running" {
            return newest;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("the chronicle run never finished:\n{}", w.engine.log());
}

/// The next run on record after `previous` (a run id, or none yet), once it
/// is finished. A run that found nothing to do is skipped and has no session,
/// so it cannot be found by one.
async fn next_finished_run(w: &World, previous: Option<&str>) -> Value {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        let runs = w.api.get("/api/missions/chronicle/runs").await;
        let newest = runs["runs"][0].clone();
        if !newest.is_null() && newest["run_id"].as_str() != previous && newest["status"] != "running" {
            return newest;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("the chronicle run never finished:\n{}", w.engine.log());
}

/// A run that had nothing to do: skipped as a `prepare` skip, no model call,
/// no session left behind (the manual trigger made one first).
fn assert_nothing_to_do(w: &World, run: &Value, sid: &str) {
    assert_eq!(run["status"], "skipped", "{run}");
    assert_eq!(run["skip"], "prepare", "{run}");
    assert!(run.get("session_id").is_none() && run.get("failure_count").is_none(), "{run}");
    assert!(!w.home.linggen_home().join("sessions").join(sid).exists(), "an empty session was left");
}

/// The month and the year are written through the writer into their files,
/// signed by the engine, from the input the engine prepared; the writer's
/// fence lets the run write the chronicle and nothing else.
#[tokio::test]
async fn the_monthly_run_writes_the_month_and_the_year_through_the_writer() {
    let year_line = line("2024", "2024 The year Linggen began");
    let elsewhere = json!({"op": "write", "scope": "person", "topic": "x", "summary": "A fact", "kind": "person"});
    let w = World::start_with_home(
        Setup { config_append: V2, ..Setup::default() }.script(
            Agent::Chronicler,
            vec![
                tool("memory_notes", line("2024-03", "2024-03-05 Chose Rust for the engine")),
                text("CHRONICLE months=[2024-03] years=[2024]\nWROTE 2 2024-03 lines=1"),
                tool("memory_notes", year_line),
                tool("memory_notes", elsewhere),
                text("WROTE 3 2024 lines=1"),
                text("DONE"),
            ],
        ),
        seed_old,
    )
    .await;
    let sid = start_run(&w).await;
    let run = finished_run(&w, &sid).await;
    assert_eq!(run["status"], "completed", "{run}");
    let home = w.home.linggen_home();
    let input = std::fs::read_to_string(home.join("memory/.chronicle-input.jsonl")).expect("the engine's input");
    assert!(input.contains("Chose Rust for the engine") && input.lines().next().unwrap().contains("\"months\":[\"2024-03\"]"), "{input}");
    let month = std::fs::read_to_string(home.join("memory/chronicle/2024/2024-03.md")).expect("the month's file");
    assert!(month.contains("## 2024-03-05 Chose Rust for the engine") && month.contains("kind: chronicle") && month.contains("source: linggen"), "{month}");
    assert!(std::fs::read_to_string(home.join("memory/chronicle/2024/2024.md")).unwrap().contains("The year Linggen began"));
    // The one thing it tried besides the chronicle was refused, and not written.
    assert!(!home.join("memory/person/x.md").exists());
    let first = &w.model.calls_of(Agent::Chronicler)[0];
    let tools = first.body.to_string();
    assert!(tools.contains("memory_notes") && !tools.contains("mcp__memory__"), "the run's tools: {tools}");
    w.assert_all_scripted();

    // Written, the months are not offered again: nothing to do, no model call.
    let before = w.model.calls_of(Agent::Chronicler).len();
    let first_run = run["run_id"].as_str().unwrap().to_string();
    let again = start_run(&w).await;
    let run = next_finished_run(&w, Some(&first_run)).await;
    assert_nothing_to_do(&w, &run, &again);
    assert_eq!(w.model.calls_of(Agent::Chronicler).len(), before, "a model call for nothing");
}

/// v1 is the default: the monthly run ends at once, quietly, with no model
/// call, no file and no session.
#[tokio::test]
async fn under_v1_the_run_does_nothing() {
    let w = World::start_with_home(Setup::default(), seed_old).await;
    let sid = start_run(&w).await;
    let run = next_finished_run(&w, None).await;
    assert_nothing_to_do(&w, &run, &sid);
    assert!(w.model.calls_of(Agent::Chronicler).is_empty());
    assert!(!w.home.linggen_home().join("memory/chronicle").exists());
}

/// Yinyue's prompt has today's dates under v2; Ling's never does, and under
/// v1 hers has none.
#[tokio::test]
async fn only_yinyues_prompt_has_the_days_dates_and_only_under_v2() {
    // The world's engine runs with TZ=UTC (support/process.rs): its today is
    // UTC's, which is not this machine's local day in the evening.
    let today = chrono::Utc::now().format("%m-%d").to_string();
    let note = format!(
        "---\ntopic: Days\nscope: person\n---\n\n## Our anniversary\n<!-- id: d000000001 · kind: person · voice: user · created: 2026-01-01 · updated: 2026-01-01 · source: yinyue · yearly: {today} -->\n"
    );
    let seed = move |home: &crate::support::home::Home| {
        let path = home.linggen_home().join("memory/person/days.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &note).unwrap();
    };
    let w = World::start_with_home(Setup { config_append: V2, ..Setup::default() }, seed.clone()).await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    let hers = w.api.export(&sid, "yinyue", "").await;
    let prompt = hers["system_prompt"].as_str().unwrap();
    assert!(prompt.contains("today · Our anniversary") && prompt.contains("never bring up yourself"), "{prompt}");
    let ling = w.api.export(&sid, "ling", &w.work()).await;
    // The note itself is a fact about the person, which Ling has too; her day's line is not.
    assert!(!ling["system_prompt"].as_str().unwrap().contains("today · Our anniversary"));

    let v1 = World::start_with_home(Setup::default(), seed).await;
    let sid = v1.api.create_session(json!({"title": "Chat"})).await;
    let hers = v1.api.export(&sid, "yinyue", "").await;
    assert!(!hers["system_prompt"].as_str().unwrap().contains("Our anniversary"));
}
