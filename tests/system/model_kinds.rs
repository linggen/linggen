//! What a configured model is for (`provider::kinds`): a row at a declared
//! live-voice host with its model is live voice, and no list a chat model
//! is picked from holds it — while the live-voice routes still work.

use crate::support::{Setup, World};
use serde_json::{json, Value};

/// The Qwen live-voice row as a person files it: a made-up text model at the
/// live host. No key is on the row.
pub(crate) const LIVE_ROW: &str = r#"
[[models]]
id = "voice-row"
provider = "qwen"
url = "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1"
model = "qwen-audio-3.1-realtime-flash"
"#;

async fn world() -> World {
    World::start(Setup {
        config_append: Some(LIVE_ROW),
        env: vec![("LINGGEN_LIVE_ALIBABA_KEY", "test-not-a-key".into())],
        ..Setup::default()
    })
    .await
}

fn tool(name: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": name, "arguments": args}})
}

fn said(reply: &Value) -> String {
    reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn a_live_voice_row_is_in_no_list_a_chat_model_is_picked_from() {
    let w = world().await;

    // GET /api/models: the chat rows, never the live-voice one.
    let listed = w.api.get("/api/models").await.to_string();
    assert!(listed.contains("fake-ling"), "{listed}");
    assert!(!listed.contains("voice-row"), "{listed}");

    // agent_status op=models.
    let reply = w.api.post("/mcp", tool("agent_status", json!({"op": "models"}))).await;
    let models = said(&reply);
    assert!(models.contains("fake-ling") && !models.contains("voice-row"), "{models}");

    // agent_run model=: refused as a model this machine lacks, and the
    // models it offers are the chat ones.
    let reply = w
        .api
        .post("/mcp", tool("agent_run", json!({"prompt": "Say done.", "model": "voice-row"})))
        .await;
    let refusal = said(&reply);
    assert_eq!(reply["result"]["isError"], true, "{reply}");
    let (asked, available) = refusal.split_once("Available:").expect(&refusal);
    assert!(asked.contains("unknown model 'voice-row'"), "{refusal}");
    assert!(available.contains("fake-ling") && !available.contains("voice-row"), "{refusal}");
}

/// The page that stores a voice key still works as before: a Qwen live key
/// is found by host, whatever row or none carries it.
#[tokio::test]
async fn the_live_voice_routes_are_unchanged_by_a_live_voice_row() {
    let w = world().await;
    let listed = w.api.get("/api/live").await;
    assert_eq!(
        listed,
        json!({"providers": [{"id": "alibaba", "ready": true}]}),
        "{listed}"
    );
}
