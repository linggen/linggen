use crate::support::{Setup, World};
use serde_json::json;

#[tokio::test]
async fn defaults_round_trip_without_changing_the_old_chain_or_exposing_live_as_chat() {
    const ROW: &str = r#"
[[models]]
id = "reasoning"
provider = "openai"
url = "https://api.openai.com/v1"
model = "gpt-5.4"
api_key = "test-not-a-key"
"#;
    let w = World::start(Setup { config_append: Some(ROW), ..Setup::default() }).await;
    let before = w.api.get("/api/uses").await;
    assert_eq!(before["chain"], json!(["fake-ling"]));
    assert!(before["live"]["choice"].is_null());
    assert!(before["speak"]["choice"].is_null());
    assert!(before["draw"]["choice"].is_null());
    assert_eq!(before["hear"]["options"][0]["id"], "local-stt");
    assert_eq!(w.api.get("/api/live").await["providers"][0]["id"], "alibaba");
    assert_eq!(w.api.put_json("/api/credentials", json!({"endpoints":[
        {"provider":"qwen","url":"https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1","api_key":"test-only"},
        {"provider":"gemini","url":"https://generativelanguage.googleapis.com/v1beta/openai","api_key":"test-only"}
    ]})).await.0, 200);
    for edit in [
        json!({"use":"chat","choice":"reasoning"}),
        json!({"use":"effort","choice":"medium"}),
        json!({"use":"live","choice":"google"}),
        json!({"use":"speak","choice":"system"}),
        json!({"use":"voice","choice":"serena"}),
        json!({"use":"draw","choice":"chatgpt"}),
        json!({"use":"app","id":"model-lesson","choice":"fake-eye"}),
    ] {
        let (code, answer) = w.api.put_json("/api/uses", edit.clone()).await;
        assert_eq!(code, 200, "{edit}: {answer}");
        assert_eq!(answer, w.api.get("/api/uses").await);
    }
    let after = w.api.get("/api/uses").await;
    assert_eq!(after["chain"], json!(["reasoning", "fake-ling"]));
    assert_eq!(after["effort"]["choice"], "medium");
    assert_eq!(after["live"]["effective"], "google");
    assert_eq!(after["speak"]["effective"], "system");
    assert_eq!(after["draw"]["effective"], "chatgpt");
    assert_eq!(w.api.get("/api/live").await["providers"][0]["id"], "google");
    assert_eq!(w.api.put_json("/api/uses", json!({"use":"effort","choice":"xhigh"})).await.0, 400);
    assert_eq!(w.api.put_json("/api/uses", json!({"use":"app","id":"model-lesson","choice":"live:google"})).await.0, 400);
    assert_eq!(w.api.get("/api/uses").await, after);
    let saved = std::fs::read_to_string(w.home.config_path()).unwrap();
    let config = linggen_engine::config::Config::from_toml(&saved).unwrap();
    assert_eq!(config.uses.apps["model-lesson"], "fake-eye");
    assert_eq!(config.uses.chat_effort.as_deref(), Some("medium"));
    assert_eq!(config.pet.voice, "serena");
}

#[tokio::test]
async fn an_app_model_runs_only_in_the_app() {
    use crate::support::{model::Agent, text};
    let w = World::start(Setup::default().script(Agent::Ling, vec![text("Home."), text("App.")])).await;
    assert_eq!(w.api.put_json("/api/uses", json!({"use":"app","id":"model-lesson","choice":"fake-eye"})).await.0, 200);
    let home = w.api.create_session(json!({"title":"Home"})).await;
    w.turn(&home, &w.work(), "Hello home.", Agent::Ling).await;
    let app = w.api.create_session(json!({"title":"Lesson", "skill":"model-lesson"})).await;
    let mark = w.mark();
    let sent = w.api.post("/api/chat", json!({"session_id": app, "skill_name": "model-lesson", "project_root": w.work(), "message": "Hello lesson."})).await;
    let sid = sent["session_id"].as_str().unwrap_or(&app);
    w.finish_run(sid, Agent::Ling, mark).await;
    let calls = w.model.calls_of(Agent::Ling);
    assert_eq!(calls[0].body["model"], "fake-ling-1");
    assert_eq!(calls[1].body["model"], "fake-eye-1");
}
