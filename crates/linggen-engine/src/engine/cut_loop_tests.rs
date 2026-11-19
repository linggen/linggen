//! A reply the provider cut at its output limit: the tool call it was still
//! writing is never run, and the model is told its output was cut so it
//! can continue.
use crate::engine::{AgentEngine, AgentRole, EngineConfig, InterfaceMode};
use crate::provider::models::ModelManager;
use axum::{extract::State, routing::post, Json, Router};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Default)]
struct Fake {
    requests: Arc<Mutex<Vec<serde_json::Value>>>,
}

fn sse(chunks: &[serde_json::Value]) -> String {
    let mut body: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    body.push_str("data: [DONE]\n\n");
    body
}

/// The first reply begins a Write and is cut mid-arguments; the next one
/// answers in words.
async fn chat(State(fake): State<Fake>, Json(body): Json<serde_json::Value>) -> String {
    let first = {
        let mut requests = fake.requests.lock().unwrap();
        requests.push(body);
        requests.len() == 1
    };
    if !first {
        return sse(&[
            serde_json::json!({"choices":[{"index":0,"delta":{"content":"Written in two parts."}}]}),
            serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
        ]);
    }
    sse(&[
        serde_json::json!({"choices":[{"index":0,"delta":{"content":"The config is {\"a\": 1}; now the file."}}]}),
        serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"Write","arguments":"{\"file_path\":\"cut.txt\",\"content\":\"the first half of a long"}}]}}]}),
        serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}),
    ])
}

#[tokio::test]
async fn a_cut_tool_call_is_not_run_and_the_model_is_told() {
    let fake = Fake::default();
    let app = Router::new()
        .route("/chat/completions", post(chat))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let root = tempfile::tempdir().unwrap();
    let cfg = EngineConfig::from_app_config(
        &crate::config::Config::default(),
        root.path().into(),
        InterfaceMode::Web,
    );
    let id = format!("cut-test-{}", url.rsplit(':').next().unwrap());
    let model = crate::config::ModelConfig {
        id: id.clone(),
        provider: "openai".into(),
        url,
        model: "writer".into(),
        api_key: Some("k".into()),
        keep_alive: None,
        context_window: Some(32_000),
        tags: vec![],
        supports_tools: Some(true),
        auth_mode: None,
        reasoning_effort: None,
        provided_by: None,
        is_builtin: false,
    };
    let models = Arc::new(ModelManager::new_with_credentials(
        vec![model],
        &crate::credentials::Credentials::default(),
    ));
    let mut engine = AgentEngine::new(cfg, models, id, AgentRole::Lead).unwrap();
    engine.auto_fallback = false;
    engine.set_task("write a long file".to_string());

    tokio::time::timeout(Duration::from_secs(20), engine.run_agent_loop(None))
        .await
        .expect("the turn ended")
        .unwrap();

    let requests = fake.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2, "the turn went on after the cut");
    let sent = requests[1]["messages"].as_array().unwrap();
    assert!(
        !sent.iter().any(|m| m["role"] == "tool" || m.get("tool_calls").is_some()),
        "the cut call was never run: {sent:#?}"
    );
    assert!(
        sent.iter().any(|m| m["role"] == "assistant"
            && m["content"] == "The config is {\"a\": 1}; now the file."),
        "what the cut reply said stays whole, JSON in it included: {sent:#?}"
    );
    let last = sent.last().unwrap();
    assert_eq!(last["role"], "user");
    assert!(
        last["content"].as_str().unwrap().contains("output limit"),
        "the model is told its output was cut: {last}"
    );
    assert!(!root.path().join("cut.txt").exists(), "nothing was written");
    assert_eq!(engine.last_assistant_text.as_deref(), Some("Written in two parts."));
}
