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
    // Asking again changes nothing: nothing is discovered in the background.
    assert_eq!(w.api.get("/api/providers").await, body);
}

#[tokio::test]
async fn an_address_edit_moves_the_key_and_every_model() {
    let w = World::start(Setup::default()).await;
    // A typed provider (其他…), not a preset: a preset owns its own address.
    let listed = w.api.post("/api/providers", json!({"preset":"DeepSeek"})).await;
    let listed = provider(&listed, "DeepSeek");
    assert_eq!(listed["listed"], true);
    assert_eq!(w.api.patch_status(&format!("/api/providers/{}", listed["id"].as_str().unwrap()), json!({"url":"https://example.invalid/v1"})).await, 400);
    let body = w.api.post("/api/providers", json!({"name":"My gateway","url":"https://gateway.example.test/v1","style":"openai"})).await;
    let added = body["providers"].as_array().unwrap().iter().find(|p| p["url"] == "https://gateway.example.test/v1").unwrap();
    assert_eq!(added["listed"], false);
    let id = added["id"].as_str().unwrap();
    let base = format!("/api/providers/{id}");
    let (status, body) = w.api.put_json(&format!("{base}/key"), json!({"api_key":"test-only-marker"})).await;
    assert_eq!(status, 200);
    assert_eq!(body["providers"].as_array().unwrap().iter().find(|p| p["id"] == id).unwrap()["key"]["set"], true);
    for id in ["first", "second"] {
        let body = w.api.post(&format!("{base}/models"), json!({"id":id,"model":"deepseek-chat"})).await;
        assert!(body["providers"].as_array().unwrap().iter().any(|p| p["models"].as_array().unwrap().iter().any(|m| m["id"] == id && m["ready"] == true)));
    }
    let before = w.api.get("/api/providers").await;
    assert_eq!(w.api.post_status(&format!("{base}/models"), json!({"id":"first","model":"another"})).await, 409);
    assert_eq!(w.api.get("/api/providers").await, before);
    let body = w.api.patch(&base, json!({"url":"https://example.invalid/v1"})).await;
    let new = body["providers"].as_array().unwrap().iter().find(|p| p["url"] == "https://example.invalid/v1").unwrap();
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
    if label == "Google Gemini" {
        let (status, _) = w.api.put_json("/api/credentials", json!({"endpoints":[{"provider":"gemini","url":"https://generativelanguage.googleapis.com/v1beta/openai","api_key":"test-only-marker"}]})).await;
        assert_eq!(status, 200);
        return w.api.get("/api/providers").await;
    }
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
async fn gemini_live_is_not_offered_even_when_google_has_a_key() {
    let w = World::start(Setup::default()).await;
    let model = "gemini-3.8-live";
    let body = file_key(&w, "Google Gemini").await;
    assert!(!body.to_string().contains(model));
    assert_eq!(w.api.get("/api/live").await["providers"][0]["ready"], false);
    assert!(!w.api.get("/api/models").await.to_string().contains(model));
}

#[tokio::test]
async fn qwen_live_is_first_when_both_are_ready() {
    let w = World::start(Setup::default()).await;
    let model = "qwen-audio-3.1-realtime-flash";
    assert_eq!(voice(&w.api.get("/api/providers").await, "Qwen Cloud", model)["ready"], false);
    file_key(&w, "Google Gemini").await;
    let body = file_key(&w, "Qwen Cloud").await;
    assert!(voice(&body, "Qwen Cloud", model).get("first").is_none());
    assert_eq!(voice(&body, "Qwen Cloud", model)["ready"], w.api.get("/api/live").await["providers"][0]["ready"]);
    assert!(!body.to_string().contains("gemini-3.8-live"));
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
        if label == "Google Gemini" {
            assert!(groups.is_empty(), "{groups:?}");
            continue;
        }
        assert_eq!(groups.len(), 1, "{label}: {groups:?}");
        assert_eq!(groups[0]["label"], label);
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

// ── no list: presets, typed providers, suggestions, Linggen Cloud's ticks ──

/// The picker offers the engine's table; 其他… files a provider by its API
/// style, and a style or a field it does not know is refused.
#[tokio::test]
async fn presets_are_the_engines_table_and_any_other_provider_is_typed() {
    let w = World::start(Setup::default()).await;
    let body = w.api.get("/api/providers").await;
    let labels: Vec<&str> = body["presets"].as_array().unwrap().iter().map(|p| p["label"].as_str().unwrap()).collect();
    assert_eq!(labels, ["Anthropic", "OpenAI", "Google Gemini", "DeepSeek", "Qwen Cloud", "OpenRouter", "Groq", "Ollama", "ChatGPT 订阅"]);
    assert!(body.get("edition").is_none(), "no region choice");
    let made = w.api.post("/api/providers", json!({"name":"My Claude gateway","url":"https://claude-gw.example.test","style":"claude"})).await;
    let row = made["providers"].as_array().unwrap().iter().find(|p| p["url"] == "https://claude-gw.example.test/").expect("the typed provider");
    assert_eq!((row["provider"].as_str(), row["style"].as_str(), row["listed"].as_bool()), (Some("anthropic"), Some("claude"), Some(false)));
    for bad in [
        json!({"name":"x","url":"https://a.example.test","style":"grpc"}),
        json!({"name":"x","url":"https://a.example.test"}),
        json!({"name":"","url":"https://a.example.test","style":"openai"}),
        json!({"preset":"Not a preset"}),
    ] {
        assert_eq!(w.api.post_status("/api/providers", bad.clone()).await, 400, "{bad}");
    }
}

/// A stand-in provider: `GET /v1/models` lists two ids, one a tts model.
async fn stand_in_provider() -> String {
    use axum::{routing::get, Json, Router};
    let app = Router::new().route("/v1/models", get(|| async {
        Json(json!({"object": "list", "data": [{"id": "stand-in-chat"}, {"id": "stand-in-tts"}]}))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    url
}

/// Suggestions are the provider's own list for the key, nothing hidden; a
/// full id typed is added whether or not the list could be read.
#[tokio::test]
async fn a_key_providers_suggestions_are_its_own_list_and_a_typed_id_is_added_either_way() {
    let w = World::start(Setup::default()).await;
    let url = stand_in_provider().await;
    let made = w.api.post("/api/providers", json!({"name":"Stand-in","url":url,"style":"openai"})).await;
    let id = made["providers"].as_array().unwrap().iter().find(|p| p["url"].as_str().unwrap().starts_with(&url)).unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(w.api.put_json(&format!("/api/providers/{id}/key"), json!({"api_key":"test-only-marker"})).await.0, 200);
    let suggested = w.api.get(&format!("/api/providers/{id}/suggestions")).await;
    let ids: Vec<&str> = suggested["models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["stand-in-chat", "stand-in-tts"]);
    let body = w.api.post(&format!("/api/providers/{id}/models"), json!({"model":"stand-in-chat"})).await;
    assert!(body.to_string().contains("stand-in-chat"));
    assert!(!body.to_string().contains("test-only-marker"));
    // A provider whose list cannot be read: no suggestions, and a typed id still goes in.
    let dead = w.api.post("/api/providers", json!({"name":"Dead","url":"http://127.0.0.1:9/v1","style":"openai"})).await;
    let dead_id = dead["providers"].as_array().unwrap().iter().find(|p| p["url"] == "http://127.0.0.1:9/v1").unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(w.api.get(&format!("/api/providers/{dead_id}/suggestions")).await["models"], Value::Null);
    let body = w.api.post(&format!("/api/providers/{dead_id}/models"), json!({"model":"some-full-id"})).await;
    assert!(body.to_string().contains("some-full-id"));
}

/// A stand-in Ollama: `GET /api/tags` lists two models.
async fn stand_in_ollama() -> String {
    use axum::{routing::get, Json, Router};
    let app = Router::new().route("/api/tags", get(|| async { Json(json!({"models": [{"name": "stand-in:7b"}, {"name": "other:1b"}]})) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    url
}

/// Ollama is an ordinary row: no key, an address the person edits, models
/// added by name and suggested from Ollama's own tags; no This Mac row.
#[tokio::test]
async fn ollama_is_a_row_with_an_address_and_no_key() {
    let w = World::start(Setup::default()).await;
    let body = w.api.get("/api/providers").await;
    assert!(body["providers"].as_array().unwrap().iter().all(|p| p["label"] != "This Mac"), "no This Mac row");
    let made = w.api.post("/api/providers", json!({"preset":"Ollama"})).await;
    let row = provider(&made, "Ollama");
    assert_eq!((row["way"].as_str(), row["listed"].as_bool()), (Some("local"), Some(false)));
    assert!(row.get("key").is_none(), "no key field");
    let id = row["id"].as_str().unwrap().to_string();
    let url = stand_in_ollama().await;
    let moved = w.api.patch_status(&format!("/api/providers/{id}"), json!({"url": url})).await;
    assert_eq!(moved, 200);
    let body = w.api.get("/api/providers").await;
    let row = provider(&body, "Ollama");
    assert_eq!(row["url"].as_str().unwrap().trim_end_matches('/'), url);
    let id = row["id"].as_str().unwrap().to_string();
    let suggested = w.api.get(&format!("/api/providers/{id}/suggestions")).await;
    let ids: Vec<&str> = suggested["models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["stand-in:7b", "other:1b"]);
    let body = w.api.post(&format!("/api/providers/{id}/models"), json!({"model":"stand-in:7b"})).await;
    assert_eq!(provider(&body, "Ollama")["models"][0]["model"], "stand-in:7b");
}

/// Linggen Cloud's models are the site's last good answer, kept on disk;
/// a tick files an ordinary row, an untick of the first default needs a
/// replacement, and an id the site does not list is not ticked.
#[tokio::test]
async fn linggen_clouds_models_are_the_kept_answer_and_tick_like_rows() {
    let kept = json!({"updated":"2026-11-26","starter":"stand-in-flash","models":[
        {"id":"stand-in-flash","label":"Stand-in Flash"},{"id":"stand-in-max","label":"Stand-in Max"}]});
    let w = World::start_with_home(Setup::default(), |home| {
        let dir = home.linggen_home().join("cache");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cloud-models.json"), kept.to_string()).unwrap();
    }).await;
    let body = w.api.get("/api/providers").await;
    let cloud = provider(&body, "Linggen Cloud");
    let lines: Vec<(&str, &str)> = cloud["cloud"].as_array().unwrap().iter().map(|l| (l["model"].as_str().unwrap(), l["label"].as_str().unwrap())).collect();
    assert_eq!(lines, [("stand-in-flash", "Stand-in Flash"), ("stand-in-max", "Stand-in Max")]);
    let (status, body) = w.api.put_json("/api/providers/cloud", json!({"model":"stand-in-max","ticked":true})).await;
    assert_eq!(status, 200);
    let row = provider(&body, "Linggen Cloud")["cloud"][1]["row"].clone();
    assert_eq!(row, "stand-in-max");
    let config = w.api.get("/api/config").await;
    assert!(config["models"].as_array().unwrap().iter().any(|m| m["id"] == "stand-in-max" && m["auth_mode"] == "linggen_account"));
    assert_eq!(w.api.put_json("/api/providers/cloud", json!({"model":"not-listed","ticked":true})).await.0, 404);
    // Untick: fine when it is no default.
    let (status, body) = w.api.put_json("/api/providers/cloud", json!({"model":"stand-in-max","ticked":false})).await;
    assert_eq!(status, 200);
    assert!(provider(&body, "Linggen Cloud")["cloud"][1].get("row").is_none());
}

/// Removing a provider takes its key and every row filed under it; the
/// defaults go on without them.
#[tokio::test]
async fn removing_a_provider_takes_its_key_and_rows() {
    let w = World::start(Setup { config_append: Some(OWN_ROWS), ..Setup::default() }).await;
    assert_eq!(w.api.put_json("/api/credentials", json!({"endpoints":[{"provider":"openai","url":"https://api.deepseek.com/v1","api_key":"test-only-marker"}]})).await.0, 200);
    let body = w.api.get("/api/providers").await;
    let id = provider(&body, "DeepSeek")["id"].as_str().unwrap().to_string();
    let after = w.api.delete(&format!("/api/providers/{id}"), json!({})).await;
    assert!(!after["providers"].as_array().unwrap().iter().any(|p| p["id"] == id.as_str()));
    let config = w.api.get("/api/config").await;
    assert!(!config["models"].as_array().unwrap().iter().any(|m| m["id"] == "own-deepseek"));
    let keys = std::fs::read_to_string(w.home.linggen_home().join("credentials.json")).unwrap();
    assert!(!keys.contains("api.deepseek.com"), "{keys}");
    let dashscope = after["providers"].as_array().unwrap().iter().find(|p| p["url"].as_str().unwrap().contains("dashscope")).expect("another provider stays");
    assert_eq!(dashscope["models"][0]["id"], "own-dashscope");
}

const OWN_ROWS: &str = r#"
[[models]]
id = "own-deepseek"
provider = "openai"
url = "https://api.deepseek.com/v1"
model = "deepseek-chat"

[[models]]
id = "own-dashscope"
provider = "qwen"
url = "https://dashscope.aliyuncs.com/compatible-mode/v1"
model = "qwen-plus"
"#;

// ── the ChatGPT sign-in is a provider the person adds ──

/// Signed in (the upgrade case: no unticks stored): the row is there with
/// the three built-ins ticked, and nothing the person used disappears.
#[tokio::test]
async fn a_signed_in_chatgpt_row_has_its_three_models_ticked() {
    let w = World::start(Setup::default()).await;
    let body = w.api.get("/api/providers").await;
    let chatgpt = provider(&body, "ChatGPT 订阅");
    let lines = chatgpt["chatgpt"].as_array().unwrap();
    assert_eq!(lines.len(), 3);
    assert!(lines.iter().all(|l| l["row"].is_string()), "{lines:?}");
    let models = w.api.get("/api/models").await;
    for id in lines.iter().map(|l| l["label"].as_str().unwrap()) {
        assert!(models.as_array().unwrap().iter().any(|m| m["id"] == id), "{id} stays offered");
    }
}

/// Not signed in: no ChatGPT row at all, and its models are not offered.
#[tokio::test]
async fn a_signed_out_chatgpt_has_no_row_and_no_models() {
    let w = World::start(Setup { signed_out: true, ..Setup::default() }).await;
    let body = w.api.get("/api/providers").await;
    // (The fixture's own stand-in row sits at another address: it is its own card, with no ticks.)
    assert!(!body["providers"].as_array().unwrap().iter().any(|p| p.get("chatgpt").is_some()));
    assert!(body["presets"].as_array().unwrap().iter().any(|p| p["label"] == "ChatGPT 订阅"), "it can be added");
    let models = w.api.get("/api/models").await;
    assert!(!models.as_array().unwrap().iter().any(|m| m["provider"] == "chatgpt" && m["id"] != "fake-sol"));
}

/// Unticking a family takes its model out of the lists and survives a save;
/// ticking it back returns it; an unknown family is refused.
#[tokio::test]
async fn a_chatgpt_family_is_ticked_and_unticked() {
    let w = World::start(Setup::default()).await;
    let body = w.api.get("/api/providers").await;
    let luna = provider(&body, "ChatGPT 订阅")["chatgpt"].as_array().unwrap().iter().find(|l| l["model"] == "luna").unwrap().clone();
    let id = luna["label"].as_str().unwrap();
    let (status, body) = w.api.put_json("/api/providers/chatgpt", json!({"family":"luna","ticked":false})).await;
    assert_eq!(status, 200, "{body}");
    let line = provider(&body, "ChatGPT 订阅")["chatgpt"].as_array().unwrap().iter().find(|l| l["model"] == "luna").unwrap().clone();
    assert!(line.get("row").is_none());
    let models = w.api.get("/api/models").await;
    assert!(!models.as_array().unwrap().iter().any(|m| m["id"] == id));
    assert_eq!(w.api.get("/api/config").await["chatgpt_unticked"], json!(["luna"]));
    let (_, body) = w.api.put_json("/api/providers/chatgpt", json!({"family":"luna","ticked":true})).await;
    assert!(provider(&body, "ChatGPT 订阅")["chatgpt"].as_array().unwrap().iter().all(|l| l["row"].is_string()));
    assert_eq!(w.api.put_json("/api/providers/chatgpt", json!({"family":"nope","ticked":true})).await.0, 404);
}

/// 移除这个服务商 on the ChatGPT row signs out: the row and its models go.
#[tokio::test]
async fn removing_the_chatgpt_provider_signs_out() {
    let w = World::start(Setup::default()).await;
    let body = w.api.get("/api/providers").await;
    let card = |body: &Value| body["providers"].as_array().unwrap().iter().find(|p| p.get("chatgpt").is_some()).cloned();
    let id = card(&body).expect("the ticked card")["id"].as_str().unwrap().to_string();
    let after = w.api.delete(&format!("/api/providers/{id}"), json!({})).await;
    assert!(card(&after).is_none());
    assert!(!w.home.linggen_home().join("codex_auth.json").exists());
    let models = w.api.get("/api/models").await;
    assert!(!models.as_array().unwrap().iter().any(|m| m["provider"] == "chatgpt" && m["id"] != "fake-sol"));
}
