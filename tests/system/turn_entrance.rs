//! What each door's turn sees and runs on today, where the doors set a
//! session turn up differently (R4, stage 1 — `AgentEngine::open_session_turn`).
//! Each test pins what a person's session gets NOW; none says it is right.
//! Stage 2 decides each (a fault, fixed and named in the changelog, or meant,
//! with a comment at the site) and changes the pin with it.
//!
//! a. A task's end, an `agent_chat` delivery and an approved plan's run never
//!    call `read_live_state`. What they see is the same as a typed turn's all
//!    the same: the loop itself reads a changed skill list and settings when
//!    it starts (`initialize_loop`). What they do not see changed is what a
//!    turn reads BEFORE the loop (the thread's compaction threshold, the
//!    model list) — pinned at the function, `open_session_turn`'s tests.
//! b. A turn told to an agent that is hers (or run at her engine by a host)
//!    skips her recall tuning and her model choice.
//! c. A plan run does not apply the session's withheld tools
//!    (`withhold_session_tools`); what it offers is what the engine still held.

use crate::support::engine::RunEnd;
use crate::support::model::{Agent, Call};
use crate::support::{say, text, tool, Setup, World};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// A project skill written through the skill-file editor: every live engine
/// reads the skill list again at its next READ of live state.
async fn add_skill(w: &World, root: &str, name: &str) {
    let file = json!({
        "project_root": root,
        "path": format!("{name}/SKILL.md"),
        "content": format!("---\nname: {name}\ndescription: {} skill.\n---\n\nBody.\n", name.to_uppercase()),
    });
    w.api.post("/api/skill-file", file).await;
}

