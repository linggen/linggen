//! An app's actions are found through a short index in the prompt and run
//! through the one `AppTool` (doc/tool-spec.md § AppTool): the tools a plain
//! session is sent do not grow with the apps installed, and Yinyue runs an
//! app's action in two model calls.
//!
//! Every app here is a made-up fixture installed by its test.

use crate::support::model::Agent;
use crate::support::{text, Setup, World};
use serde_json::json;

/// Install a made-up app `name` whose tools are `tools` (SKILL.md `tools:`
/// entries, already indented), with an edit grant on its own folder.
pub(crate) async fn install_app(w: &World, name: &str, about: &str, tools: &str) {
    let dir = w.home.linggen_home().join(format!("skills/{name}"));
    std::fs::create_dir_all(&dir).expect("app dir");
    let text = format!(
        "---\nname: {name}\ndescription: {about}\ncwd: {dir}\npermission:\n  paths:\n    - {{ path: {dir}, mode: edit }}\ntools:\n{tools}---\n\nYou run {name}.\n",
        dir = dir.display()
    );
    std::fs::write(dir.join("SKILL.md"), text).expect("SKILL.md");
    w.api.post("/api/skills/reload", json!({})).await;
}

/// One declared action: its class word, whether it is offered `anywhere`.
pub(crate) fn action(name: &str, class: &str, anywhere: bool, cmd: &str) -> String {
    let reach = if anywhere { "    reach: anywhere\n" } else { "" };
    format!(
        "  - name: {name}\n    description: The {name} action.\n    cmd: \"{cmd}\"\n    tier: {class}\n{reach}    args:\n      what:\n        type: string\n        description: What to {name}.\n"
    )
}

/// An app with a read, an edit and a destructive action, all `anywhere`.
pub(crate) async fn install_jukebox(w: &World, name: &str) {
    let tools = [
        action("List", "read", true, "echo listing"),
        action("Play", "edit", true, "echo playing {{what}}"),
        action("Delete", "destructive", true, "echo deleted {{what}}"),
        action("Tune", "read", false, "echo tuned"),
    ]
    .concat();
    install_app(w, name, "Plays music on this Mac.", &tools).await;
}

/// The characters of tool schema a call carried.
pub(crate) fn schema_chars(call: &crate::support::model::Call) -> usize {
    call.body["tools"].to_string().len()
}

/// A plain Ling session is sent the same tool schemas whatever apps are
/// installed: an app adds a line to the prompt's index and nothing to the
/// tools. The number is printed (`-- --nocapture`).
#[tokio::test]
async fn a_plain_sessions_tool_schema_does_not_grow_with_the_apps_installed() {
    let setup = Setup::default().script(
        Agent::Ling,
        vec![text("One."), text("Two.")],
    );
    let w = World::start(setup).await;
    for n in 1..=6 {
        install_jukebox(&w, &format!("box{n}")).await;
    }
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    w.turn(&sid, &w.work(), "Hello.", Agent::Ling).await;
    install_jukebox(&w, "box7").await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    w.turn(&sid, &w.work(), "Hello again.", Agent::Ling).await;

    let calls = w.model.calls_of(Agent::Ling);
    let (six, seven) = (schema_chars(&calls[0]), schema_chars(&calls[1]));
    eprintln!("tool schema of a plain Ling session: {six} chars with six apps, {seven} with seven");
    assert_eq!(six, seven, "an app grew the tool schema");
    assert!(six > 0, "no tools were sent");
    w.assert_all_scripted();
}
