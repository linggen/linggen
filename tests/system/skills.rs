//! The test skills' own tools and tool sets (doc/skill-spec.md).

use crate::support::model::Agent;
use crate::support::rows::calls_by;
use crate::support::engine::RunEnd;
use crate::support::{text, tool, Setup, World};
use serde_json::json;

/// A skill's `tier: edit` tool runs under the grant the skill declares on
/// its own folder — no prompt — and writes there. An app's own session,
/// with the setting off; in the day:
/// `day_session::an_apps_edit_tool_runs_under_its_own_grant_in_the_day`.
#[tokio::test]
async fn skill_edit_tool_runs_under_its_own_grant() {
    let w = World::start(Setup::default().setting_off().script(
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

/// `allowed-tools: []`: a zero-tool session — the skill's tools: none. The
/// agent's own notes are not the skill's (`note_write`, `note_done` follow
/// the agent to every table), so they are the only tools in the export and on
/// the model call. An app's own session, with the setting off; in the day:
/// `day_session::a_zero_tool_app_offers_no_tools_in_the_day`.
#[tokio::test]
async fn zero_tool_skill_offers_no_tools() {
    let setup = Setup::default().setting_off().script(Agent::Ling, vec![text("Hello.")]);
    let w = World::start(setup).await;
    let sid = w
        .api
        .create_session(json!({"title": "quiet", "skill": "quiet"}))
        .await;
    let export = w.api.export(&sid, "ling", "").await;
    let mut listed: Vec<&str> = export["tools"]
        .as_array()
        .map(|all| all.iter().filter_map(|t| t["name"].as_str()).collect())
        .unwrap_or_default();
    listed.sort();
    assert_eq!(listed, ["note_done", "note_write"], "the export lists {export}");

    w.turn(&sid, "", "Hi.", Agent::Ling).await;
    let call = w.model.calls_of(Agent::Ling).pop().expect("his call");
    let mut offered = call.tool_names();
    offered.sort();
    assert_eq!(offered, ["note_done", "note_write"], "the call offered {offered:?}");
    w.assert_all_scripted();
}

/// A skill written, deleted or uninstalled through the API reaches a chat
/// already under way: its next turn lists exactly the skills on disk. Every
/// "skills changed" path ends in one follow-up (`api/skills.rs`
/// `skills_changed`), which has each live engine read its skill list again
/// at its next turn.
#[tokio::test]
async fn a_change_of_skills_reaches_a_live_session() {
    let ling = (1..=4).map(|n| text(&format!("Turn {n}."))).collect();
    let w = World::start(Setup::default().setting_off().script(Agent::Ling, ling)).await;
    let root = w.work();
    let sid = w.api.create_session(json!({"title": "chat"})).await;
    let last_call = |w: &World| w.model.calls_of(Agent::Ling).pop().expect("his call").text();

    w.turn(&sid, &root, "One.", Agent::Ling).await;
    assert!(!last_call(&w).contains("PROBE-WRITTEN"));

    // A project skill written through the skill-file editor.
    let file = json!({
        "project_root": root,
        "path": "probe-written/SKILL.md",
        "content": "---\nname: probe-written\ndescription: PROBE-WRITTEN skill.\n---\n\nBody.\n",
    });
    w.api.post("/api/skill-file", file).await;
    w.turn(&sid, &root, "Two.", Agent::Ling).await;
    assert!(last_call(&w).contains("PROBE-WRITTEN"), "the written skill is not listed");

    // The same file deleted.
    let gone = json!({"project_root": root, "path": "probe-written/SKILL.md"});
    w.api.delete_status("/api/skill-file", gone).await;
    w.turn(&sid, &root, "Three.", Agent::Ling).await;
    assert!(!last_call(&w).contains("PROBE-WRITTEN"), "the deleted skill is still listed");

    // A global skill uninstalled from the marketplace page.
    let dir = w.home.linggen_home().join("skills/probe-global");
    std::fs::create_dir_all(&dir).expect("skill dir");
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: probe-global\ndescription: PROBE-GLOBAL skill.\n---\n\nBody.\n",
    )
    .expect("SKILL.md");
    w.api.post("/api/skills/reload", json!({"project_root": root})).await;
    w.api
        .delete_status("/api/marketplace/uninstall", json!({"name": "probe-global", "project_root": root}))
        .await;
    w.turn(&sid, &root, "Four.", Agent::Ling).await;
    assert!(!last_call(&w).contains("PROBE-GLOBAL"), "the uninstalled skill is still listed");
    w.assert_all_scripted();
}

/// A skill saved while a turn waits on the person: the turn goes on with
/// what it holds — the folder the person then declines stays closed for the
/// rest of the run, no second prompt — and the session's next turn lists the
/// new skill. The follow-up never drops a session's engine
/// (`a_change_of_skills_keeps_every_live_engine`).
#[tokio::test]
async fn a_skill_saved_mid_turn_reaches_the_next_turn_and_keeps_the_decline() {
    let ling = vec![
        tool("Write", json!({"path": "first.txt", "content": "one"})),
        tool("Write", json!({"path": "second.txt", "content": "two"})),
        text("Left them."),
        text("Next."),
    ];
    let w = World::start(Setup::default().script(Agent::Ling, ling)).await;
    let root = w.work();
    let sid = w.api.create_session(json!({"title": "chat", "project_root": root})).await;

    let mark = w.mark();
    w.api.chat(&sid, &root, "Write two files.").await;
    let RunEnd::Asked(ask) = w.wait_run(&sid, Agent::Ling, mark).await else {
        panic!("the Write ran without a permission prompt");
    };
    let file = json!({
        "project_root": root,
        "path": "probe-midturn/SKILL.md",
        "content": "---\nname: probe-midturn\ndescription: PROBE-MIDTURN skill.\n---\n\nBody.\n",
    });
    w.api.post("/api/skill-file", file).await;
    w.api.answer(&ask, "deny").await;
    // The second Write is refused without asking: the decline is still held.
    w.finish_run(&sid, Agent::Ling, mark).await;
    assert!(!w.home.work().join("first.txt").exists());
    assert!(!w.home.work().join("second.txt").exists());

    w.turn(&sid, &root, "And now?", Agent::Ling).await;
    let call = w.model.calls_of(Agent::Ling).pop().expect("his call");
    assert!(call.text().contains("PROBE-MIDTURN"), "the saved skill is not listed");
    w.assert_all_scripted();
}
