//! A call through `AppTool` is judged at the permission gate as the action
//! it names — the app, the action and its class (`skill::judge`) — not as
//! "AppTool, read": a destructive action asks by its own name, an edit one
//! needs edit, a turn's ceiling holds, and the companion's own rules come
//! first. What a person was not given is refused before anyone is asked.

use super::declined_gate_tests::{engine, Pending};
use super::*;
use crate::engine::agent::AgentManager;
use crate::engine::permission::{PathMode, PermissionMode, SessionPermissions};
use crate::engine::session_policy::SessionPolicy;
use crate::engine::skill::{Skill, SkillSource};
use std::sync::Arc;

fn jukebox(dir: &std::path::Path, grants: &str) -> Skill {
    let text = format!(
        "---\nname: jukebox\ndescription: Plays music.\npermission:\n  paths:\n    - {{ path: \"{dir}\", mode: {grants} }}\ntools:\n\
         \x20 - name: List\n    description: d\n    cmd: \"echo listing\"\n    tier: read\n    reach: anywhere\n\
         \x20 - name: Play\n    description: d\n    cmd: \"echo playing\"\n    tier: edit\n    reach: anywhere\n\
         \x20 - name: Delete\n    description: d\n    cmd: \"echo deleted\"\n    tier: destructive\n    reach: anywhere\n---\nbody\n",
        dir = dir.display(),
        grants = grants
    );
    let mut skill = crate::extensions::skills::parse_skill_text(&text, SkillSource::Project).unwrap();
    skill.skill_dir = Some(dir.to_path_buf());
    skill
}

/// An engine at `work`, its agent `agent`, with the jukebox installed,
/// whose own block grants its folder (`work`) edit. The session may read
/// in `work`; `admin`: it may do everything.
fn world(work: &std::path::Path, pending: &Pending, agent: &str, admin: bool) -> AgentEngine {
    world_granting(work, pending, agent, admin, "edit")
}

/// [`world`] with the app's own block granting its folder `grants`.
fn world_granting(work: &std::path::Path, pending: &Pending, agent: &str, admin: bool, grants: &str) -> AgentEngine {
    let mut engine = engine(work, pending, false);
    // Nobody to ask in a session that is not the admin one: a question
    // would wait for ever.
    let mut path_modes = vec![PathMode { path: work.to_string_lossy().into(), mode: PermissionMode::Read }];
    if admin {
        path_modes.push(PathMode { path: "*".into(), mode: PermissionMode::Admin });
    }
    engine.session_permissions = SessionPermissions { interactive: admin, path_modes, ..Default::default() };
    let (manager, _events) = AgentManager::new(
        crate::config::Config::default(),
        None,
        Arc::new(crate::engine::test_registries::Installed(vec![jukebox(work, grants)])),
        Arc::new(crate::engine::test_registries::Empty),
        Arc::new(crate::engine::test_registries::Empty),
        crate::engine::InterfaceMode::Web,
    );
    engine.tools.builtins.set_context(manager, agent.into());
    engine.agent_id = Some(agent.into());
    engine
}

/// The gate's answer to an `AppTool` call: `None` it passes; else what the
/// model is told.
async fn gate(engine: &mut AgentEngine, tool: &str) -> Option<String> {
    let args = serde_json::json!({"app": "jukebox", "tool": tool});
    let id = Some("c1".to_string());
    let t = ToolTurn { tool: "AppTool", canonical: "AppTool", args: &args, session_id: None, tool_call_id: &id };
    let mut messages = Vec::new();
    let out = engine.gate_permission(&t, &mut messages, &mut Vec::new()).await;
    out.map(|_| messages.last().map(|m| m.content.clone()).unwrap_or_default())
}

