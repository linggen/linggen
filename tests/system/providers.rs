//! Provider settings only project the configured groups and stored key presence.

use crate::support::{Setup, World};
use serde_json::{json, Value};

#[track_caller]
fn provider<'a>(body: &'a Value, label: &str) -> &'a Value {
    body["providers"].as_array().unwrap().iter().find(|row| row["label"] == label).unwrap()
}

#[tokio::test]
async fn providers_group_by_sign_in_and_address_without_returning_keys() {
    const ROW: &str = r#"
[[models]]
id = "own-deepseek"
provider = "openai"
url = "https://api.deepseek.com/v1"
model = "deepseek-chat"
"#;
    let w = World::start(Setup { config_append: Some(ROW), ..Setup::default() }).await;
    let credentials = json!({"endpoints": [
        {"provider": "openai", "url": "https://api.openai.com/v1", "api_key": "test-only-marker"},
        {"provider": "openai", "url": "https://api.deepseek.com/v1/", "api_key": "test-only-marker"}
    ]});
    assert_eq!(w.api.put_json("/api/credentials", credentials).await.0, 200);
    let body = w.api.get("/api/providers").await;
    assert!(!body.to_string().contains("test-only-marker"));
    assert!(!body.to_string().contains("api_key"));
    let openai = body["providers"].as_array().unwrap().iter()
        .find(|p| p["url"] == "https://api.openai.com/v1").unwrap();
    assert_eq!(openai["models"], json!([]));
    assert_eq!(openai["key"], json!({"set": true}));
    let deepseek = provider(&body, "DeepSeek");
    assert_ne!(deepseek["id"], openai["id"]);
    assert_eq!(deepseek["models"][0]["id"], "own-deepseek");
    assert_eq!(deepseek["models"][0]["ready"], true);
    assert_eq!(provider(&body, "Linggen Cloud")["way"], "account");
    let chatgpt = provider(&body, "ChatGPT 订阅");
    assert_eq!(chatgpt["way"], "subscription");
    assert!(chatgpt.get("key").is_none());
    // Background discovery may change family evidence, but never changes the grouping.
    let groups = |mut answer: Value| {
        for row in answer["providers"].as_array_mut().unwrap() {
            row.as_object_mut().unwrap().remove("families");
        }
        answer
    };
    assert_eq!(groups(w.api.get("/api/providers").await), groups(body));
}

#[tokio::test]
async fn an_address_edit_moves_the_key_and_every_model() {
    let w = World::start(Setup::default()).await;
    let body = w.api.post("/api/providers", json!({"preset":"DeepSeek"})).await;
    let id = provider(&body, "DeepSeek")["id"].as_str().unwrap();
    let base = format!("/api/providers/{id}");
    let (status, body) = w.api.put_json(&format!("{base}/key"), json!({"api_key":"test-only-marker"})).await;
    assert_eq!(status, 200);
    assert_eq!(provider(&body, "DeepSeek")["key"]["set"], true);
    for id in ["first", "second"] {
        let body = w.api.post(&format!("{base}/models"), json!({"id":id,"model":"deepseek-chat"})).await;
        assert!(provider(&body, "DeepSeek")["models"].as_array().unwrap().iter().any(|m| m["id"] == id && m["ready"] == true));
    }
    let before = w.api.get("/api/providers").await;
    assert_eq!(w.api.post_status(&format!("{base}/models"), json!({"id":"first","model":"another"})).await, 409);
    assert_eq!(w.api.get("/api/providers").await, before);
    let body = w.api.patch(&base, json!({"url":"https://example.invalid/v1"})).await;
    let new = provider(&body, "DeepSeek");
    assert_ne!(new["id"], id);
    assert_eq!(new["url"], "https://example.invalid/v1");
    assert_eq!(new["key"]["set"], true);
    assert_eq!(new["models"].as_array().unwrap().len(), 2);
    assert!(new["models"].as_array().unwrap().iter().all(|m| m["ready"] == true));
    assert!(!body["providers"].as_array().unwrap().iter().any(|p| p["id"] == id));
    assert!(!body.to_string().contains("test-only-marker"));
    assert_eq!(w.api.get("/api/providers").await, body);
}

