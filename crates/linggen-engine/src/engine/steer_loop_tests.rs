//! Steer during a model reply: the stream stops, what it said stays as an
//! assistant message, the steered message joins, and the turn goes on.
use crate::engine::steer::SteerSignal;
use crate::engine::{AgentEngine, AgentRole, EngineConfig, InboxMessage, InterfaceMode};
use crate::provider::models::ModelManager;
use axum::{body::Body, extract::State, routing::post, Json, Router};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Default)]
struct Fake {
    requests: Arc<Mutex<Vec<serde_json::Value>>>,
}

fn line(content: &str, done: bool) -> String {
    let row =
        serde_json::json!({"message": {"role": "assistant", "content": content}, "done": done});
    format!("{row}\n")
}

/// The first reply says a little, then hangs as a long reply would; the
/// next answers at once.
async fn chat(State(fake): State<Fake>, Json(body): Json<serde_json::Value>) -> Body {
    let first = {
        let mut requests = fake.requests.lock().unwrap();
        requests.push(body);
        requests.len() == 1
    };
    if !first {
        return Body::from(line("Answered the new message.", false) + &line("", true));
    }
    let chunks = futures_util::stream::unfold(0, |step| async move {
        if step > 0 {
            tokio::time::sleep(Duration::from_secs(30)).await;
            return None;
        }
        Some((
            Ok::<_, std::convert::Infallible>(line("Here is the long story, part one", false)),
            1,
        ))
    });
    Body::from_stream(chunks)
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
        provided_by: None,
        is_builtin: false,
    }
}

#[tokio::test]
async fn steer_during_a_reply_keeps_what_it_said_and_takes_the_message() {
    let fake = Fake::default();
    let app = Router::new()
        .route("/api/chat", post(chat))
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
    let id = format!("steer-test-{}", url.rsplit(':').next().unwrap());
    let mut m = model("story", &url);
    m.id = id.clone();
    let models = Arc::new(ModelManager::new_with_credentials(
        vec![m],
        &crate::credentials::Credentials::default(),
    ));
    let mut engine = AgentEngine::new(cfg, models, id.clone(), AgentRole::Lead).unwrap();
    engine.auto_fallback = false;
    engine.set_task("tell me a long story".to_string());

    let steer = Arc::new(SteerSignal::default());
    engine.tools.builtins.set_steer(Some(steer.clone()));
    let steered = Arc::new(AtomicBool::new(false));
    let waiting = Arc::new(Mutex::new(None::<InboxMessage>));
    let (taken, mut accepted) = tokio::sync::oneshot::channel();
    *waiting.lock().unwrap() = Some(InboxMessage {
        text: "stop, just say hi".into(),
        images: vec![],
        taken,
    });
    engine.inbox = Some(Arc::new({
        let (steered, waiting) = (steered.clone(), waiting.clone());
        move || {
            let ready = steered.load(Ordering::SeqCst);
            let message = if ready {
                waiting.lock().unwrap().take()
            } else {
                None
            };
            Box::pin(async move { message.into_iter().collect() })
        }
    }));

    let fire = {
        let (fake, steer, steered) = (fake.clone(), steer.clone(), steered.clone());
        tokio::spawn(async move {
            while fake.requests.lock().unwrap().is_empty() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
            steered.store(true, Ordering::SeqCst);
            steer.fire();
        })
    };
    tokio::time::timeout(Duration::from_secs(10), engine.run_agent_loop(None))
        .await
        .expect("the steered reply did not wait out the hanging stream")
        .unwrap();
    fire.await.unwrap();

    let requests = fake.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2, "the turn went on with the message");
    let sent = requests[1]["messages"].as_array().unwrap();
    let tail: Vec<(&str, &str)> = sent[sent.len() - 2..]
        .iter()
        .map(|m| (m["role"].as_str().unwrap(), m["content"].as_str().unwrap()))
        .collect();
    assert_eq!(
        tail,
        [
            ("assistant", "Here is the long story, part one"),
            ("user", "stop, just say hi")
        ]
    );
    assert_eq!(accepted.try_recv(), Ok(()), "the message was taken in");
    assert!(!steer.pending(), "the step that took it settled the steer");
    assert_eq!(
        engine.last_assistant_text.as_deref(),
        Some("Answered the new message.")
    );
}
