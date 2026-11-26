//! What a session spent (`doc/models.md` § What a session cost): a model call
//! on Anthropic's wire reports its tokens, the engine keeps them per session
//! and model, as tokens: no price is looked up or kept. The host here is a stand-in that answers every Messages request
//! with one scripted usage block — no real model, no real key.

use crate::support::{Setup, World};
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

const INPUT: u64 = 1_200;
const CACHE_WRITE: u64 = 3_000;
const CACHE_READ: u64 = 5_000;
const OUTPUT: u64 = 400;

fn script() -> String {
    let start = json!({"type": "message_start", "message": {"id": "m1", "type": "message", "role": "assistant",
        "model": "stub", "content": [], "usage": {"input_tokens": INPUT,
        "cache_creation_input_tokens": CACHE_WRITE, "cache_read_input_tokens": CACHE_READ, "output_tokens": 1}}});
    let block = json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}});
    let delta = json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Done."}});
    let stop = json!({"type": "content_block_stop", "index": 0});
    let end = json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": OUTPUT}});
    let done = json!({"type": "message_stop"});
    [
        ("message_start", start),
        ("content_block_start", block),
        ("content_block_delta", delta),
        ("content_block_stop", stop),
        ("message_delta", end),
        ("message_stop", done),
    ]
    .iter()
    .map(|(name, data)| format!("event: {name}\ndata: {data}\n\n"))
    .collect()
}

async fn messages(State(seen): State<Arc<AtomicUsize>>) -> impl IntoResponse {
    seen.fetch_add(1, Ordering::SeqCst);
    ([("content-type", "text/event-stream")], script())
}

/// The stand-in host: its address, and how many Messages requests it got.
pub(crate) async fn host() -> (String, Arc<AtomicUsize>) {
    let seen = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/v1/models/{id}", get(|| async { StatusCode::NOT_FOUND }))
        .route("/v1/messages", post(messages))
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, seen)
}

async fn world(url: &str) -> World {
    let models = format!(
        r#"
[[models]]
id = "stub-priced"
provider = "anthropic"
url = "{url}"
model = "claude-sonnet-5-5"
api_key = "test-not-a-key"
context_window = 200000
supports_tools = true
price = {{ input = 2.0, output = 10.0, cache_read = 0.1, cache_write = 2.5 }}

# The same model name behind a gateway: Anthropic's table does not apply.
[[models]]
id = "stub-gateway"
provider = "anthropic"
url = "{url}"
model = "claude-sonnet-5-5"
api_key = "test-not-a-key"
context_window = 200000
supports_tools = true
"#
    );
    World::start(Setup {
        config_append: Some(Box::leak(models.into_boxed_str())),
        ..Setup::default()
    })
    .await
}

async fn run_on(w: &World, model: &str) -> String {
    let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "agent_run",
        "arguments": {"prompt": "Say done.", "write_dir": w.work(), "work": "none", "model": model}}});
    let reply = w.api.post("/mcp", call).await;
    let said = reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(said.contains("Done."), "{reply}");
    let dir = w.home.linggen_home().join("sessions");
    let mut made: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|id| {
            std::fs::read_to_string(dir.join(id).join("session.yaml"))
                .unwrap_or_default()
                .contains("creator: external")
                && std::fs::read_to_string(dir.join(id).join("session.yaml"))
                    .unwrap_or_default()
                    .contains(model)
        })
        .collect();
    assert_eq!(made.len(), 1, "one session on {model}: {made:?}");
    made.remove(0)
}

async fn status(w: &World, session: &str) -> Value {
    let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "agent_status", "arguments": {"op": "session", "session_id": session}}});
    let reply = w.api.post("/mcp", call).await;
    serde_json::from_str(
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{reply}")),
    )
    .unwrap()
}

async fn page_state(w: &World, session: &str) -> Value {
    w.api
        .get(&format!(
            "/api/workspace/state?session_id={session}&project_root={}",
            urlencoding::encode(&w.work())
        ))
        .await
}

/// A key model's calls: what each call reported lands in the session's
/// ledger per model, as tokens; no dollar figure, even with a `price` left
/// in the config from before (it is ignored), and `agent_status` and the
/// page's session state tell the same facts.
#[tokio::test]
async fn a_key_models_calls_are_kept_as_tokens_and_never_priced() {
    let (url, seen) = host().await;
    let w = world(&url).await;
    let sid = run_on(&w, "stub-priced").await;
    let calls = seen.load(Ordering::SeqCst) as u64;
    assert!(calls >= 1);

    let fact = status(&w, &sid).await;
    let usage = &fact["usage"];
    let m = &usage["models"][0];
    assert_eq!(usage["models"].as_array().unwrap().len(), 1, "{usage}");
    assert_eq!(m["model"], "stub-priced");
    assert_eq!(m["billing"], "metered");
    assert_eq!(m["calls"], calls);
    assert_eq!(m["input"], calls * INPUT);
    assert_eq!(m["cache_write"], calls * CACHE_WRITE);
    assert_eq!(m["cache_read"], calls * CACHE_READ);
    assert_eq!(m["output"], calls * OUTPUT);
    assert!(m.get("cost_usd").is_none() && usage.get("cost_usd").is_none(), "{usage}");
    assert_eq!(usage["tokens"]["input"], calls * INPUT);

    // The page asks for the same facts, from the same place.
    let page = page_state(&w, &sid).await;
    assert_eq!(
        page["usage"], *usage,
        "the top bar and agent_status tell one story"
    );
}