/// Answer the first question with `answer`; the question as it was put.
fn answer_first(pending: &Pending, answer: &'static str) -> tokio::sync::oneshot::Receiver<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let pending = pending.clone();
    tokio::spawn(async move {
        loop {
            let next = pending.lock().await.keys().next().cloned();
            if let Some(id) = next {
                let entry = pending.lock().await.remove(&id).unwrap();
                let _ = tx.send(format!("{:?}", entry.questions));
                let reply = tools::AskUserAnswer { question_index: 0, selected: vec![answer.into()], custom_text: None };
                let _ = entry.sender.send(vec![reply]);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
    });
    rx
}

fn no_question(pending: &Pending) -> bool {
    pending.try_lock().map(|p| p.is_empty()).unwrap_or(false)
}

/// A read from a plain session raises no question. An edit action needs
/// edit where the app runs — its folder — and what its own block grants
/// there counts, as it does in the app's own session; a block that grants
/// less leaves the edit to the session, which here cannot be asked.
#[tokio::test]
async fn a_read_passes_and_an_edit_action_needs_edit_where_the_app_runs() {
    let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
    let pending: Pending = Default::default();
    let mut ling = world(work.path(), &pending, "ling", false);
    assert_eq!(gate(&mut ling, "List").await, None);
    assert_eq!(gate(&mut ling, "Play").await, None, "the app's own block grants its folder edit");
    assert!(no_question(&pending), "nothing was asked");

    let mut reads = world_granting(work.path(), &pending, "ling", false, "read");
    assert_eq!(gate(&mut reads, "List").await, None);
    let told = gate(&mut reads, "Play").await.expect("an edit action needs edit");
    assert!(!told.is_empty());
    assert!(no_question(&pending), "nobody can be asked here");
    let table = reads.session_permissions.path_modes.clone();
    gate(&mut reads, "Play").await;
    assert_eq!(reads.session_permissions.path_modes, table, "the session's table is not changed by the call");
}

/// A destructive action is asked of the person by its own name, whatever
/// was granted; one answer, and only that call is allowed.
#[tokio::test]
async fn a_destructive_action_asks_by_its_own_name() {
    let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
    let pending: Pending = Default::default();
    let asked = answer_first(&pending, crate::engine::permission::ALLOW_ONCE);
    let mut ling = world(work.path(), &pending, "ling", true);
    assert_eq!(gate(&mut ling, "Delete").await, None, "allowed once");
    let question = tokio::time::timeout(std::time::Duration::from_secs(5), asked).await.expect("asked").unwrap();
    assert!(question.contains("Delete") && !question.contains("AppTool"), "{question}");
}

/// A turn's ceiling holds over the companion's rules: the person asked her
/// to play, and the turn's device stands at read.
#[tokio::test]
async fn a_turn_under_a_ceiling_stays_under_it_for_the_companion_too() {
    let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
    let pending: Pending = Default::default();
    let mut her = world(work.path(), &pending, "yinyue", true);
    her.tools.builtins.person_spoke = true;
    assert_eq!(gate(&mut her, "Play").await, None, "no ceiling: the person asked, she may");
    her.respec(|spec| spec.ceiling = Some(PermissionMode::Read));
    let told = gate(&mut her, "Play").await.expect("refused under the ceiling");
    assert!(told.contains("Beyond this turn's level"), "{told}");
    assert_eq!(gate(&mut her, "List").await, None, "a read is within it");
    let mut ling = world(work.path(), &pending, "ling", true);
    ling.respec(|spec| spec.ceiling = Some(PermissionMode::Read));
    let told = gate(&mut ling, "Play").await.expect("refused under the ceiling");
    assert!(told.contains("Beyond this turn's level"), "{told}");
}

/// She changes things only in a turn the person started, and the person's
/// own deletion is never hers — refused before anyone is asked.
#[tokio::test]
async fn the_companion_changes_things_only_when_asked_and_never_destroys() {
    let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
    let pending: Pending = Default::default();
    let mut her = world(work.path(), &pending, "yinyue", true);
    let told = gate(&mut her, "Play").await.expect("her own turn");
    assert!(told.contains("this turn is yours"), "{told}");
    her.tools.builtins.person_spoke = true;
    assert_eq!(gate(&mut her, "Play").await, None);
    let told = gate(&mut her, "Delete").await.expect("never hers");
    assert!(told.contains("the person does this themselves, in the app"), "{told}");
    assert!(no_question(&pending), "the person is not asked to let her");
}

/// An app a person was not given is refused to their turn before anyone is
/// asked, and a turn with no such limit is not.
#[tokio::test]
async fn an_app_the_person_was_not_given_is_refused_before_anyone_is_asked() {
    let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
    let pending: Pending = Default::default();
    let mut person = world(work.path(), &pending, "ling", true);
    SessionPolicy::person(vec!["other".into()], vec![]).apply(&mut person);
    for tool in ["List", "Delete"] {
        let told = gate(&mut person, tool).await.expect("closed to them");
        assert!(told.contains("is not one of the apps you may use"), "{told}");
    }
    assert!(no_question(&pending));
    // Nor is it listed to them, or run by name past the gate.
    let ask = |args: serde_json::Value| tools::ToolCall { tool: "AppTool".into(), args, block_id: None };
    let text = |r: tools::ToolResult| match r {
        tools::ToolResult::Success(text) => text,
        other => format!("{other:?}"),
    };
    let listed = text(person.tools.builtins.execute(ask(serde_json::json!({}))).await.unwrap());
    assert!(listed.contains("No app offers you"), "{listed}");
    let why = person.tools.builtins.execute(ask(serde_json::json!({"app": "jukebox"}))).await.unwrap_err();
    assert_eq!(why.to_string(), "'jukebox' is not one of the apps you may use");
    SessionPolicy::person(vec!["jukebox".into()], vec![]).apply(&mut person);
    assert_eq!(gate(&mut person, "List").await, None);
    let listed = text(person.tools.builtins.execute(ask(serde_json::json!({}))).await.unwrap());
    assert!(listed.contains("\"tool\":\"List\""), "{listed}");
}

/// Under a turn's ceiling Linggen's own folder is the owner's alone —
/// whatever the session's grants or the app's own block — and every app
/// installed lives in it: a read of an app the person was given is refused
/// as beyond the turn's level. The rule that was there first wins
/// (`permission::owners_alone`).
#[tokio::test]
async fn under_a_ceiling_an_app_in_linggens_own_folder_is_not_reached() {
    let dir = crate::paths::linggen_home().join("skills/zz-gate-jukebox");
    std::fs::create_dir_all(&dir).unwrap();
    let pending: Pending = Default::default();
    let mut person = world(&dir, &pending, "ling", true);
    SessionPolicy::person(vec!["jukebox".into()], vec![]).apply(&mut person);
    assert_eq!(gate(&mut person, "List").await, None, "no ceiling: the owner's own folder is theirs");
    person.respec(|spec| spec.ceiling = Some(PermissionMode::Read));
    let told = gate(&mut person, "List").await.expect("refused under the ceiling");
    assert!(told.contains("Beyond this turn's level"), "{told}");
    std::fs::remove_dir_all(&dir).ok();
}