#[tokio::test]
async fn deleting_the_last_voice_row_files_its_override_as_the_endpoint_key() {
    let w = World::start(Setup::default()).await;
    let body = w.api.post("/api/providers", json!({"preset":"Qwen Cloud"})).await;
    let id = provider(&body, "Qwen Cloud")["id"].as_str().unwrap();
    let base = format!("/api/providers/{id}/models");
    let body = w.api.post(&base, json!({"id":"voice","model":"qwen-audio-3.1-realtime-flash"})).await;
    assert_eq!(provider(&body, "Qwen Cloud")["models"][0]["kind"], "live_voice");
    assert_eq!(w.api.put_json("/api/credentials", json!({"models":{"voice":{"api_key":"test-only-marker"}}})).await.0, 200);
    let body = w.api.delete(&format!("{base}/voice"), json!({})).await;
    let qwen = provider(&body, "Qwen Cloud");
    assert_eq!(qwen["models"].as_array().unwrap().len(), 1);
    assert_eq!(qwen["models"][0]["id"], "live:alibaba");
    assert_eq!(qwen["key"]["set"], true);
    assert!(!body.to_string().contains("test-only-marker"));
    assert_eq!(w.api.get("/api/credentials").await["models"], json!({}));
    assert_eq!(w.api.get("/api/live").await["providers"][0]["ready"], true);
}

async fn file_key(w: &World, label: &str) -> Value {
    let body = w.api.get("/api/providers").await;
    let id = provider(&body, label)["id"].as_str().unwrap();
    let (status, body) = w.api.put_json(&format!("/api/providers/{id}/key"), json!({"api_key":"test-only-marker"})).await;
    assert_eq!(status, 200);
    body
}

fn voice<'a>(body: &'a Value, label: &str, model: &str) -> &'a Value {
    let rows: Vec<_> = provider(body, label)["models"].as_array().unwrap().iter()
        .filter(|row| row["kind"] == "live_voice").collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["model"], model);
    rows[0]
}

#[tokio::test]
async fn gemini_live_is_first_when_only_google_is_ready() {
    let w = World::start(Setup::default()).await;
    let model = "gemini-3.8-live";
    assert_eq!(voice(&w.api.get("/api/providers").await, "Google Gemini", model)["ready"], false);
    let body = file_key(&w, "Google Gemini").await;
    assert_eq!(voice(&body, "Google Gemini", model)["first"], true);
    assert_eq!(voice(&body, "Google Gemini", model)["ready"], w.api.get("/api/live").await["providers"][1]["ready"]);
    assert!(!w.api.get("/api/models").await.to_string().contains(model));
}

#[tokio::test]
async fn qwen_live_is_first_when_both_are_ready() {
    let w = World::start(Setup::default()).await;
    let model = "qwen-audio-3.1-realtime-flash";
    assert_eq!(voice(&w.api.get("/api/providers").await, "Qwen Cloud", model)["ready"], false);
    file_key(&w, "Google Gemini").await;
    let body = file_key(&w, "Qwen Cloud").await;
    assert_eq!(voice(&body, "Qwen Cloud", model)["first"], true);
    assert_eq!(voice(&body, "Qwen Cloud", model)["ready"], w.api.get("/api/live").await["providers"][0]["ready"]);
    assert!(voice(&body, "Google Gemini", "gemini-3.8-live").get("first").is_none());
    assert!(!w.api.get("/api/models").await.to_string().contains(model));
}

#[tokio::test]
async fn declared_live_rows_at_socket_addresses_belong_only_to_the_preset_groups() {
    const ROWS: &str = r#"
[[models]]
id = "qwen-socket"
provider = "qwen"
url = "wss://ap-southeast-1.maas.qwencloudapi.com/api-ws/v1/realtime"
model = "qwen-audio-3.1-realtime-flash"

[[models]]
id = "google-socket"
provider = "gemini"
url = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContentConstrained"
model = "gemini-3.8-live"
"#;
    let w = World::start(Setup { config_append: Some(ROWS), ..Setup::default() }).await;
    let body = w.api.get("/api/providers").await;
    for (host, label, model) in [
        ("ap-southeast-1.maas.qwencloudapi.com", "Qwen Cloud", "qwen-audio-3.1-realtime-flash"),
        ("generativelanguage.googleapis.com", "Google Gemini", "gemini-3.8-live"),
    ] {
        let groups: Vec<_> = body["providers"].as_array().unwrap().iter()
            .filter(|p| p["url"].as_str().is_some_and(|url| url.contains(host))).collect();
        assert_eq!(groups.len(), 1, "{label}: {groups:?}");
        assert_eq!(groups[0]["label"], label);
        assert!(groups[0]["families"].is_object());
        voice(&body, label, model);
    }
}

