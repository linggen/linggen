//! The engine's HTTP API, as a test calls it.

use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone)]
pub struct Api {
    base: String,
    client: reqwest::Client,
}

impl Api {
    pub fn new(base: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("http client");
        Self { base, client }
    }

    pub async fn get(&self, path: &str) -> Value {
        let resp = self
            .client
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET {path}: {e}"));
        body(resp, path).await
    }

    pub async fn post(&self, path: &str, payload: Value) -> Value {
        let resp = self
            .client
            .post(format!("{}{path}", self.base))
            .json(&payload)
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST {path}: {e}"));
        body(resp, path).await
    }

    pub async fn patch(&self, path: &str, payload: Value) -> Value {
        let resp = self
            .client
            .patch(format!("{}{path}", self.base))
            .json(&payload)
            .send()
            .await
            .unwrap_or_else(|e| panic!("PATCH {path}: {e}"));
        body(resp, path).await
    }

    /// `POST /api/sessions` → the new session's id.
    pub async fn create_session(&self, payload: Value) -> String {
        let resp = self.post("/api/sessions", payload).await;
        resp["id"]
            .as_str()
            .unwrap_or_else(|| panic!("no session id: {resp}"))
            .to_string()
    }

    /// `POST /api/chat` → `{status, agent_id, …}`.
    pub async fn chat(&self, sid: &str, root: &str, message: &str) -> Value {
        self.post(
            "/api/chat",
            json!({"session_id": sid, "project_root": root, "message": message}),
        )
        .await
    }

    /// The system-prompt export of one member.
    pub async fn export(&self, sid: &str, agent: &str, root: &str) -> Value {
        let q = format!(
            "/api/chat/system-prompt?project_root={}&agent_id={agent}&session_id={sid}",
            urlencoding::encode(root)
        );
        self.get(&q).await
    }

    /// Open questions in `sid` (of `agent`, when given).
    pub async fn pending(&self, sid: &str, agent: Option<&str>) -> Vec<Value> {
        let all = self.get("/api/pending-ask-user").await;
        let all = all.as_array().cloned().unwrap_or_default();
        all.into_iter()
            .filter(|q| q["session_id"] == sid)
            .filter(|q| agent.is_none_or(|a| q["agent_id"] == a))
            .collect()
    }

    /// Answer an open question with the option whose label matches `pick`.
    pub async fn answer(&self, ask: &Value, pick: &str) {
        let options = ask["questions"][0]["options"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let label = options
            .iter()
            .filter_map(|o| o["label"].as_str())
            .find(|l| l.to_lowercase().contains(&pick.to_lowercase()))
            .unwrap_or_else(|| panic!("no '{pick}' option in {ask}"))
            .to_string();
        let payload = json!({
            "question_id": ask["question_id"],
            "answers": [{"question_index": 0, "selected": [label], "custom_text": null}],
        });
        self.post("/api/ask-user-response", payload).await;
    }

    /// The history view a chat page loads.
    pub async fn history(&self, sid: &str, root: &str) -> Value {
        let q = format!(
            "/api/workspace/state?session_id={sid}&project_root={}",
            urlencoding::encode(root)
        );
        self.get(&q).await
    }
}

/// JSON when the body is JSON; else the text as a JSON string. Non-2xx panics.
async fn body(resp: reqwest::Response, path: &str) -> Value {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        panic!("{path} → {status}: {text}");
    }
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}
