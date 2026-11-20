//! Two real turns of one session's engine, in two places, against a fake
//! model: what the second turn sends the model holds nothing of the first
//! place — its rules, its tools.

use crate::engine::skill::SkillSource;
use crate::engine::skill_activation::ActivationMode;
use crate::engine::turn_place::TurnPlace;
use crate::engine::{AgentEngine, AgentRole, EngineConfig, InterfaceMode};
use crate::provider::models::ModelManager;
use axum::{extract::State, routing::post, Json, Router};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Fake {
    requests: Arc<Mutex<Vec<serde_json::Value>>>,
}

async fn chat(State(fake): State<Fake>, Json(body): Json<serde_json::Value>) -> String {
    fake.requests.lock().unwrap().push(body);
    let row = serde_json::json!({"message": {"role": "assistant", "content": "Done."}, "done": true});
    format!("{row}\n")
}

fn model(id: &str, url: &str) -> crate::config::ModelConfig {
    crate::config::ModelConfig {
        id: id.into(),
        provider: "ollama".into(),
        url: url.into(),
        model: id.into(),
        api_key: None,
        keep_alive: None,
        context_window: Some(32_000),
        tags: vec![],
        supports_tools: Some(true),
        auth_mode: None,
        reasoning_effort: None,
        max_output_tokens: None,
        provided_by: None,
        is_builtin: false,
    }
}

fn app(name: &str, tools: &str, rules: &str, folder: &std::path::Path) -> crate::engine::skill::Skill {
    let text = format!(
        "---\nname: {name}\ndescription: An app.\nallowed-tools: {tools}\ncwd: {}\n\
         tools:\n  - name: {name}-look\n    description: Look.\n    cmd: \"echo x\"\n---\n{rules}",
        folder.display()
    );
    crate::extensions::skills::parse_skill_text(&text, SkillSource::Global).unwrap()
}

/// What one request offered and said.
fn sent(request: &serde_json::Value) -> (String, Vec<String>) {
    let system = request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "system")
        .filter_map(|m| m["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let tools = request["tools"]
        .as_array()
        .map(|t| t.iter().filter_map(|t| t["function"]["name"].as_str().map(String::from)).collect())
        .unwrap_or_default();
    (system, tools)
}

#[tokio::test]
async fn a_turn_in_a_second_place_sends_nothing_of_the_first() {
    let fake = Fake::default();
    let router = Router::new().route("/api/chat", post(chat)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let cfg = EngineConfig::from_app_config(&crate::config::Config::default(), dir.path().into(), InterfaceMode::Web);
    let id = format!("place-test-{}", url.rsplit(':').next().unwrap());
    let mut m = model("m", &url);
    m.id = id.clone();
    let models = Arc::new(ModelManager::new_with_credentials(vec![m], &crate::credentials::Credentials::default()));
    let mut engine = AgentEngine::new(cfg, models, id, AgentRole::Lead).unwrap();
    engine.auto_fallback = false;
    engine.tools.builtins.set_session_id(Some("sess-1-loop-places".into()));
    crate::engine::session_policy::SessionPolicy::owner().apply(&mut engine);

    let x = app("zz-x", "[Read]", "THE X APP'S OWN RULES.", &dir.path().join("x"));
    let y = app("zz-y", "[Grep]", "THE Y APP'S OWN RULES.", &dir.path().join("y"));
    let own_x = TurnPlace { skill: Some("zz-x".into()), own: true, ..TurnPlace::default() };
    let other_y = TurnPlace { skill: Some("zz-y".into()), own: false, ..TurnPlace::default() };

    let mut turns = Vec::new();
    for (place, skill) in [(own_x, x), (other_y, y)] {
        let from = fake.requests.lock().unwrap().len();
        engine.enter_place(place, Some(skill), ActivationMode::SessionBound).await;
        engine.set_task("say done".to_string());
        engine.run_agent_loop(None).await.unwrap();
        engine.leave_place();
        let requests = fake.requests.lock().unwrap()[from..].to_vec();
        assert!(!requests.is_empty());
        turns.push(requests.iter().map(sent).collect::<Vec<_>>());
    }

    let has = |turn: &[(String, Vec<String>)], rules: &str| turn.iter().any(|(s, _)| s.contains(rules));
    let offers = |turn: &[(String, Vec<String>)], tool: &str| turn.iter().any(|(_, t)| t.iter().any(|n| n == tool));
    assert!(has(&turns[0], "THE X APP'S OWN RULES.") && offers(&turns[0], "zz-x-look"));
    assert!(!has(&turns[1], "THE X APP'S OWN RULES."), "x's rules sent in y's turn");
    assert!(!offers(&turns[1], "zz-x-look") && !offers(&turns[1], "Read"), "x's tools offered in y's turn");
    assert!(has(&turns[1], "THE Y APP'S OWN RULES.") && offers(&turns[1], "zz-y-look"));
}