const QWEN_URL: &str = "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1";
const QWEN_ALIASES: &str = r#"
[[models]]
id = "own-flash"
provider = "openai"
url = "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1"
model = "deepseek-v4.1-flash"
"#;

#[tokio::test]
async fn qwen_aliases_with_the_same_key_share_one_card() {
    let w = World::start(Setup { config_append: Some(QWEN_ALIASES), ..Setup::default() }).await;
    assert_eq!(w.api.put_json("/api/credentials", json!({"endpoints":[
        {"provider":"qwen", "url":format!("{QWEN_URL}/"), "api_key":"model-key-marker"},
        {"provider":"openai", "url":QWEN_URL, "api_key":"model-key-marker"}
    ]})).await.0, 200);
    let body = w.api.get("/api/providers").await;
    assert_eq!(body["providers"].as_array().unwrap().iter().filter(|p| p["url"] == QWEN_URL).count(), 1);
    let qwen = provider(&body, "Qwen Cloud");
    assert_eq!(qwen["key"], json!({"set":true}));
    assert_eq!(qwen["provider"], "qwen");
    assert!(qwen["models"].as_array().unwrap().iter().any(|m| m["id"] == "own-flash" && m["ready"] == true));
    assert_eq!(voice(&body, "Qwen Cloud", "qwen-audio-3.1-realtime-flash")["ready"], true);
    assert!(!body.to_string().contains("model-key-marker"));
}

#[tokio::test]
async fn removing_the_key_of_a_card_removes_it_under_every_name() {
    let w = World::start(Setup { config_append: Some(QWEN_ALIASES), ..Setup::default() }).await;
    assert_eq!(w.api.put_json("/api/credentials", json!({"endpoints":[
        {"provider":"qwen", "url":format!("{QWEN_URL}/"), "api_key":"model-key-marker"},
        {"provider":"openai", "url":QWEN_URL, "api_key":"model-key-marker"}
    ]})).await.0, 200);
    let body = w.api.get("/api/providers").await;
    let route = format!("/api/providers/{}/key", provider(&body, "Qwen Cloud")["id"].as_str().unwrap());
    let (status, body) = w.api.put_json(&route, json!({"api_key": null})).await;
    assert_eq!(status, 200);
    assert_eq!(provider(&body, "Qwen Cloud")["key"], json!({"set":false}));
}

