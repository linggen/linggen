//! An `agent_run` delegate's session (`doc/session-spec.md` § The session
//! spec): a person typing into it keeps the delegate's spec.

use crate::support::model::Agent;
use crate::support::{text, Setup, World};
use serde_json::{json, Value};

/// The one session an external host made.
fn delegate_session(w: &World) -> String {
    let dir = w.home.linggen_home().join("sessions");
    let mut made: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|id| {
            let meta =
                std::fs::read_to_string(dir.join(id).join("session.yaml")).unwrap_or_default();
            meta.lines().any(|l| l.trim() == "creator: external")
        })
        .collect();
    assert_eq!(made.len(), 1, "one delegated session: {made:?}");
    made.remove(0)
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

/// A person typing into a delegated run's session: the same toolset as
/// the run (no Task, no AskUser, no Bash on a read run), and the session
/// still answers to its caller.
#[tokio::test]
async fn a_person_typing_into_a_delegated_session_keeps_its_toolset() {
    let setup = Setup::default().script(Agent::Ling, vec![text("Done."), text("Still here.")]);
    let w = World::start(setup).await;
    let call = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "agent_run", "arguments": {"prompt": "Say done."}}
    });
    let ran = w.api.post("/mcp", call).await;
    assert_eq!(ran["result"]["content"][0]["text"], "Done.", "{ran}");
    let sid = delegate_session(&w);

    let cwd = {
        let meta = std::fs::read_to_string(
            w.home
                .linggen_home()
                .join("sessions")
                .join(&sid)
                .join("session.yaml"),
        )
        .unwrap();
        let line = meta
            .lines()
            .find(|l| l.starts_with("cwd:"))
            .unwrap()
            .to_string();
        line.trim_start_matches("cwd:")
            .trim()
            .trim_matches('\'')
            .to_string()
    };
    w.turn(&sid, &cwd, "And now?", Agent::Ling).await;

    let calls = w.model.calls_of(Agent::Ling);
    assert_eq!(calls.len(), 2, "the run, then the typed turn");
    let run = sorted(calls[0].tool_names());
    let typed = sorted(calls[1].tool_names());
    assert!(
        !run.contains(&"Bash".to_string()),
        "a read run has no Bash: {run:?}"
    );
    assert_eq!(typed, run, "the typed turn keeps the delegate's toolset");

    let perms: Value = serde_json::from_str(
        &std::fs::read_to_string(
            w.home
                .linggen_home()
                .join("sessions")
                .join(&sid)
                .join("permission.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(perms["interactive"], false, "{perms}");
    assert_eq!(
        perms["ask_caller"], true,
        "it still answers to its caller: {perms}"
    );
    w.assert_all_scripted();
}
