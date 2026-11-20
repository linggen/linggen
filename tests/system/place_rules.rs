//! Rules written for one part of a project (`doc/session-spec.md` § Native
//! project instructions): a sub-folder's rule file and a `.claude/rules/`
//! rule scoped by `paths:` reach a coding session's model before its first
//! change there — once — and never reach work elsewhere.

use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use serde_json::{json, Value};

const UI_RULE: &str = "UI-RULE: indent with tabs";
const API_RULE: &str = "API-RULE: every handler logs";

fn agent_run(args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "agent_run", "arguments": args}})
}

#[tokio::test]
async fn place_rules_reach_the_model_before_the_change_and_only_there() {
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![
            tool("Write", json!({"path": "ui/new.txt", "content": "made"})),
            tool("Write", json!({"path": "ui/new.txt", "content": "made"})),
            tool("Read", json!({"path": "src/api/users.ts"})),
            text("Done."),
        ],
    ))
    .await;
    let work = w.home.work();
    std::fs::create_dir_all(work.join("ui")).unwrap();
    std::fs::create_dir_all(work.join("src/api")).unwrap();
    std::fs::create_dir_all(work.join(".claude/rules")).unwrap();
    std::fs::write(work.join("ui/CLAUDE.md"), UI_RULE).unwrap();
    std::fs::write(work.join("src/api/users.ts"), "export {}").unwrap();
    std::fs::write(
        work.join(".claude/rules/api.md"),
        format!("---\npaths:\n  - \"src/api/**/*.ts\"\n---\n{API_RULE}"),
    )
    .unwrap();

    let ran = w
        .api
        .post("/mcp", agent_run(json!({"prompt": "Make the UI file.", "write_dir": w.work(), "work": "code"})))
        .await;
    assert_eq!(ran["result"]["content"][0]["text"], "Done.", "{ran}");

    let calls = w.model.calls_of(Agent::Ling);
    assert_eq!(calls.len(), 4);
    let count = |i: usize, rule: &str| calls[i].text().matches(rule).count();
    assert_eq!((count(0, UI_RULE), count(0, API_RULE)), (0, 0), "a fresh session carries neither");
    assert_eq!(count(1, UI_RULE), 1, "the UI rule, once, before the change");
    assert!(calls[1].text().contains("Not changed yet"), "the first Write waited");
    assert_eq!(count(1, API_RULE), 0, "work in ui/ does not carry the API rule");
    assert_eq!(count(2, UI_RULE), 1);
    assert!(work.join("ui/new.txt").exists(), "the second Write went through");
    assert_eq!((count(3, UI_RULE), count(3, API_RULE)), (1, 1));
    w.assert_all_scripted();
}