#[tokio::test]
async fn replacing_the_key_of_a_card_keeps_every_row_under_every_name_ready() {
    const ROWS: &str = concat!(r#"
[[models]]
id = "own-flash"
provider = "openai"
url = "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1"
model = "deepseek-v4.1-flash"

[[models]]
id = "own-plus"
provider = "qwen"
url = "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1"
model = "qwen-plus"
"#);
    let w = World::start(Setup { config_append: Some(ROWS), ..Setup::default() }).await;
    assert_eq!(w.api.put_json("/api/credentials", json!({"endpoints":[
        {"provider":"qwen", "url":QWEN_URL, "api_key":"old-key-marker"},
        {"provider":"openai", "url":QWEN_URL, "api_key":"old-key-marker"}
    ]})).await.0, 200);
    let body = w.api.get("/api/providers").await;
    let route = format!("/api/providers/{}/key", provider(&body, "Qwen Cloud")["id"].as_str().unwrap());
    let (status, body) = w.api.put_json(&route, json!({"api_key": "new-key-marker"})).await;
    assert_eq!(status, 200);
    let qwen = provider(&body, "Qwen Cloud");
    assert_eq!(qwen["key"], json!({"set":true}));
    assert!(qwen["models"].as_array().unwrap().iter().all(|m| m["ready"] == true), "{qwen}");
}

#[tokio::test]
async fn cleanup_refuses_when_two_model_rows_use_different_keys() {
    const ROWS: &str = concat!(r#"
[[models]]
id = "own-flash"
provider = "openai"
url = "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1"
model = "deepseek-v4.1-flash"

[[models]]
id = "own-plus"
provider = "qwen"
url = "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1"
model = "qwen-plus"
"#);
    let w = World::start(Setup { config_append: Some(ROWS), ..Setup::default() }).await;
    assert_eq!(w.api.put_json("/api/credentials", json!({"endpoints":[
        {"provider":"qwen", "url":QWEN_URL, "api_key":"qwen-key-marker"},
        {"provider":"openai", "url":QWEN_URL, "api_key":"model-key-marker"}
    ]})).await.0, 200);
    let body = w.api.get("/api/providers").await;
    let qwen = provider(&body, "Qwen Cloud");
    assert_eq!(qwen["key"], json!({"set":true,"conflict":true}));
    let route = format!("/api/providers/{}/key", qwen["id"].as_str().unwrap());
    assert_eq!(w.api.put_json(&route, json!({"action":"keep_model_key"})).await.0, 409);
    let body = w.api.get("/api/providers").await;
    assert!(provider(&body, "Qwen Cloud")["models"].as_array().unwrap().iter().all(|m| m["ready"] == true));
}

#[tokio::test]
async fn qwen_cloud_borrows_only_listed_candidates_and_never_dashscope_rates() {
    const ROW: &str = r#"
[[models]]
id = "my-deepseek"
provider = "qwen"
url = "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1"
model = "deepseek-v4.1-flash"
"#;
    let w = World::start_with_home(Setup { config_append: Some(ROW), ..Setup::default() }, |home| {
        let fixture = json!({"today":"2026-11-23", "lists": {
            "qwen-cloud": ["qwen3.8-max", "qwen-plus", "deepseek-v4-flash-0731", "kimi-k3"],
            "dashscope-cn": ["qwen3.8-max"]
        }});
        std::fs::write(home.linggen_home().join("test-families.json"), fixture.to_string()).unwrap();
    }).await;
    let before = w.api.get("/api/providers").await;
    assert_eq!(provider(&before, "Qwen Cloud")["families"]["families"], json!([]));
    let body = file_key(&w, "Qwen Cloud").await;
    let qwen = provider(&body, "Qwen Cloud");
    let families = qwen["families"]["families"].as_array().unwrap();
    assert_eq!(families.len(), 4, "{families:?}");
    assert!(families.iter().all(|f| f["next"].get("price").is_none()));
    assert!(families.iter().any(|f| f["next"]["model"] == "qwen-plus"));
    assert!(qwen["models"].as_array().unwrap().iter().any(|m| m["id"] == "my-deepseek" && m["ready"] == true));
    w.api.post("/api/providers", json!({"preset":"通义千问 (DashScope)"})).await;
    let body = file_key(&w, "通义千问 (DashScope)").await;
    let dashscope = provider(&body, "通义千问 (DashScope)");
    let max = dashscope["families"]["families"].as_array().unwrap().iter().find(|f| f["family"] == "qwen-max").unwrap();
    assert_eq!(max["next"]["price"]["source"], "catalog");
}

/// The page types a model name only; the engine makes the row's id, and a
/// second one of the same name is its own row.
#[tokio::test]
async fn a_typed_model_without_an_id_gets_one_from_the_engine() {
    let w = World::start(Setup::default()).await;
    let body = w.api.post("/api/providers", json!({"preset":"DeepSeek"})).await;
    let id = provider(&body, "DeepSeek")["id"].as_str().unwrap().to_string();
    let base = format!("/api/providers/{id}/models");
    let first = w.api.post(&base, json!({"model":"deepseek-chat"})).await;
    let second = w.api.post(&base, json!({"model":"deepseek-chat"})).await;
    let rows = provider(&second, "DeepSeek")["models"].as_array().unwrap().clone();
    assert_eq!(rows.len(), 2, "{first}");
    assert!(rows.iter().all(|m| m["model"] == "deepseek-chat" && m["id"].as_str().unwrap().starts_with("own-")));
    assert_ne!(rows[0]["id"], rows[1]["id"]);
}
