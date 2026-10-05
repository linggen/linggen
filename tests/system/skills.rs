//! The test skills' own tools and tool sets (doc/skill-spec.md).

use crate::support::model::Agent;
use crate::support::rows::calls_by;
use crate::support::{text, tool, Setup, World};
use serde_json::json;

/// A skill's `tier: edit` tool runs under the grant the skill declares on
/// its own folder — no prompt — and writes there.
#[tokio::test]
async fn skill_edit_tool_runs_under_its_own_grant() {
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![tool("Roll", json!({"faces": 9})), text("Five.")],
    ))
    .await;
    let sid = w
        .api
        .create_session(json!({"title": "table", "skill": "dice"}))
        .await;
    w.turn(&sid, "", "Roll a nine-sided die.", Agent::Ling)
        .await;

    assert_eq!(calls_by(&w.rows(&sid), "ling"), ["Roll"]);
    let table = std::fs::read_to_string(w.home.linggen_home().join("skills/dice/data/table.json"))
        .expect("table");
    assert!(
        table.contains(r#""last_roll":{"faces":9,"value":5}"#),
        "Roll did not write the table: {table}"
    );
    w.assert_all_scripted();
}

/// `allowed-tools: []`: a zero-tool session — nothing in the export, nothing
/// offered on the model call.
#[tokio::test]
async fn zero_tool_skill_offers_no_tools() {
    let w = World::start(Setup::default().script(Agent::Ling, vec![text("Hello.")])).await;
    let sid = w
        .api
        .create_session(json!({"title": "quiet", "skill": "quiet"}))
        .await;
    let export = w.api.export(&sid, "ling", "").await;
    assert_eq!(export["tools"], json!([]), "the export lists tools");

    w.turn(&sid, "", "Hi.", Agent::Ling).await;
    let call = w.model.calls_of(Agent::Ling).pop().expect("his call");
    assert!(
        call.tool_names().is_empty(),
        "the call offered {:?}",
        call.tool_names()
    );
    w.assert_all_scripted();
}
