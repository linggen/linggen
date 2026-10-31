//! A light turn sends a small prompt with no tools, keeps a line once and
//! keeps nothing for SILENT.

use super::*;
use crate::engine::agent::AgentManager;
use crate::engine::skill::SkillSource;
use crate::engine::{AgentRole, EngineConfig, InterfaceMode};
use crate::provider::models::ModelManager;
use axum::{extract::State, routing::post, Json, Router};
use std::sync::{Arc, Mutex};

const LING: &str = include_str!("../../../agents/ling.md");
const RULES: &str = "THE APP'S OWN RULES.";

#[derive(Clone)]
struct Fake {
    requests: Arc<Mutex<Vec<serde_json::Value>>>,
    reply: &'static str,
}

async fn chat(State(fake): State<Fake>, Json(body): Json<serde_json::Value>) -> String {
    fake.requests.lock().unwrap().push(body);
    format!(
        "{{\"message\":{{\"role\":\"assistant\",\"content\":{}}},\"done\":false}}\n\
         {{\"message\":{{\"role\":\"assistant\",\"content\":\"\"}},\"done\":true,\"prompt_eval_count\":20,\"eval_count\":2}}\n",
        serde_json::to_string(fake.reply).unwrap()
    )
}

fn model(url: &str) -> crate::config::ModelConfig {
    crate::config::ModelConfig {
        id: "light-test".into(),
        provider: "ollama".into(),
        url: url.into(),
        model: "light-test".into(),
        api_key: None,
        keep_alive: None,
        context_window: Some(128_000),
        tags: vec![],
        supports_tools: Some(true),
        auth_mode: None,
        reasoning_effort: None,
        provided_by: None,
        is_builtin: false,
    }
}

/// A big app: its SKILL.md body is what a full turn carries and this doesn't.
fn app() -> crate::engine::skill::Skill {
    let text = format!(
        "---\nname: zz\ndescription: A test world told in chat.\nallowed-tools: [AskUser]\napp:\n  launcher: web\n  entry: index.html\n---\n{}",
        RULES.repeat(400)
    );
    crate::extensions::skills::parse_skill_text(&text, SkillSource::Global).unwrap()
}

fn engine_on(url: &str, root: &std::path::Path) -> AgentEngine {
    let cfg = EngineConfig::from_app_config(
        &crate::config::Config::default(),
        root.to_path_buf(),
        InterfaceMode::Web,
    );
    let models = Arc::new(ModelManager::new_with_credentials(
        vec![model(url)],
        &crate::credentials::Credentials::default(),
    ));
    let mut engine = AgentEngine::new(cfg, models, "light-test".into(), AgentRole::Lead).unwrap();
    let (spec, body) = crate::extensions::agents::parse_agent_markdown(LING).unwrap();
    engine.set_spec("ling".into(), spec, body);
    engine.active_skill = Some(app());
    engine.skill_bound = true;
    engine
}

/// The manager, its session store in `dir` — never the person's own.
fn manager_in(dir: &std::path::Path) -> Arc<AgentManager> {
    let (mut manager, _events) = AgentManager::new(
        crate::config::Config::default(),
        None,
        Arc::new(crate::engine::test_registries::Empty),
        Arc::new(crate::engine::test_registries::Empty),
        Arc::new(crate::engine::test_registries::Empty),
        InterfaceMode::Web,
    );
    Arc::get_mut(&mut manager)
        .expect("a fresh manager")
        .global_sessions = crate::state_fs::SessionStore::with_sessions_dir(dir.to_path_buf());
    manager
}

