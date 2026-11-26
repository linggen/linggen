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
    // An added provider (添加服务商), not a listed one: the list owns its own address.
    let listed = w.api.post("/api/providers", json!({"preset":"DeepSeek"})).await;
    let listed = provider(&listed, "DeepSeek");
    assert_eq!(listed["listed"], true);
    assert_eq!(w.api.patch_status(&format!("/api/providers/{}", listed["id"].as_str().unwrap()), json!({"url":"https://example.invalid/v1"})).await, 400);
    let body = w.api.post("/api/providers", json!({"name":"deepseek","url":"https://gateway.example.test/v1"})).await;
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
async fn qwen_cloud_offers_only_what_the_list_approves_and_never_dashscope_rates() {
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
            "dashscope-cn": ["qwen-plus"]
        }});
        std::fs::write(home.linggen_home().join("test-families.json"), fixture.to_string()).unwrap();
    }).await;
    // Before a key: the one entry the list approves, not yet usable; no candidate comes from the provider's own list.
    let before = w.api.get("/api/providers").await;
    let families = provider(&before, "Qwen Cloud")["families"]["families"].as_array().unwrap().clone();
    assert_eq!(families.len(), 1, "{families:?}");
    assert_eq!((families[0]["family"].as_str(), families[0]["hold"].as_str()), (Some("qwen-plus"), Some("no_key")));
    // The endpoint already has a row, so the key is not a new connection and no starter is ticked;
    // a person's tick runs the approved id, pinned, with no price borrowed.
    let body = file_key(&w, "Qwen Cloud").await;
    let qwen = provider(&body, "Qwen Cloud");
    assert_eq!(qwen["families"]["rates_unavailable"], true);
    assert_eq!(qwen["families"]["families"][0]["ticked"], false);
    let qid = qwen["id"].as_str().unwrap().to_string();
    let (status, _) = w.api.put_text(&format!("/api/providers/{qid}/families/qwen-plus"), json!({"action":"tick"})).await;
    assert_eq!(status, 200);
    let body = w.api.get("/api/providers").await;
    let qwen = provider(&body, "Qwen Cloud");
    let families = qwen["families"]["families"].as_array().unwrap();
    assert_eq!(families.len(), 1, "{families:?}");
    assert_eq!((families[0]["ticked"].clone(), families[0]["mode"].clone()), (true.into(), "pinned".into()));
    assert!(families[0]["current"].get("price").is_none());
    assert!(qwen["models"].as_array().unwrap().iter().any(|m| m["id"] == "my-deepseek" && m["ready"] == true));
    // DashScope CN is another list address (mainland edition): a row there is priced by its own facts alone.
    let created = w.api.post("/api/providers", json!({"name":"qwen","url":"https://dashscope.aliyuncs.com/compatible-mode/v1"})).await;
    assert!(provider(&created, "通义千问 (DashScope)")["id"].is_string());
    let body = file_key(&w, "通义千问 (DashScope)").await;
    let dashscope = provider(&body, "通义千问 (DashScope)");
    let id = dashscope["id"].as_str().unwrap().to_string();
    let (status, _) = w.api.put_text(&format!("/api/providers/{id}/families/qwen-plus"), json!({"action":"tick"})).await;
    assert_eq!(status, 200);
    let body = w.api.get("/api/providers").await;
    let dashscope = provider(&body, "通义千问 (DashScope)");
    // Its own facts exist (so rates are not "unavailable"); this model's lists a rate we cannot estimate: no figure.
    assert!(dashscope["families"].get("rates_unavailable").is_none());
    assert_eq!(dashscope["families"]["families"][0]["current"]["model"], "qwen-plus");
    assert!(dashscope["families"]["families"][0]["current"].get("price").is_none());
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

// ── editions: what is offered changes; what a person has does not ──

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

fn offers(body: &Value, label: &str) -> bool {
    body["presets"].as_array().unwrap().iter().any(|p| p["label"] == label)
}

/// Everything a person has, read as the engine answers it: rows, defaults and the key file.
async fn what_a_person_has(w: &World) -> (Value, Vec<u8>) {
    let mut config = w.api.get("/api/config").await;
    config.as_object_mut().unwrap().remove("model_catalog");
    let keys = std::fs::read(w.home.linggen_home().join("credentials.json")).unwrap();
    (json!({"models": config["models"], "routing": config["routing"], "uses": config["uses"], "starts": config["family_starts"]}), keys)
}

