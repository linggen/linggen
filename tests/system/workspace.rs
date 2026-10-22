//! Where a new session works (doc/session-spec.md § Working folder): a plain
//! chat starts in the default workspace, a skill in its declared `cwd:`, a
//! chosen folder wins — and nothing a surface sends later rebinds it.

use crate::support::engine::RunEnd;
use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use serde_json::json;
use std::path::PathBuf;

/// The session's `cwd` in its session.yaml.
fn cwd_of(w: &World, sid: &str) -> String {
    let path = w
        .home
        .linggen_home()
        .join("sessions")
        .join(sid)
        .join("session.yaml");
    let meta: serde_json::Value =
        serde_norway::from_str(&std::fs::read_to_string(path).expect("session.yaml"))
            .expect("yaml");
    meta["cwd"].as_str().unwrap_or_default().to_string()
}

fn workspace(w: &World) -> PathBuf {
    w.home.linggen_home().join("workspace")
}

#[tokio::test]
async fn a_plain_chat_starts_in_the_default_workspace() {
    let w = World::start(Setup::default()).await;
    let made = w.api.post("/api/sessions", json!({"title": "Chat"})).await;
    let sid = made["id"].as_str().expect("id");
    let want = workspace(&w).display().to_string();
    assert_eq!(cwd_of(&w, sid), want);
    assert_eq!(made["cwd"], want, "the reply does not name the folder");
    assert!(workspace(&w).is_dir(), "the workspace was not made");
}

#[tokio::test]
async fn a_skill_session_keeps_its_declared_cwd() {
    let w = World::start(Setup::default()).await;
    let sid = w
        .api
        .create_session(json!({"title": "dice", "skill": "dice", "project_root": w.work()}))
        .await;
    let want = w.home.linggen_home().join("skills/dice");
    assert_eq!(cwd_of(&w, &sid), want.display().to_string());
}

#[tokio::test]
async fn a_chosen_folder_wins() {
    let w = World::start(Setup::default()).await;
    let sid = w
        .api
        .create_session(json!({"title": "Chat", "project_root": w.work()}))
        .await;
    assert_eq!(cwd_of(&w, &sid), w.work());
}

/// The 2026-10-22 bug: a page still holding another session's folder sent it
/// with the first `!` command, and the session was bound there.
#[tokio::test]
async fn another_sessions_folder_does_not_rebind_a_new_chat() {
    let w = World::start(Setup::default()).await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    let ran = w
        .api
        .post(
            "/api/bash",
            json!({"session_id": sid, "project_root": w.work(), "command": "pwd"}),
        )
        .await;
    let want = workspace(&w).display().to_string();
    assert_eq!(ran["stdout"].as_str().unwrap_or_default().trim(), want);
    assert_eq!(cwd_of(&w, &sid), want);
}

#[tokio::test]
async fn a_skill_session_with_no_declared_cwd_starts_in_the_skills_folder() {
    let w = World::start(Setup::default()).await;
    let sid = w
        .api
        .create_session(json!({"title": "quiet", "skill": "quiet"}))
        .await;
    let want = w.home.linggen_home().join("skills/quiet");
    assert_eq!(cwd_of(&w, &sid), want.display().to_string());
}

/// A chat sent to a session that does not exist yet, with no folder, makes
/// it in the default workspace — not in `~`.
#[tokio::test]
async fn a_session_made_by_its_first_message_starts_in_the_default_workspace() {
    let w = World::start(Setup::default().script(Agent::Ling, vec![text("Hello.")])).await;
    let sid = "sess-1790000099-f1257c0a";
    let mark = w.mark();
    w.api.chat(sid, "", "Hello?").await;
    w.finish_run(sid, Agent::Ling, mark).await;
    assert_eq!(cwd_of(&w, sid), workspace(&w).display().to_string());
    w.assert_all_scripted();
}

/// A session.yaml an older engine wrote with no folder: read as the default
/// workspace (its `!` commands run there) and kept so from its next write.
#[tokio::test]
async fn a_session_kept_without_a_folder_works_in_the_default_workspace() {
    let w = World::start(Setup::default()).await;
    let sid = "sess-1790000098-01dc3a0d";
    let dir = w.home.linggen_home().join("sessions").join(sid);
    std::fs::create_dir_all(&dir).expect("session dir");
    let yaml = format!("id: {sid}\ntitle: Old chat\ncreated_at: 1790000098\ncreator: user\n");
    std::fs::write(dir.join("session.yaml"), yaml).expect("session.yaml");
    std::fs::write(dir.join("messages.jsonl"), "").expect("messages.jsonl");

    let ran = w
        .api
        .post(
            "/api/bash",
            json!({"session_id": sid, "project_root": w.work(), "command": "pwd"}),
        )
        .await;
    let want = workspace(&w).display().to_string();
    assert_eq!(ran["stdout"].as_str().unwrap_or_default().trim(), want);
    assert_eq!(cwd_of(&w, sid), want, "not kept from the next write");
}

/// The chat half of the same bug: a turn sent with another session's folder
/// runs in the session's own, and its first Bash does not move the session.
#[tokio::test]
async fn a_turn_with_another_sessions_folder_runs_in_the_sessions_own() {
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![tool("Bash", json!({"cmd": "pwd"})), text("Done.")],
    ))
    .await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "Where are we?").await;
    if let RunEnd::Asked(ask) = w.wait_run(&sid, Agent::Ling, mark).await {
        w.api.answer(&ask, "allow once").await;
        w.finish_run(&sid, Agent::Ling, mark).await;
    }
    assert_eq!(cwd_of(&w, &sid), workspace(&w).display().to_string());
    let call = w
        .model
        .calls_of(Agent::Ling)
        .pop()
        .expect("his second call");
    let thread = call.thread();
    let (_, ran) = thread
        .iter()
        .rev()
        .find(|(role, _)| role == "tool")
        .expect("no Bash result");
    assert!(
        ran.contains(&workspace(&w).display().to_string()) && !ran.contains(&w.work()),
        "pwd did not run in the workspace: {ran}"
    );
    w.assert_all_scripted();
}