/// Finding 2: the `/compact` button is a model call like any other. A
/// session's ledger counts the two turns and the summary: three calls.
#[tokio::test]
async fn a_manual_compaction_is_a_call_in_the_ledger() {
    use crate::support::model::{text, Agent};
    let w = World::start(
        Setup::default()
            .script(Agent::Ling, vec![text("OK."), text("Fine.")])
            .extra("context compaction assistant", "SUMMARY: filler was sent."),
    )
    .await;
    let sid = w.api.create_session(json!({"title": "Long"})).await;
    w.turn(
        &sid,
        &w.work(),
        &format!("Ignore this filler.\n{}", crate::shared_table::filler()),
        Agent::Ling,
    )
    .await;
    w.turn(&sid, &w.work(), "Short one.", Agent::Ling).await;
    let file = w
        .home
        .linggen_home()
        .join("sessions")
        .join(&sid)
        .join("usage.json");
    let calls = || -> u64 {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap_or_default())
            .unwrap_or(json!({}));
        v["entries"].as_array().into_iter().flatten().filter(|e| e["model"] == "fake-ling").filter_map(|e| e["calls"].as_u64()).sum()
    };
    assert_eq!(calls(), 2, "the two turns");
    let resp = w
        .api
        .post(
            "/api/chat/compact",
            json!({"session_id": sid, "project_root": w.work()}),
        )
        .await;
    assert_eq!(resp["compacted"], true, "{resp}");
    assert_eq!(calls(), 3, "the summary is a call of the summarizing model");
}

/// A model on this machine's network is listed in the ledger but is no
/// spend: no tokens in the total, no dollars, nothing to name as unpriced.
/// (The fixture's fake models are `openai` on loopback.)
#[tokio::test]
async fn a_local_models_calls_are_listed_but_are_no_spend() {
    use crate::support::model::{text, Agent};
    let w = World::start(Setup::default().script(Agent::Ling, vec![text("Done.")])).await;
    let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "agent_run",
        "arguments": {"prompt": "Say done.", "write_dir": w.work(), "work": "none"}}});
    w.api.post("/mcp", call).await;
    let dir = w.home.linggen_home().join("sessions");
    let sid = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .find(|id| {
            std::fs::read_to_string(dir.join(id).join("session.yaml"))
                .unwrap_or_default()
                .contains("creator: external")
        })
        .expect("the delegated session");
    let usage = status(&w, &sid).await["usage"].clone();
    let m = &usage["models"][0];
    assert_eq!(m["model"], "fake-ling", "{usage}");
    assert_eq!(m["billing"], "local", "{usage}");
    assert!(m["calls"].as_u64().unwrap() >= 1);
    assert!(
        m.get("cost_usd").is_none() && usage.get("cost_usd").is_none(),
        "{usage}"
    );
    assert_eq!(
        usage["tokens"],
        json!({"input": 0, "output": 0, "cache_read": 0, "cache_write": 0, "prompt": 0}),
        "{usage}"
    );
    assert_eq!(usage["spent"], false, "{usage}");
    assert!(usage.get("unpriced").is_none(), "{usage}");
}

/// A config from before prices were dropped: its `price` is ignored, every
/// model and setting is the person's own.
#[tokio::test]
async fn an_old_price_in_the_file_is_ignored_and_the_rest_is_in_force() {
    let extra = r#"
[[models]]
id = "bad-price"
provider = "openai"
url = "{{MODEL_URL}}/v1"
model = "bad-price-1"
api_key = "test-not-a-key"
price = { input = -1.0, output = 2.0 }
"#;
    let w = World::start(Setup {
        config_append: Some(extra),
        ..Setup::default()
    })
    .await;
    let config = w.api.get("/api/config").await;
    let models = config["models"].as_array().unwrap();
    let ids: Vec<&str> = models.iter().filter_map(|m| m["id"].as_str()).collect();
    assert!(
        ids.contains(&"fake-ling") && ids.contains(&"bad-price"),
        "the person's models are still there: {ids:?}"
    );
    let bad = models.iter().find(|m| m["id"] == "bad-price").unwrap();
    assert!(bad.get("price").is_none(), "the price is not kept: {bad}");
    assert_eq!(
        config["routing"]["default_models"],
        json!(["fake-ling"]),
        "routing is the person's own"
    );
}

/// The older hazard (it is the config's, not the price's): a config file that
/// cannot be loaded is never written over by a save through the API — its
/// original bytes are kept under a set-aside name.
#[tokio::test]
async fn a_save_never_writes_over_a_config_file_that_cannot_load() {
    let w = World::start(Setup::default()).await;
    let config = w.api.get("/api/config").await;
    let path = w.home.config_path();
    let broken = b"[server\nurl = \"the person's own file, broken by a hand edit\"\n";
    std::fs::write(&path, broken).unwrap();
    w.api.post("/api/config", config).await;
    let dir = path.parent().unwrap();
    let aside: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .contains(".unreadable-")
        })
        .collect();
    assert_eq!(aside.len(), 1, "{aside:?}");
    assert_eq!(
        std::fs::read(&aside[0]).unwrap(),
        broken,
        "the original bytes are kept"
    );
    assert_ne!(
        std::fs::read(&path).unwrap(),
        broken,
        "the save wrote the new config"
    );
}