async fn until_calls(w: &World, agent: Agent, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while w.model.calls_of(agent).len() < n {
        assert!(
            Instant::now() < deadline,
            "only {} calls of {}; log:\n{}",
            w.model.calls_of(agent).len(),
            agent.id(),
            w.engine.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn call(w: &World, agent: Agent, i: usize) -> Call {
    w.model.calls_of(agent).into_iter().nth(i).unwrap_or_else(|| panic!("no call {i} of {}", agent.id()))
}

/// A shell that ends on its own, started in the session's own turn.
async fn first_turn(w: &World, sid: &str) {
    let mark = w.mark();
    w.api.chat(sid, &w.work(), "Run it in the background").await;
    if let RunEnd::Asked(ask) = w.wait_run(sid, Agent::Ling, mark).await {
        w.api.answer(&ask, "grant:all").await;
        w.finish_run(sid, Agent::Ling, mark).await;
    }
}

/// (a) The end of a background task is told to Ling in a turn of its own.
/// That turn does not call `read_live_state`, and still lists a skill saved
/// after the engine's last turn: the loop reads it when it starts.
#[tokio::test]
async fn a_task_end_turn_lists_a_skill_saved_since_the_engine_last_read() {
    let w = World::start(Setup::default().setting_off().script(
        Agent::Ling,
        vec![
            tool("Bash", json!({"cmd": "sleep 3; echo finished", "run_in_background": true})),
            text("Started it."),
            text("Noted the end."),
            text("Anything new."),
        ],
    ))
    .await;
    let root = w.work();
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    first_turn(&w, &sid).await;
    add_skill(&w, &root, "probe-task").await;
    until_calls(&w, Agent::Ling, 3).await;
    let end = call(&w, Agent::Ling, 2);
    assert!(end.text().contains("ended: completed"), "the third call is the task's end");
    assert!(end.text().contains("PROBE-TASK"), "a task-end turn did not list the new skill");
    w.turn(&sid, &root, "Anything new?", Agent::Ling).await;
    w.assert_all_scripted();
}

/// (a) A message another agent hands Ling lands in her latest session and
/// runs there. That turn does not call `read_live_state` either, and lists
/// the skill saved since: the loop reads it when it starts.
#[tokio::test]
async fn an_agent_chat_delivery_lists_a_skill_saved_since_the_engine_last_read() {
    let w = World::start(
        Setup::default()
            .setting_off()
            .script(Agent::Ling, vec![text("One."), text("Counted.")])
            .script(
                Agent::Yinyue,
                vec![tool("agent_chat", json!({"to": "ling", "message": "Count the shares."})), text("Passed on.")],
            ),
    )
    .await;
    let root = w.work();
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    w.turn(&sid, &root, "One.", Agent::Ling).await;
    add_skill(&w, &root, "probe-chat").await;
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "Ask Ling to count."})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    until_calls(&w, Agent::Ling, 2).await;
    let delivered = call(&w, Agent::Ling, 1);
    assert!(delivered.text().contains("Count the shares."), "the second call is the delivery");
    assert!(delivered.text().contains("PROBE-CHAT"), "a delivered turn did not list the new skill");
    w.assert_all_scripted();
}

const PLAN_SID: &str = "sess-1790000002-c0ac7ed2";

/// The planning turn, to the point where the plan waits for its decision.
async fn plan_waits(w: &World, sid: &str) {
    let turn_end = |event: &Value| event["kind"] == "turn_complete" && event["session_id"] == sid;
    let mark = w.mark();
    w.api.chat(sid, &w.work(), "Add a flag, plan it first.").await;
    w.finish_run(sid, Agent::Ling, mark).await;
    let events = w.events_when(|events| events.iter().any(turn_end)).await;
    assert!(events.iter().any(turn_end), "the planning turn never closed");
}

async fn approve(w: &World, sid: &str) {
    let mark = w.mark();
    let action = json!({"project_root": w.work(), "agent_id": "ling", "session_id": sid});
    w.api.post("/api/plan/approve", action).await;
    w.finish_run(sid, Agent::Ling, mark).await;
}

/// (a) An approved plan's run is a turn of its own on the session's engine.
/// It does not call `read_live_state`, and lists a skill saved while the
/// plan waited: the loop reads it when it starts.
#[tokio::test]
async fn an_approved_plans_run_lists_a_skill_saved_since_the_engine_last_read() {
    let w = World::start(Setup::default().setting_off().script(
        Agent::Ling,
        vec![
            tool("EnterPlanMode", json!({"reason": "plan it"})),
            tool("ExitPlanMode", json!({"plan_text": "## Plan\n\n1. Add the flag"})),
            text("Ran the plan."),
        ],
    ))
    .await;
    let root = w.work();
    plan_waits(&w, PLAN_SID).await;
    let planned = w.model.calls_of(Agent::Ling).len();
    add_skill(&w, &root, "probe-plan").await;
    approve(&w, PLAN_SID).await;
    let run = call(&w, Agent::Ling, planned);
    assert!(run.text().contains("Execute the approved plan"), "the call after the plan is its run");
    assert!(run.text().contains("PROBE-PLAN"), "the plan's run did not list the new skill");
    w.assert_all_scripted();
}

/// (c) A session whose record withholds a tool, and a plan made in it: the
/// plan's run is on the engine that made the plan, which still holds what
/// the planning turn read from the record — the tool stays withheld.
#[tokio::test]
async fn a_plan_run_on_the_live_engine_still_withholds_what_the_record_withholds() {
    let (planning, run) = withheld_plan_run(false).await;
    assert!(!planning.tool_names().contains(&"Glob".to_string()), "the record's tool was offered when planning");
    assert!(!run.tool_names().contains(&"Glob".to_string()), "the plan's run on the live engine offered it");
}

/// (c) The same plan approved after the engine restarted: the session's
/// engine is built again, and the plan's run never reads the record's
/// withheld tools — it offers the tool the record withholds.
#[tokio::test]
async fn a_plan_run_on_a_rebuilt_engine_offers_what_the_record_withholds() {
    let (planning, run) = withheld_plan_run(true).await;
    assert!(!planning.tool_names().contains(&"Glob".to_string()), "the record's tool was offered when planning");
    assert!(run.tool_names().contains(&"Glob".to_string()), "the plan's run on a rebuilt engine withheld it");
}

/// The planning call and the plan's run call, in a session that withholds
/// `Glob`; `restart`: the engine restarts between the plan and its approval.
async fn withheld_plan_run(restart: bool) -> (Call, Call) {
    let mut w = World::start(Setup::default().setting_off().script(
        Agent::Ling,
        vec![
            tool("EnterPlanMode", json!({"reason": "plan it"})),
            tool("ExitPlanMode", json!({"plan_text": "## Plan\n\n1. Add the flag"})),
            text("Ran the plan."),
        ],
    ))
    .await;
    let sid = w.api.create_session(json!({"title": "Table", "withheld_tools": ["Glob"]})).await;
    plan_waits(&w, &sid).await;
    let planned = w.model.calls_of(Agent::Ling).len();
    if restart {
        w.engine.restart(&w.home).await;
    }
    approve(&w, &sid).await;
    let planning = call(&w, Agent::Ling, 0);
    let run = call(&w, Agent::Ling, planned);
    assert!(run.text().contains("Execute the approved plan"), "the call after the plan is its run");
    (planning, run)
}

/// (b) An `agent_chat` to her is never delivered as a message to a chat
/// agent (`deliver_to_chat_agent`): `on_agent_chat` sends her own address to
/// her own door (`run_yinyue_turn`), where her recall and her model are set.
/// So the delivery door, which skips both, cannot reach her.
#[tokio::test]
async fn an_agent_chat_to_her_runs_at_her_own_door_on_her_own_model() {
    let w = World::start(
        Setup::default()
            .setting_off()
            .script(
                Agent::Ling,
                vec![tool("agent_chat", json!({"to": "yinyue", "message": "The dice are back."})), text("Told her.")],
            )
            .script(Agent::Yinyue, vec![text("I'm here."), say("Welcome back.")]),
    )
    .await;
    let mark = w.mark();
    w.api.post("/api/yinyue/chat", json!({"text": "Hello"})).await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "Tell her the dice are back").await;
    w.finish_run(&sid, Agent::Ling, mark).await;
    until_calls(&w, Agent::Yinyue, 2).await;
    let hers = call(&w, Agent::Yinyue, 1);
    assert!(hers.text().contains("sent you a message, addressed to YOU"), "her own door's kickoff");
    assert_eq!(hers.model(), "fake-yinyue-1", "her model is the Pet setting's");
    assert_eq!(call(&w, Agent::Ling, 0).model(), "fake-ling-1");
}

fn rpc(name: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}})
}

/// (b) A host that continues a session with `agent: "yinyue"` (`agent_run`
/// with a session id) tells her a turn through the task-end door
/// (`tell_from`), as the engine's start: no recall tuning, no Pet model. Her
/// engine in that session runs on the routing default, not on the model her
/// own door gives her (`an_agent_chat_to_her_runs_at_her_own_door…`).
#[tokio::test]
async fn a_turn_told_to_her_through_the_task_end_door_runs_on_the_routing_default() {
    let w = World::start(
        Setup::default()
            .setting_off()
            .script(Agent::Ling, vec![text("One.")])
            .script(Agent::Yinyue, vec![text("Told.")]),
    )
    .await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    w.turn(&sid, &w.work(), "One.", Agent::Ling).await;
    let ran = w
        .api
        .post("/mcp", rpc("agent_run", json!({"prompt": "Say hello.", "session_id": sid, "agent": "yinyue"})))
        .await;
    let said = ran["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string();
    assert_ne!(ran["result"]["isError"], true, "{said}");
    until_calls(&w, Agent::Yinyue, 1).await;
    assert_eq!(call(&w, Agent::Yinyue, 0).model(), "fake-ling-1", "her Pet model was not applied");
}
