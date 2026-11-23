//! A held turn (one the engine started for her) on a model without native
//! tool calls: its plain-text reply — however short, an envelope is one line —
//! is the turn's answer for its caller, and nothing of it is shown live.
use crate::engine::{AgentEngine, AgentRole, EngineConfig, InterfaceMode};
use crate::provider::models::ModelManager;
use axum::{extract::State, routing::post, Json, Router};
use std::sync::Arc;
use std::time::Duration;

async fn chat(State(()): State<()>, Json(_body): Json<serde_json::Value>) -> String {
    let chunks = [
        serde_json::json!({"choices":[{"index":0,"delta":{"content":"<say>The build is done.</say>"}}]}),
        serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
    ];
    let mut body: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    body.push_str("data: [DONE]\n\n");
    body
}

async fn engine_without_native_tools() -> (AgentEngine, tempfile::TempDir) {
    let app = Router::new().route("/chat/completions", post(chat)).with_state(());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    let cfg = EngineConfig::from_app_config(
        &crate::config::Config::default(),
        root.path().into(),
        InterfaceMode::Web,
    );
    let id = format!("held-test-{}", url.rsplit(':').next().unwrap());
    let model = crate::config::ModelConfig {
        id: id.clone(),
        provider: "openai".into(),
        url,
        model: "writer".into(),
        api_key: Some("k".into()),
        keep_alive: None,
        context_window: Some(32_000),
        max_output_tokens: None,
        tags: vec![],
        supports_tools: Some(false),
        auth_mode: None,
        reasoning_effort: None,
        provided_by: None,
        is_builtin: false,
        family_binding: None,
        price: None,
    };
    let models = Arc::new(ModelManager::new_with_credentials(
        vec![model],
        &crate::credentials::Credentials::default(),
    ));
    let mut engine = AgentEngine::new(cfg, models, id, AgentRole::Lead).unwrap();
    engine.allow_model_fallback = false;
    engine.set_task("tell her the build is done".to_string());
    engine.reply_held = true;
    (engine, root)
}

#[tokio::test]
async fn a_short_envelope_from_a_model_without_native_tools_is_the_held_turns_reply() {
    let (mut engine, _root) = engine_without_native_tools().await;
    tokio::time::timeout(Duration::from_secs(20), engine.run_agent_loop(None))
        .await
        .expect("the turn ended")
        .unwrap();
    assert_eq!(
        engine.last_assistant_text.as_deref(),
        Some("<say>The build is done.</say>"),
        "the caller reads the envelope, not a nudge loop"
    );
}
