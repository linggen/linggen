//! Provider settings only project the configured groups and stored key presence.

use crate::support::{Setup, World};
use serde_json::{json, Value};

pub(super) fn provider<'a>(body: &'a Value, label: &str) -> &'a Value {
    body["providers"].as_array().unwrap().iter().find(|row| row["label"] == label).unwrap()
}

#[tokio::test]
async fn providers_group_by_sign_in_name_and_address_without_returning_keys() {
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
    assert_eq!(w.api.get("/api/providers").await, body);
}
