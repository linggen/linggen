//! The deny rules of the person's Claude Code settings in a coding session
//! (`doc/permission-spec.md` § Claude Code's rules), through a real turn.

use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use serde_json::json;

const MARKER: &str = "the-lighthouse-key-is-5520-north";

/// A delegated coding run whose project forbids reading `.env` (and whose
/// settings file allows everything, which grants nothing): the model's
/// Read of it is refused with the rule and its file named, and the model is
/// never shown what the file holds.
#[tokio::test]
async fn a_project_deny_rule_refuses_a_read_in_a_coding_run() {
    let setup = Setup::default().script(
        Agent::Ling,
        vec![tool("Read", json!({"path": ".env"})), tool("Read", json!({"path": "notes.txt"})), text("Done.")],
    );
    let w = World::start(setup).await;
    let work = std::path::Path::new(&w.work()).to_path_buf();
    std::fs::write(work.join(".env"), format!("KEY={MARKER}\n")).unwrap();
    std::fs::write(work.join("notes.txt"), "plain notes\n").unwrap();
    std::fs::create_dir_all(work.join(".claude")).unwrap();
    let settings = json!({"permissions": {"deny": ["Read(./.env)"], "allow": ["Read", "Bash"]}});
    std::fs::write(work.join(".claude/settings.json"), settings.to_string()).unwrap();

    let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "agent_run",
        "arguments": {"prompt": "Read .env and notes.txt.", "write_dir": w.work()}}});
    let ran = w.api.post("/mcp", call).await;
    assert_eq!(ran["result"]["content"][0]["text"], "Done.", "{ran}");

    let calls = w.model.calls_of(Agent::Ling);
    assert_eq!(calls.len(), 3, "the refused read, the read, the answer");
    let after_refusal = calls[1].body.to_string();
    assert!(after_refusal.contains("Refused by Claude Code settings:"), "{after_refusal}");
    assert!(after_refusal.contains("Read(./.env)") && after_refusal.contains(".claude/settings.json"), "{after_refusal}");
    assert!(calls[2].body.to_string().contains("plain notes"), "another read in the folder still runs");
    assert!(
        calls.iter().all(|c| !c.body.to_string().contains(MARKER)),
        "the model was shown the denied file"
    );
}
