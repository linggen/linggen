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

#[test]
fn the_sdk_sees_only_what_a_shell_bun_and_tls_need() {
    let vars = [
        ("PATH", "/bin"),
        ("HOME", "/h"),
        ("LC_ALL", "C"),
        ("LANG", "en"),
        ("HTTPS_PROXY", "http://p"),
        ("OPENAI_API_KEY", "sk-other"),
        ("LINGGEN_HOME", "/h/.linggen"),
        ("GITHUB_TOKEN", "t"),
    ]
    .map(|(k, v)| (k.to_string(), v.to_string()));
    let kept: Vec<String> = runtime::inherited_env(vars.into_iter())
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(kept, ["PATH", "HOME", "LC_ALL", "LANG", "HTTPS_PROXY"]);
}

#[test]
fn session_approvals_live_in_memory_until_the_conversation_ends() {
    let host = Host::default();
    let rule = json!({"type":"addRules","behavior":"allow","rules":[{"toolName":"Bash"}]});
    host.add_session_rules("s", &[rule.clone()]);
    host.add_session_rules("other", &[rule.clone()]);
    assert_eq!(host.session_rules("s"), [rule.clone()]);
    host.forget_session("s");
    assert!(host.session_rules("s").is_empty());
    assert_eq!(host.session_rules("other"), [rule]);
}

#[test]
fn widget_ids_are_unguessable() {
    let a = interaction::new_question_id();
    let b = interaction::new_question_id();
    assert_ne!(a, b);
    assert!(uuid::Uuid::parse_str(a.strip_prefix("sdk-").unwrap()).is_ok());
}

#[test]
fn a_parent_gets_the_outcome_as_facts_not_engine_words() {
    let meta = new_meta("Fix tests".into(), "/project".into(), Some("parent".into()));
    let status: serde_json::Value =
        serde_json::from_str(&session_status(&meta, "completed")).unwrap();
    assert_eq!(
        status,
        json!({"type":"session_status", "session_id":meta.id, "title":"Fix tests",
            "runtime":"claude", "status":"completed"})
    );
}

#[test]
fn chat_pictures_reach_the_bridge_as_data_urls() {
    use super::media::data_url;
    // A PNG's head, sent bare as the chat input sends it.
    assert_eq!(data_url("iVBORw0KGgoAAAANSUhEUg=="), "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==");
    assert_eq!(data_url("/9j/4AAQSkZJRgABAQ=="), "data:image/jpeg;base64,/9j/4AAQSkZJRgABAQ==");
    assert_eq!(data_url("data:image/gif;base64,R0lGOD"), "data:image/gif;base64,R0lGOD");
}

/// What is typed while a turn runs joins it at the next tool round, in the
/// order it came and only into its own session; Steer sends one at once.
#[test]
fn queued_messages_join_the_turn_at_its_tool_round() {
    use host::Delivery;
    let host = Host::default();
    let mut first = host.await_delivery("s", "q1");
    let mut other = host.await_delivery("t", "q2");
    let mut second = host.await_delivery("s", "q3");
    let mut steered = host.await_delivery("s", "q4");
    assert!(host.steer("q4"));
    assert_eq!(steered.try_recv(), Ok(Delivery::Now));
    assert!(!host.steer("q4"), "a message goes in once");
    assert_eq!(host.deliver_waiting("s"), 2);
    assert_eq!(first.try_recv(), Ok(Delivery::Next));
    assert_eq!(second.try_recv(), Ok(Delivery::Next));
    assert!(other.try_recv().is_err(), "another session's message keeps waiting");
    assert_eq!(host.deliver_waiting("s"), 0);
    host.forget_waiting("q2");
    assert_eq!(host.deliver_waiting("t"), 0);
}
