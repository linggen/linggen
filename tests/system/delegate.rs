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

#[tokio::test]
async fn work_agent_run_switches() {
    for (write, requested, expected) in [(true, None, Some("code")), (true, Some("none"), None), (false, None, None), (false, Some("code"), Some("code"))] {
        let w = World::start(Setup::default().script(Agent::Ling, vec![text("Done."), text("Again.")])).await;
        let mut args = json!({"prompt":"Say done."});
        if write { args["write_dir"] = json!(w.work()); }
        if let Some(value) = requested { args["work"] = json!(value); }
        let call = |args| json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":"agent_run", "arguments":args}});
        let result = w.api.post("/mcp", call(args)).await;
        assert_eq!(result["result"]["content"][0]["text"], "Done.", "{result}");
        let sid = delegate_session(&w);
        let path = w.home.linggen_home().join("sessions").join(&sid).join("session.yaml");
        let read = || -> Value { serde_norway::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap() };
        assert_eq!(read()["work"].as_str(), expected);
        // What the run itself was sent — not an export of the session: the
        // section is in the prompt the model saw exactly when the work is code.
        let sent = w.model.calls_of(Agent::Ling);
        let saw = sent.iter().any(|c| c.body.to_string().contains("## Working on code"));
        assert_eq!(saw, expected.is_some(), "write {write}, asked {requested:?}");
        let other = if expected.is_some() { "none" } else { "code" };
        let result = w.api.post("/mcp", call(json!({"prompt":"Again", "session_id":sid, "work":other}))).await;
        assert_eq!(result["result"]["content"][0]["text"], "Again.", "{result}");
        assert_eq!(read()["work"].as_str(), expected);
    }
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
