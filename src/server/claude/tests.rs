use super::*;
use crate::state_fs::sessions::ClaudePermissionMode;

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
fn existing_sessions_keep_native_runtime_and_sdk_modes_cannot_be_native_tiers() {
    let meta: SessionMeta =
        serde_norway::from_str("id: old\ntitle: Existing\ncreated_at: 1\n").unwrap();
    assert_eq!(meta.runtime, SessionRuntime::Linggen);
    for mode in ["read", "edit", "admin", "bypassPermissions"] {
        assert!(serde_json::from_value::<ClaudePermissionMode>(json!(mode)).is_err());
    }
    assert_eq!(
        serde_json::to_value(ClaudePermissionMode::AcceptEdits).unwrap(),
        "acceptEdits"
    );
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
