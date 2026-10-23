use super::*;
use crate::engine::permission::{PermissionMode, SessionPermissions};
use crate::engine::{AgentRole, EngineConfig, InterfaceMode};
use crate::provider::models::ModelManager;
use std::sync::Arc;

fn engine() -> AgentEngine {
    let cfg = EngineConfig::from_app_config(
        &crate::config::Config::default(),
        PathBuf::from("/workspace/sdk-check"),
        InterfaceMode::Web,
    );
    let models = Arc::new(ModelManager::new_with_credentials(
        Vec::new(),
        &crate::credentials::Credentials::default(),
    ));
    let mut engine = AgentEngine::new(cfg, models, "test".into(), AgentRole::Lead).unwrap();
    engine.session_permissions = SessionPermissions::default();
    engine.session_permissions.interactive = false;
    engine
        .session_permissions
        .set_path_mode("/workspace/sdk-check", PermissionMode::Read);
    engine
}

#[tokio::test]
async fn sdk_uses_existing_path_grants_and_safety_floor() {
    let mut engine = engine();
    let cwd = Path::new("/workspace/sdk-check");
    assert!(engine
        .authorize_sdk_tool("Read", &json!({"file_path":"file.rs"}), cwd)
        .await
        .is_ok());
    assert!(!engine
        .authorize_sdk_tool("Write", &json!({"file_path":"file.rs"}), cwd)
        .await
        .is_ok());
    assert!(!engine
        .authorize_sdk_tool("Read", &json!({"file_path":"/private/secrets/key"}), cwd)
        .await
        .is_ok());
    engine
        .session_permissions
        .set_path_mode("/", PermissionMode::Admin);
    assert!(!engine
        .authorize_sdk_tool("Bash", &json!({"command":"sudo rm file"}), cwd)
        .await
        .is_ok());
}

#[tokio::test]
async fn sdk_search_notebooks_and_shell_cwd_cannot_escape_a_grant() {
    let mut engine = engine();
    let cwd = Path::new("/workspace/sdk-check");
    assert!(!engine
        .authorize_sdk_tool("Glob", &json!({"pattern":"/private/secrets/*"}), cwd)
        .await
        .is_ok());
    assert!(!engine
        .authorize_sdk_tool("Read", &json!({"file_path":"missing/../../other/key"}), cwd)
        .await
        .is_ok());
    assert!(engine
        .authorize_sdk_tool("Read", &json!({}), cwd)
        .await
        .is_err());
    assert!(!engine
        .authorize_sdk_tool(
            "Grep",
            &json!({"path":"/private/secrets","pattern":"key"}),
            cwd
        )
        .await
        .is_ok());
    assert!(!engine
        .authorize_sdk_tool("Glob", &json!({"path":"../other","pattern":"*"}), cwd)
        .await
        .is_ok());
    assert!(!engine
        .authorize_sdk_tool(
            "Bash",
            &json!({"command":"ls"}),
            Path::new("/private/secrets")
        )
        .await
        .is_ok());
    assert!(!engine
        .authorize_sdk_tool(
            "NotebookEdit",
            &json!({"notebook_path":"/private/secrets/a.ipynb"}),
            cwd
        )
        .await
        .is_ok());
}

#[tokio::test]
async fn sdk_respects_mission_and_withheld_tool_restrictions() {
    let mut engine = engine();
    let cwd = Path::new("/workspace/sdk-check");
    engine.withheld_tools.insert("Read".into());
    assert!(!engine
        .authorize_sdk_tool("Read", &json!({"file_path":"file"}), cwd)
        .await
        .is_ok());
    engine.cfg.mission_allowed_tools = Some(["Read".into()].into());
    assert!(!engine
        .authorize_sdk_tool("Bash", &json!({"command":"ls"}), cwd)
        .await
        .is_ok());
}

#[tokio::test]
async fn sdk_tool_discovery_does_not_grant_the_discovered_tools() {
    let mut engine = engine();
    let cwd = Path::new("/workspace/sdk-check");
    engine.session_permissions.path_modes.clear();
    assert!(engine
        .authorize_sdk_tool("ToolSearch", &json!({"query":"Read"}), cwd)
        .await
        .is_ok());
    assert!(engine
        .authorize_sdk_tool("TaskCreate", &json!({"subject":"Investigate"}), cwd)
        .await
        .is_ok());
    assert!(engine
        .authorize_sdk_tool("Read", &json!({"file_path":"file.rs"}), cwd)
        .await
        .is_err());
}

#[test]
fn sdk_resume_requires_history_and_the_same_working_folder() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("resume.json");
    std::fs::write(&path, r#"{"cwd":"/project","session_id":"session-one"}"#).unwrap();
    assert_eq!(
        read_resume(&path, Path::new("/project"), true, 0),
        Some("session-one".into())
    );
    assert!(read_resume(&path, Path::new("/project"), false, 0).is_none());
    assert!(read_resume(&path, Path::new("/other"), true, 0).is_none());
    assert!(read_resume(&path, Path::new("/project"), true, 1).is_none());
}

#[test]
fn sdk_catches_up_with_other_members_and_recall_without_replaying_old_turns() {
    let mut engine = engine();
    use crate::message::ChatMessage as Message;
    engine.task = Some("Continue".into());
    engine.chat_history = vec![
        Message::new("user", "Old request"),
        Message::new("assistant", "Old answer"),
        Message::new("user", "[Ling]: new context"),
        Message::new("system", "fresh recall"),
        Message::new("user", "Continue"),
    ];
    let resumed = engine.sdk_prompt(true).unwrap();
    assert!(resumed.contains("new context") && resumed.contains("fresh recall"));
    assert!(!resumed.contains("Old request"));
    assert!(engine.sdk_prompt(false).unwrap().contains("Old request"));
}

#[test]
fn coder_uses_sdk_without_a_second_persona_prompt() {
    let (spec, body) =
        crate::extensions::agents::parse_agent_markdown(include_str!("../../../agents/coder.md"))
            .unwrap();
    assert_eq!(
        spec.runtime,
        crate::engine::agent::record::AgentRuntime::ClaudeAgentSdk
    );
    assert!(body.is_empty());
}
