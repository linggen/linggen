//! A turn's step limit (`doc/agentic-loop.md`): a coding session has its own,
//! far higher, and a turn that ends at its limit says so as a fact a caller
//! reads, and the same session continues it.

use crate::support::model::{Agent, Reply};
use crate::support::{text, tool, Setup, World};
use serde_json::{json, Value};

/// The fixture config's chat limit (`tests/fixtures/home/config`).
const CHAT_LIMIT: usize = 12;

/// `n` distinct reads, each a step, then `last`.
fn reads_then(n: usize, last: Reply) -> Vec<Reply> {
    let mut script: Vec<Reply> = (0..n)
        .map(|i| tool("Read", json!({"path": format!("page-{i}.txt")})))
        .collect();
    script.push(last);
    script
}

fn agent_run(args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "agent_run", "arguments": args}})
}

async fn status(w: &World, session: &str) -> Value {
    let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "agent_status", "arguments": {"op": "session", "session_id": session}}});
    let reply = w.api.post("/mcp", call).await;
    let text = reply["result"]["content"][0]["text"].as_str().unwrap_or_else(|| panic!("{reply}"));
    serde_json::from_str(text).unwrap()
}

/// The one session an external host made.
fn only_session(w: &World) -> String {
    let dir = w.home.linggen_home().join("sessions");
    let mut made: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|id| {
            let meta = std::fs::read_to_string(dir.join(id).join("session.yaml")).unwrap_or_default();
            meta.lines().any(|l| l.trim() == "creator: external")
        })
        .collect();
    assert_eq!(made.len(), 1, "one delegated session: {made:?}");
    made.remove(0)
}

/// A chat turn stops at the chat limit and says what happened: the limit,
/// as a typed reason with the work kept — not a failure to refine — and the
/// same session goes on from the reading it had done.
#[tokio::test]
async fn a_turn_at_its_limit_is_a_fact_the_session_continues_from() {
    let w = World::start(
        Setup::default().script(Agent::Ling, reads_then(CHAT_LIMIT, text("Finished the pages."))),
    )
    .await;

    let ran = w
        .api
        .post("/mcp", agent_run(json!({"prompt": "Read every page.", "write_dir": w.work(), "work": "none"})))
        .await;

    let said = ran["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string();
    assert!(said.contains(&format!("limit of {CHAT_LIMIT}")), "{ran}");
    assert!(!said.contains("refine"), "the request was fine: {ran}");
    assert_eq!(w.model.calls_of(Agent::Ling).len(), CHAT_LIMIT, "one call a step, none past the limit");

    let sid = only_session(&w);
    let fact = status(&w, &sid).await;
    assert_eq!(fact["status"], "completed", "{fact}");
    let why = &fact["cannot_run"];
    assert_eq!(why["reason"], "step_limit", "{fact}");
    assert_eq!(why["steps"], CHAT_LIMIT, "{fact}");
    assert_eq!(why["work_kept"], true, "{fact}");
    assert!(why["line"].as_str().unwrap_or_default().contains("continued in the same session"), "{fact}");

    let again = w
        .api
        .post("/mcp", agent_run(json!({"prompt": "Go on.", "session_id": sid})))
        .await;
    assert_eq!(again["result"]["content"][0]["text"], "Finished the pages.", "{again}");
    let continued = w.model.calls_of(Agent::Ling).pop().unwrap().body.to_string();
    assert!(continued.contains("page-0.txt") && continued.contains("page-11.txt"), "the reading was lost");
    assert_eq!(status(&w, &sid).await["cannot_run"], Value::Null);
}

/// A coding session takes the same script past the chat limit and finishes.
#[tokio::test]
async fn a_coding_session_passes_the_chat_limit() {
    let w = World::start(
        Setup::default().script(Agent::Ling, reads_then(CHAT_LIMIT + 3, text("Finished the pages."))),
    )
    .await;

    let ran = w
        .api
        .post("/mcp", agent_run(json!({"prompt": "Read every page.", "write_dir": w.work(), "work": "code"})))
        .await;

    assert_eq!(ran["result"]["content"][0]["text"], "Finished the pages.", "{ran}");
    assert_eq!(w.model.calls_of(Agent::Ling).len(), CHAT_LIMIT + 4);
    let fact = status(&w, &only_session(&w)).await;
    assert_eq!(fact["cannot_run"], Value::Null, "{fact}");
}
