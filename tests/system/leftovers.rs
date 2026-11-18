//! What one turn sets for itself never reaches a later turn on the same
//! engine (doc/session-spec.md § How a turn is set up).

use crate::day_session::{day_world_at, turn_at};
use crate::support::model::{Agent, Call};
use crate::support::{text, tool, Setup, World};
use serde_json::json;
use std::path::Path;

const MISSION: &str = "tally:count";
const LEFTOVER: &str = "LEFTOVER-BRANCH: count again.";

/// A made-up skill with a mission whose `kickoff-then` branch waits for a
/// reply of exactly `MORE`, which the run itself never gives.
fn install_tally(skills: &Path) {
    let dir = skills.join("tally");
    std::fs::create_dir_all(dir.join("missions/count")).expect("tally dirs");
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: tally\ndescription: A test skill — counts.\n\
         cwd: ~/.linggen/skills/tally\n---\n\nCount.\n",
    )
    .expect("SKILL.md");
    std::fs::write(
        dir.join("missions/count/mission.md"),
        format!(
            "---\nname: Tally count\nschedule: '0 3 * * *'\nenabled: false\nagent: ling\n\
             cwd: ~/.linggen/skills/tally\nkickoff:\n  - \"Count once.\"\n\
             kickoff-then:\n  MORE:\n    - \"{LEFTOVER}\"\n---\n\nCount once, in one line.\n"
        ),
    )
    .expect("mission.md");
}

/// A mission run's kickoff belongs to that run. The run ends with its
/// branch unfired; the person's next turn in the
/// session answers `MORE`, and nothing of the mission's kickoff follows.
#[tokio::test]
async fn a_finished_runs_kickoff_never_reaches_a_persons_turn() {
    let ling = vec![text("Counted: one."), text("MORE")];
    let w = World::start(Setup::default().script(Agent::Ling, ling)).await;
    install_tally(&w.home.linggen_home().join("skills"));
    w.api.post("/api/skills/reload", json!({})).await;

    let mark = w.mark();
    let path = format!("/api/missions/{}/trigger", urlencoding::encode(MISSION));
    let resp = w.api.post(&path, json!({})).await;
    let sid = resp["session_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no session: {resp}"))
        .to_string();
    w.finish_run(&sid, Agent::Ling, mark).await;

    w.turn(&sid, &w.work(), "Anything else?", Agent::Ling).await;
    let leaked: Vec<_> = w
        .raw_rows(&sid)
        .into_iter()
        .filter(|r| r["content"].as_str().is_some_and(|c| c.contains("LEFTOVER-BRANCH")))
        .collect();
    assert!(leaked.is_empty(), "the run's kickoff reached the chat: {leaked:#?}");
    assert_eq!(w.model.calls_of(Agent::Ling).len(), 2);
    w.assert_all_scripted();
}

/// Her moment at an app's table is sealed: it reaches no other agent. A
/// message Ling then hands her in the same day is a turn of its own, open
/// as any delivery: nothing of the moment's seal is left on it.
#[tokio::test]
async fn a_delivery_after_her_sealed_moment_is_not_sealed() {
    let setup = Setup::default()
        .script(Agent::Yinyue, vec![text("Clouds over the harbor."), text("Noted.")])
        .script(
            Agent::Ling,
            vec![
                tool("agent_chat", json!({"to": "yinyue", "message": "The dice are back."})),
                text("Told her."),
            ],
        );
    let (w, _) = day_world_at(12, setup).await;
    let day = w.api.get("/api/sessions/day").await["id"].as_str().unwrap().to_string();

    let mark = w.mark();
    let moment = json!({"app": "dice", "text": "The player looks at the sky.", "asked": true});
    assert_eq!(w.api.post("/api/yinyue/event", moment).await, json!("ok"));
    w.finish_run(&day, Agent::Yinyue, mark).await;

    let mark = w.mark();
    turn_at(&w, &day, None, "Tell her the dice are back", Agent::Ling).await;
    w.finish_run(&day, Agent::Yinyue, mark).await;
    let delivered = w.model.calls_of(Agent::Yinyue).pop().expect("her delivery's call");
    assert!(
        offers(&delivered, "agent_chat"),
        "the moment's seal reached the delivery: {:?}",
        delivered.tool_names()
    );
    w.assert_all_scripted();
}

fn offers(call: &Call, tool: &str) -> bool {
    call.tool_names().iter().any(|t| t == tool)
}

/// An approved plan's run is a turn of its own. The turn that made the plan
/// left its last words (the plan's text) on the engine; a run that ends
/// with no words of its own (the model answers nothing) shows nothing, not
/// those.
#[tokio::test]
async fn an_approved_plans_run_never_shows_the_last_turns_words() {
    const SID: &str = "sess-1790000002-c0ac7ed2";
    let plan = "## Plan\n\n1. PLAN-WORDS: add a flag";
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![
            tool("EnterPlanMode", json!({"reason": "plan the flag"})),
            tool("ExitPlanMode", json!({"plan_text": plan})),
            text(""),
            text(""),
            text(""),
        ],
    ))
    .await;
    let mark = w.mark();
    w.api.chat(SID, &w.work(), "Add a flag, plan it first.").await;
    w.finish_run(SID, Agent::Ling, mark).await;
    // EnterPlanMode ends the first run, not the chat turn. Wait until
    // planning has submitted the plan and the whole turn has closed.
    let turn_end = |event: &serde_json::Value| {
        event["kind"] == "turn_complete" && event["session_id"] == SID
    };
    let events = w.events_when(|events| events.iter().any(turn_end)).await;
    let made = events.iter().position(turn_end).expect("the planning turn closed");
    assert!(events[..made].iter().any(|e| {
        e["kind"] == "message"
            && e["session_id"] == SID
            && e["text"].as_str().is_some_and(|t| t.contains("PLAN-WORDS"))
    }), "the planning turn never showed its words");

    let mark = w.mark();
    let action = json!({"project_root": w.work(), "agent_id": "ling", "session_id": SID});
    w.api.post("/api/plan/approve", action).await;
    w.finish_run(SID, Agent::Ling, mark).await;
    let after_plan = made + 1;
    let events = w.events_when(|events| events.iter().skip(after_plan).any(turn_end)).await;
    assert!(events.iter().skip(after_plan).any(turn_end), "the approved turn never closed");
    let shown: Vec<_> = events[after_plan..]
        .iter()
        .filter(|e| e["kind"] == "message" && e["text"].as_str().is_some_and(|t| t.contains("PLAN-WORDS")))
        .collect();
    assert!(shown.is_empty(), "the plan's turn's words were shown again: {shown:#?}");
    w.assert_all_scripted();
}