/// One light turn against a fake model answering `reply`: what the model
/// was sent, and the rows the session kept.
async fn run(reply: &'static str) -> (serde_json::Value, Vec<crate::state_fs::sessions::ChatMsg>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let fake = Fake {
        requests: requests.clone(),
        reply,
    };
    let server = Router::new()
        .route("/api/chat", post(chat))
        .with_state(fake);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let served = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine_on(&url, dir.path());
    let manager = manager_in(&dir.path().join("sessions"));
    engine.set_manager_context(manager.clone());
    let tail = vec![
        ChatMessage::new("assistant", "The pass is quiet under snow."),
        ChatMessage::new("user", "[Yinyue]: 本王赢了。"),
    ];
    engine
        .light_reply(tail, "Answer her in ONE line, or SILENT.", "s-light")
        .await;
    served.abort();
    let sent = requests
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("one model call");
    let rows = manager.global_sessions.get_chat_history("s-light").unwrap();
    (sent, rows)
}

#[tokio::test]
async fn a_light_turn_sends_soul_tail_and_ask_and_no_tools() {
    let (sent, _) = run("Well fought.").await;
    assert!(sent
        .get("tools")
        .is_none_or(|t| t.as_array().is_some_and(Vec::is_empty)));
    let msgs = sent["messages"].as_array().unwrap();
    let system = msgs[0]["content"].as_str().unwrap();
    assert!(system.contains("## Identity") && system.contains("A test world told in chat."));
    assert!(!system.contains(RULES), "no SKILL.md body");
    assert!(!system.contains("AskUser"), "no tool talk");
    assert_eq!(msgs.len(), 4, "persona, two tail lines, the ask");
    assert_eq!(msgs[2]["content"], "[Yinyue]: 本王赢了。");
    assert_eq!(msgs[3]["content"], "Answer her in ONE line, or SILENT.");
    let chars: usize = msgs
        .iter()
        .map(|m| m["content"].as_str().unwrap().len())
        .sum();
    assert!(
        chars < 8_000,
        "a light prompt, not a full one: {chars} bytes"
    );
}

#[tokio::test]
async fn a_line_is_kept_once_as_the_agents() {
    let (_, rows) = run("  Well fought.  ").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].from_id.as_str(), rows[0].content.as_str()),
        ("ling", "Well fought.")
    );
}

#[tokio::test]
async fn silence_keeps_nothing() {
    let (_, rows) = run("SILENT").await;
    assert!(rows.is_empty(), "{rows:?}");
}

#[test]
fn a_long_tail_line_is_cut_recall_is_dropped_and_the_ask_comes_last() {
    let long = "x".repeat(TAIL_CHARS + 50);
    let tail = vec![
        ChatMessage::new("system", "From memory (preference, …): RECALL"),
        ChatMessage::new("user", long),
    ];
    let msgs = light_messages("P".into(), tail, "ASK");
    assert_eq!(msgs.len(), 3, "persona, the line, the ask — no recall note");
    assert_eq!(msgs[1].content.chars().count(), TAIL_CHARS + 1);
    assert_eq!(
        (msgs[2].role.as_str(), msgs[2].content.as_str()),
        ("user", "ASK")
    );
}

#[test]
fn silence_and_nothing_are_no_line() {
    assert_eq!(kept_line(" SILENT. "), None);
    assert_eq!(kept_line("  "), None);
    assert_eq!(kept_line(" 好。 ").as_deref(), Some("好。"));
}

/// The size of the two prompts for the same agent at the same app table:
/// the full turn's system messages + tool schemas vs the light turn's
/// persona. `cargo test light_vs_full -- --nocapture` prints them.
#[test]
fn light_vs_full_prompt_size() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine_on("http://127.0.0.1:9", dir.path());
    let light = engine.light_system_prompt();
    let (msgs, allowed, _) =
        engine.prepare_loop_messages("x", true, crate::engine::prompt::PromptPurpose::Preview);
    let system: String = msgs
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.as_str())
        .collect();
    let schemas =
        serde_json::to_string(&engine.offered_tool_definitions(allowed.as_ref())).unwrap();
    let tokens = AgentEngine::estimate_tokens_for_text;
    println!(
        "light persona ≈{} tokens; full system ≈{} + tool schemas ≈{} tokens",
        tokens(&light),
        tokens(&system),
        tokens(&schemas)
    );
    assert!(tokens(&light) * 3 < tokens(&system) + tokens(&schemas));
}