#[tokio::test]
async fn the_edition_is_chosen_by_the_owner_and_changes_only_what_is_offered() {
    let mut w = World::start(Setup { config_append: Some(OWN_ROWS), ..Setup::default() }).await;
    let credentials = json!({"endpoints": [
        {"provider": "anthropic", "url": "https://api.anthropic.com", "api_key": "test-only-marker"},
        {"provider": "openai", "url": "https://api.deepseek.com/v1", "api_key": "test-only-marker"},
        {"provider": "qwen", "url": "https://dashscope.aliyuncs.com/compatible-mode/v1", "api_key": "test-only-marker"}
    ]});
    assert_eq!(w.api.put_json("/api/credentials", credentials).await.0, 200);
    let auto = w.api.get("/api/providers").await;
    assert_eq!(auto["edition"]["choice"], "auto");
    // International first: Anthropic is offered, DashScope's China address is kept, not offered.
    let (status, intl) = w.api.put_json("/api/providers/edition", json!({"choice": "international"})).await;
    assert_eq!((status, intl["edition"].clone()), (200, json!({"edition": "international", "choice": "international"})));
    assert!(offers(&intl, "Anthropic") && !offers(&intl, "通义千问 (DashScope)"));
    assert_eq!(provider(&intl, "通义千问 (DashScope)")["retained"], true);
    assert!(provider(&intl, "Anthropic").get("retained").is_none() && provider(&intl, "DeepSeek").get("retained").is_none());
    let had = what_a_person_has(&w).await;
    // Mainland: the offer turns over; every card, key and row stays, the other edition's marked kept.
    let (status, mainland) = w.api.put_json("/api/providers/edition", json!({"choice": "mainland"})).await;
    assert_eq!((status, mainland["edition"].clone()), (200, json!({"edition": "mainland", "choice": "mainland"})));
    assert!(offers(&mainland, "通义千问 (DashScope)") && !offers(&mainland, "Anthropic"));
    let anthropic = provider(&mainland, "Anthropic");
    assert_eq!((anthropic["retained"].clone(), anthropic["key"]["set"].clone()), (true.into(), true.into()));
    assert!(provider(&mainland, "通义千问 (DashScope)").get("retained").is_none());
    assert_eq!(provider(&mainland, "DeepSeek")["models"][0]["id"], "own-deepseek", "the row stays where it was");
    assert!(mainland["presets"].as_array().unwrap().iter().filter(|p| p["way"] == "key").all(|p| p["unverified"] == true), "mainland entries are marked");
    assert_eq!(what_a_person_has(&w).await, had, "no key, row, default or starter moved");
    // The choice survives a restart and still moves nothing.
    w.engine.restart(&w.home).await;
    let again = w.api.get("/api/providers").await;
    assert_eq!(again["edition"], json!({"edition": "mainland", "choice": "mainland"}));
    assert_eq!(what_a_person_has(&w).await, had);
    // A saved settings page keeps the choice; an unknown word is refused.
    let config = w.api.get("/api/config").await;
    assert_eq!(config["model_catalog"]["edition"], "mainland");
    assert_eq!(w.api.post_status("/api/config", config).await, 200);
    assert_eq!(w.api.get("/api/providers").await["edition"]["choice"], "mainland");
    assert_eq!(w.api.put_json("/api/providers/edition", json!({"choice": "world"})).await.0, 400);
    // Back to Auto: a choice like the others, so it stays in the file (a later
    // default may move; `config/explicit.rs`); the cards are as they were.
    let (_, auto) = w.api.put_json("/api/providers/edition", json!({"choice": "auto"})).await;
    assert_eq!(auto["edition"]["choice"], "auto");
    assert_eq!(w.api.get("/api/config").await["model_catalog"]["edition"], "auto");
    assert_eq!(what_a_person_has(&w).await, had);
}

/// The list the owner publishes next, with Anthropic moved to `new`, left as the last good copy.
fn seed_moved_list(home: &crate::support::home::Home, new: &str) {
    let shipped = concat!(env!("CARGO_MANIFEST_DIR"), "/crates/linggen-engine/src/provider/models/curated/model-catalog-v2.json");
    let mut list: Value = serde_json::from_slice(&std::fs::read(shipped).unwrap()).unwrap();
    let old = list["endpoints"]["anthropic"]["base"].clone();
    list["revision"] = json!(list["revision"].as_u64().unwrap() + 1);
    list["endpoints"]["anthropic"]["base"] = new.into();
    for record in list["facts"].as_array_mut().unwrap() {
        if record["address"] == old { record["address"] = new.into(); }
    }
    let cache = home.linggen_home().join("cache/model-catalog-v2");
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    std::fs::write(cache, format!("2026-11-24T00:00:00+00:00\n\n{list}\n")).unwrap();
}

#[tokio::test]
async fn a_list_that_moves_an_address_shows_a_new_connection_without_a_key_and_the_old_key_stays() {
    const OLD: &str = "https://api.anthropic.com";
    const NEW: &str = "https://api.anthropic.example.test/v9";
    let w = World::start_with_home(Setup::default(), |home| {
        seed_moved_list(home, NEW);
        let creds = json!({"version": 2, "endpoints": [{"provider": "anthropic", "url": OLD, "api_key": "test-only-marker"}]});
        std::fs::write(home.linggen_home().join("credentials.json"), creds.to_string()).unwrap();
    }).await;
    let key_file = || std::fs::read(w.home.linggen_home().join("credentials.json")).unwrap();
    let before = key_file();
    // The list loads off the start path: wait for it to be the one in force.
    let mut body = w.api.get("/api/providers").await;
    for _ in 0..100 {
        if body["presets"].as_array().unwrap().iter().any(|p| p["url"] == NEW) { break; }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        body = w.api.get("/api/providers").await;
    }
    assert!(body["presets"].as_array().unwrap().iter().any(|p| p["label"] == "Anthropic" && p["url"] == NEW), "the moved list is in force");
    // The page adds the list's connection: a new card with no key, beside the old one that keeps its own.
    let created = w.api.post("/api/providers", json!({"preset": "Anthropic"})).await;
    let cards: Vec<&Value> = created["providers"].as_array().unwrap().iter().filter(|p| p["provider"] == "anthropic").collect();
    // A page shows an address with its root slash; the cards are told apart by address, not spelling.
    let at = |url: &str| cards.iter().find(|p| p["url"].as_str().unwrap().trim_end_matches('/') == url).copied().unwrap_or_else(|| panic!("no card at {url}: {created}"));
    assert_eq!(at(NEW)["key"]["set"], false);
    assert_eq!(at(OLD)["key"]["set"], true, "the old key stays at the old address");
    assert_ne!(at(NEW)["id"], at(OLD)["id"]);
    assert_eq!(key_file(), before, "nothing was copied or moved");
    assert!(!created.to_string().contains("test-only-marker"));
}
