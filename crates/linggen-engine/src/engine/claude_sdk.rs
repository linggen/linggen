//! What a Claude Agent SDK session is, as the engine needs it: the member
//! id, which models can run one, and a new session's record. The SDK host
//! itself is the server's (`server::claude`); the engine's
//! `StartClaudeSession` tool creates the session through these.

use crate::config::ModelConfig;
use crate::state_fs::sessions::{SessionMember, SessionMeta, SessionRuntime};

pub const AGENT_ID: &str = "claude";

pub fn supports_model(model: &ModelConfig) -> bool {
    model.provider == "anthropic"
        && model.auth_mode.as_deref() != Some("claude_oauth")
        && model.api_key.as_deref().is_some_and(|key| !key.is_empty())
}

pub fn new_meta(title: String, cwd: String, parent: Option<String>) -> SessionMeta {
    let mut meta = SessionMeta {
        id: format!(
            "sess-{}-{}",
            crate::util::now_ts_secs(),
            &uuid::Uuid::new_v4().to_string()[..8]
        ),
        title,
        created_at: crate::util::now_ts_secs(),
        runtime: SessionRuntime::ClaudeAgentSdk,
        agents: vec![SessionMember::new(AGENT_ID)],
        parent_session_id: parent,
        creator: "user".into(),
        title_locked: false,
        ..Default::default()
    };
    meta.set_cwd(&cwd);
    meta
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_fs::sessions::ClaudePermissionMode;
    use serde_json::json;

    #[test]
    fn claude_sessions_are_created_without_an_agent_spec_or_native_members() {
        let meta = new_meta("Fix tests".into(), "/project".into(), Some("parent".into()));
        assert_eq!(meta.runtime, SessionRuntime::ClaudeAgentSdk);
        assert_eq!(meta.agents, [SessionMember::new("claude")]);
        assert_eq!(meta.parent_session_id.as_deref(), Some("parent"));
        let stored = serde_norway::to_string(&meta).unwrap();
        let loaded: SessionMeta = serde_norway::from_str(&stored).unwrap();
        assert_eq!(loaded.runtime, SessionRuntime::ClaudeAgentSdk);
        assert_eq!(loaded.claude_permission_mode, ClaudePermissionMode::Default);
    }

    #[test]
    fn sdk_requires_a_compatible_model_with_resolved_credentials() {
        let model = |provider: &str, key: Option<&str>, mode: Option<&str>| {
            serde_json::from_value::<ModelConfig>(json!({
            "id":"test", "provider":provider, "model":"test", "url":"http://unused", "api_key":key, "auth_mode":mode
        })).unwrap()
        };
        assert!(supports_model(&model("anthropic", Some("test-key"), None)));
        assert!(!supports_model(&model("anthropic", None, None)));
        assert!(!supports_model(&model(
            "anthropic",
            Some("test-key"),
            Some("claude_oauth")
        )));
        assert!(!supports_model(&model("openai", Some("test-key"), None)));
    }
}
