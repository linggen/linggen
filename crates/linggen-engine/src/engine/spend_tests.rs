//! Every model call that reported usage is in the session's ledger — a call
//! that finished, was stopped, failed half-way, or was a summary: the person
//! is billed for what the provider counted either way. A fake Anthropic host
//! sends the scripted frames of each case.

use crate::engine::agent::AgentManager;
use crate::engine::{AgentEngine, AgentRole, EngineConfig, InterfaceMode};
use crate::message::ChatMessage;
use crate::provider::models::{ModelManager, Tokens};
use axum::{
    body::Body,
    extract::Path,
    http::StatusCode,
    response::Response,
    routing::{get, post},
    Router,
};
use futures_util::StreamExt;
use std::sync::Arc;

pub(crate) fn frame(name: &str, data: serde_json::Value) -> String {
    format!("event: {name}\ndata: {data}\n\n")
}

/// `message_start` with the prompt counted: 1,200 fresh, 3,000 written,
/// 5,000 read.
pub(crate) fn started() -> String {
    frame(
        "message_start",
        serde_json::json!({"type": "message_start", "message": {"usage": {
            "input_tokens": 1_200, "cache_creation_input_tokens": 3_000,
            "cache_read_input_tokens": 5_000, "output_tokens": 1}}}),
    )
}

fn text_frames() -> String {
    frame(
        "content_block_start",
        serde_json::json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
    ) + &frame(
        "content_block_delta",
        serde_json::json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hi"}}),
    )
}

fn finished() -> String {
    frame(
        "message_delta",
        serde_json::json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 400}}),
    ) + &frame("message_stop", serde_json::json!({"type": "message_stop"}))
}

/// What the host does after `message_start`, by the path a model's url ends in.
async fn messages(Path(mode): Path<String>) -> Response {
    let (head, tail): (String, Option<String>) = match mode.as_str() {
        // The provider's own error event, mid-stream.
        "fails" => (
            started() + &text_frames(),
            Some(frame(
                "error",
                serde_json::json!({"type": "error", "error": {"message": "overloaded"}}),
            )),
        ),
        // Silence after the prompt was counted, until the caller gives up.
        "hangs" => (started() + &text_frames(), None),
        // Nothing counted at all.
        "silent" => (text_frames(), None),
        _ => (started() + &text_frames(), Some(finished())),
    };
    let first = futures_util::stream::once(async move { Ok::<_, std::io::Error>(head) });
    let rest: futures_util::stream::BoxStream<'static, Result<String, std::io::Error>> = match tail
    {
        Some(tail) => futures_util::stream::once(async move { Ok(tail) }).boxed(),
        None => futures_util::stream::pending().boxed(),
    };
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(first.chain(rest)))
        .unwrap()
}

