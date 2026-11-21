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

    /// POST that may fail: the status code alone.
    pub async fn post_status(&self, path: &str, payload: Value) -> u16 {
        self.client
            .post(format!("{}{path}", self.base))
            .json(&payload)
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST {path}: {e}"))
            .status()
            .as_u16()
    }

    /// POST that may fail: the status and the JSON body (`null` if none).
    pub async fn post_with_status(&self, path: &str, payload: Value) -> (u16, Value) {
        let resp = self
            .client
            .post(format!("{}{path}", self.base))
            .json(&payload)
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST {path}: {e}"));
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// POST raw bytes (a clip, say): the status and the JSON body (`null` if none).
    pub async fn post_bytes(&self, path: &str, content_type: &str, bytes: Vec<u8>) -> (u16, Value) {
        let resp = self
            .client
            .post(format!("{}{path}", self.base))
            .header("content-type", content_type)
            .body(bytes)
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST {path}: {e}"));
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// Any method on `path`: the status and the JSON body (`null` if none).
    pub async fn call(&self, method: &str, path: &str) -> (u16, Value) {
        let method = reqwest::Method::from_bytes(method.as_bytes()).expect("method");
        let resp = self
            .client
            .request(method, format!("{}{path}", self.base))
            .send()
            .await
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// A PUT of `payload`: the status and the JSON body (`null` if none).
    pub async fn put_json(&self, path: &str, payload: Value) -> (u16, Value) {
        let resp = self
            .client
            .put(format!("{}{path}", self.base))
            .json(&payload)
            .send()
            .await
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
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

    /// `DELETE`, answering only the status (a refusal is not a panic).
    pub async fn delete_status(&self, path: &str, payload: Value) -> u16 {
        let resp = self
            .client
            .delete(format!("{}{path}", self.base))
            .json(&payload)
            .send()
            .await
            .unwrap_or_else(|e| panic!("DELETE {path}: {e}"));
        resp.status().as_u16()
    }

    pub async fn delete(&self, path: &str, payload: Value) -> Value {
        let resp = self
            .client
            .delete(format!("{}{path}", self.base))
            .json(&payload)
            .send()
            .await
            .unwrap_or_else(|e| panic!("DELETE {path}: {e}"));
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
        self.answer_as(ask, pick, None).await
    }

    /// [`Self::answer`], as the person `user` (`None`: the builder).
    pub async fn answer_as(&self, ask: &Value, pick: &str, user: Option<&str>) {
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
        let mut payload = json!({
            "question_id": ask["question_id"],
            "answers": [{"question_index": 0, "selected": [label], "custom_text": null}],
        });
        if let Some(user) = user {
            payload["user_id"] = json!(user);
        }
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
