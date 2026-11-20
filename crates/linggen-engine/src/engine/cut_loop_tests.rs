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
    let (mut engine, root) = engine_on(fake.clone(), post(chat)).await;

    run(&mut engine).await.unwrap();

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
    let told = last["content"].as_str().unwrap();
    assert!(told.contains("output limit"), "the model is told its output was cut: {last}");
    assert!(told.contains("call to Write"), "and which call was cut: {last}");
    assert!(told.contains("nothing ran"), "and that nothing ran: {last}");
    assert!(told.contains("make a smaller call"), "and to make a smaller one: {last}");
    assert!(
        !told.to_lowercase().contains("continue from"),
        "a cut call cannot be gone on from: {last}"
    );
    assert!(!root.path().join("cut.txt").exists(), "nothing was written");
    assert_eq!(engine.last_assistant_text.as_deref(), Some("Written in two parts."));
}

/// The first reply is words, cut mid-sentence; the next answers.
async fn text_cut_then_done(State(fake): State<Fake>, Json(body): Json<serde_json::Value>) -> String {
    let first = {
        let mut requests = fake.requests.lock().unwrap();
        requests.push(body);
        requests.len() == 1
    };
    if !first {
        return sse(&[
            serde_json::json!({"choices":[{"index":0,"delta":{"content":"The rest."}}]}),
            serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
        ]);
    }
    sse(&[
        serde_json::json!({"choices":[{"index":0,"delta":{"content":"The long answer begins and"}}]}),
        serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}),
    ])
}

/// Words cut off can be gone on from: the model is told to continue.
#[tokio::test]
async fn cut_words_are_told_to_go_on() {
    let fake = Fake::default();
    let (mut engine, _root) = engine_on(fake.clone(), post(text_cut_then_done)).await;

    run(&mut engine).await.unwrap();

    let requests = fake.requests.lock().unwrap().clone();
    let sent = requests[1]["messages"].as_array().unwrap();
    let told = sent.last().unwrap()["content"].as_str().unwrap();
    assert!(told.contains("Continue from where it stopped"), "{told}");
    assert!(!told.contains("smaller call"), "{told}");
}

/// Every reply is cut at the output limit.
async fn always_cut(State(fake): State<Fake>, Json(body): Json<serde_json::Value>) -> String {
    fake.requests.lock().unwrap().push(body);
    sse(&[
        serde_json::json!({"choices":[{"index":0,"delta":{"content":"Here is the first part of"}}]}),
        serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}),
    ])
}

/// A reply cut every time does not loop to the step limit: after three cut
/// replies in a row the turn ends as a failure that says the model's output
/// limit keeps cutting its answer.
#[tokio::test]
async fn three_cut_replies_in_a_row_end_the_turn_as_a_failure() {
    let fake = Fake::default();
    let (mut engine, _root) = engine_on(fake.clone(), post(always_cut)).await;

    let ended = run(&mut engine).await;

    assert_eq!(fake.requests.lock().unwrap().len(), 3, "the turn stopped after three cut replies");
    let err = ended.expect_err("a turn cut every time is a failure");
    assert!(format!("{err:#}").contains("output limit keeps cutting"), "{err:#}");
    assert!(format!("{err:#}").contains("in the middle of text"), "{err:#}");
    assert_eq!(
        crate::provider::error::kind_of(&err),
        crate::provider::error::ProviderErrorKind::OutputCut,
        "the reason is typed"
    );
    assert_eq!(
        crate::provider::error::kind_of_message(&err.to_string()),
        crate::provider::error::ProviderErrorKind::OutputCut,
        "and stays that kind as text"
    );
}

/// Every reply is a tool call cut mid-arguments.
async fn always_cut_call(State(fake): State<Fake>, Json(body): Json<serde_json::Value>) -> String {
    fake.requests.lock().unwrap().push(body);
    sse(&[
        serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"Write","arguments":"{\"file_path\":\"big.txt\",\"content\":\"a very long"}}]}}]}),
        serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}),
    ])
}

/// The three cuts that end a turn name what was cut: here, a call to Write.
#[tokio::test]
async fn three_cut_calls_end_the_turn_naming_the_call() {
    let fake = Fake::default();
    let (mut engine, root) = engine_on(fake.clone(), post(always_cut_call)).await;

    let err = run(&mut engine).await.expect_err("a call cut every time is a failure");

    assert_eq!(fake.requests.lock().unwrap().len(), 3);
    let words = format!("{err:#}");
    assert!(words.contains("output limit keeps cutting"), "{words}");
    assert!(words.contains("in the middle of a call to Write"), "{words}");
    assert!(!root.path().join("big.txt").exists(), "nothing was written");
}

/// The engine's turn, which must end within 20 s.
async fn run(engine: &mut AgentEngine) -> anyhow::Result<crate::engine::AgentOutcome> {
    tokio::time::timeout(Duration::from_secs(20), engine.run_agent_loop(None))
        .await
        .expect("the turn ended")
}

/// An engine on a fake OpenAI-style server answering with `route`.
async fn engine_on(
    fake: Fake,
    route: axum::routing::MethodRouter<Fake>,
) -> (AgentEngine, tempfile::TempDir) {
    let app = Router::new()
        .route("/chat/completions", route)
        .with_state(fake);
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
        max_output_tokens: None,
        provided_by: None,
        is_builtin: false,
            price: None,
    };
    let models = Arc::new(ModelManager::new_with_credentials(
        vec![model],
        &crate::credentials::Credentials::default(),
    ));
    let mut engine = AgentEngine::new(cfg, models, id, AgentRole::Lead).unwrap();
    engine.auto_fallback = false;
    engine.set_task("write a long file".to_string());
    (engine, root)
}