pub(crate) async fn host() -> String {
    let app = Router::new()
        .route(
            "/{mode}/v1/models/{id}",
            get(|| async { StatusCode::NOT_FOUND }),
        )
        .route("/{mode}/v1/messages", post(messages));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

pub(crate) fn anthropic(id: &str, url: &str) -> crate::config::ModelConfig {
    serde_json::from_value(serde_json::json!({
        "id": id, "provider": "anthropic", "url": url, "model": "claude-sonnet-5-5",
        "api_key": "k", "supports_tools": true, "context_window": 200_000,
    }))
    .unwrap()
}

pub(crate) struct Case {
    pub engine: AgentEngine,
    pub manager: Arc<AgentManager>,
    pub session: String,
    pub run_id: String,
    pub _root: tempfile::TempDir,
}

impl Case {
    pub fn ledger(&self, model: &str) -> Option<crate::state_fs::session_usage::LedgerEntry> {
        let mut entries = self
            .manager
            .global_sessions
            .session_usage(&self.session)
            .entries
            .into_iter()
            .filter(|e| e.model == model);
        let first = entries.next();
        assert!(entries.next().is_none(), "one entry expected for {model}");
        first
    }
}

/// An engine in a stored session and run, on one Anthropic model whose url
/// the host answers as `mode`.
pub(crate) async fn case(mode: &str) -> Case {
    let url = format!("{}/{mode}", host().await);
    let root = tempfile::tempdir().unwrap();
    let config = crate::config::Config::default();
    let cfg = EngineConfig::from_app_config(&config, root.path().into(), InterfaceMode::Web);
    let id = format!("spend-{mode}");
    let models = Arc::new(ModelManager::new_with_credentials(
        vec![anthropic(&id, &url)],
        &crate::credentials::Credentials::default(),
    ));
    let mut engine = AgentEngine::new(cfg, models, id, AgentRole::Lead).unwrap();
    engine.agent_id = Some("ling".into());
    let (mut manager, _events) = AgentManager::new(
        config,
        None,
        Arc::new(crate::engine::test_registries::Empty),
        Arc::new(crate::engine::test_registries::Empty),
        Arc::new(crate::engine::test_registries::Empty),
        InterfaceMode::Web,
    );
    Arc::get_mut(&mut manager)
        .expect("a fresh manager")
        .global_sessions =
        crate::state_fs::SessionStore::leaked(root.path().join("sessions"));
    let session = format!("s-spend-{mode}");
    manager
        .global_sessions
        .add_session(&crate::state_fs::sessions::SessionMeta {
            id: session.clone(),
            model_fallback: Some(false),
            effort: None,
            agents: vec![crate::state_fs::sessions::SessionMember::new("ling")],
            ..Default::default()
        })
        .unwrap();
    let run_id = manager
        .begin_agent_run(root.path(), Some(&session), "ling", None, None)
        .await
        .unwrap();
    engine.set_manager_context(manager.clone());
    engine.session_id = Some(session.clone());
    engine.set_run_id(Some(run_id.clone()));
    Case {
        engine,
        manager,
        session,
        run_id,
        _root: root,
    }
}

fn hi() -> Vec<ChatMessage> {
    vec![ChatMessage::new("user", "hello")]
}

const PROMPT: Tokens = Tokens {
    input: 1_200,
    output: 0,
    cache_read: 5_000,
    cache_write: 3_000,
};

#[tokio::test]
async fn a_call_that_finished_is_recorded_once() {
    let mut c = case("ok").await;
    c.engine.stream_with_fallback(&hi(), None).await.unwrap();
    let spent = c.ledger("spend-ok").expect("recorded");
    assert_eq!(spent.tally.calls, 1);
    assert_eq!(
        spent.tally.tokens,
        Tokens {
            output: 400,
            ..PROMPT
        }
    );
    // The class the call had when it was made, kept with it.
    assert_eq!(
        spent.billing,
        Some(crate::provider::models::Billing::Metered)
    );
}

/// Finding 1: the provider counted the prompt, then failed mid-stream — the
/// call was billed, and the ledger says so.
#[tokio::test]
async fn a_call_that_failed_half_way_is_recorded_with_what_was_counted() {
    let mut c = case("fails").await;
    let err = c
        .engine
        .stream_with_fallback(&hi(), None)
        .await
        .err()
        .expect("the stream failed");
    assert!(err.to_string().contains("overloaded"), "{err}");
    let spent = c
        .ledger("spend-fails")
        .expect("the failed call is in the ledger");
    assert_eq!((spent.tally.calls, spent.tally.unreported), (1, 0));
    assert_eq!(
        spent.tally.tokens, PROMPT,
        "the prompt as counted; output was never reported"
    );
}

/// Finding 1: a stop while the model is still writing.
#[tokio::test]
async fn a_call_the_person_stopped_is_recorded_with_what_was_counted() {
    let mut c = case("hangs").await;
    let (manager, run) = (c.manager.clone(), c.run_id.clone());
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        manager.cancel_run_tree(&run).await.unwrap();
    });
    let err = c
        .engine
        .stream_with_fallback(&hi(), None)
        .await
        .err()
        .expect("stopped");
    assert!(err.to_string().contains("cancelled"), "{err}");
    let spent = c
        .ledger("spend-hangs")
        .expect("the stopped call is in the ledger");
    assert_eq!((spent.tally.calls, spent.tally.unreported), (1, 0));
    assert_eq!(spent.tally.tokens, PROMPT);
}

/// Finding 1: a call that was opened and ended before any usage arrived is
/// one call that reported nothing.
#[tokio::test]
async fn a_call_stopped_before_any_usage_counts_as_one_unreported_call() {
    let mut c = case("silent").await;
    let (manager, run) = (c.manager.clone(), c.run_id.clone());
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        manager.cancel_run_tree(&run).await.unwrap();
    });
    c.engine
        .stream_with_fallback(&hi(), None)
        .await
        .err()
        .expect("stopped");
    let spent = c.ledger("spend-silent").expect("counted");
    assert_eq!((spent.tally.calls, spent.tally.unreported), (1, 1));
    assert!(spent.tally.tokens.is_zero());
}

// ---------------------------------------------------------------------------
// What a call cost is fixed when the call is made: a later change of config
// or price re-prices nothing already made.
// ---------------------------------------------------------------------------

/// A key provider's call is kept as tokens: no price is looked up, none is
/// added up, and no cap stops a call.
#[tokio::test]
async fn a_key_providers_call_is_tokens_only() {
    let mut c = case("ok").await;
    c.engine.stream_with_fallback(&hi(), None).await.unwrap();
    c.engine.stream_with_fallback(&hi(), None).await.unwrap();
    let spend = c.manager.session_spend(&c.session).expect("a ledger");
    assert!(spend.spent);
    assert_eq!(spend.cost_usd, None);
    assert_eq!(spend.tokens.input, 2 * 1_200);
}
