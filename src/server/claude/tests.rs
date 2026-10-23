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

#[test]
fn sdk_permission_names_are_native_and_new_modes_roundtrip() {
    for mode in ["default", "acceptEdits", "auto", "plan", "dontAsk"] {
        let parsed: ClaudePermissionMode = serde_json::from_value(json!(mode)).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), mode);
    }
}

#[tokio::test]
async fn sdk_images_reuse_persistent_session_images_and_resolvable_references() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        crate::state_fs::sessions::SessionStore::with_sessions_dir(dir.path().join("sessions"));
    let meta = new_meta("picture".into(), dir.path().to_string_lossy().into(), None);
    store.add_session(&meta).unwrap();
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    let shown = super::media::keep(&store, &meta.id, png, "Read image").unwrap();
    let id = shown["reference"].as_str().unwrap();
    let refs =
        crate::engine::answer_refs::view::refs_for(&store, &meta.id, [format!("[{id}]").as_str()]);
    assert!(refs[id]["src"]
        .as_str()
        .unwrap()
        .starts_with("/api/sessions/images?"));
    assert!(super::media::keep(&store, &meta.id, "not an image", "bad").is_err());
    use base64::Engine;
    std::fs::write(
        dir.path().join("image.png"),
        base64::engine::general_purpose::STANDARD
            .decode(png)
            .unwrap(),
    )
    .unwrap();
    let opened = super::media::display(&store, &meta, "image.png", "Local image")
        .await
        .unwrap();
    assert_ne!(opened["reference"], shown["reference"]);
    assert!(!store.session_dir(&meta.id).join("permission.json").exists());
}
