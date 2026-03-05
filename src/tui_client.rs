use crate::server::UiSseMessage;
use anyhow::Result;
use reqwest::Client;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tracing::warn;

/// Percent-encode a query parameter value.
fn encode_param(s: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                write!(out, "%{:02X}", b).unwrap();
            }
        }
    }
    out
}

pub struct TuiClient {
    base_url: String,
    client: Client,
}

impl TuiClient {
    pub fn new(port: u16) -> Self {
        Self {
            base_url: format!("http://127.0.0.1:{}", port),
            client: Client::new(),
        }
    }

    pub async fn health_check(&self) -> bool {
        self.client
            .get(format!("{}/api/health", self.base_url))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Send a chat message. Returns the `session_id` from the server response
    /// (useful when the server auto-creates a new session).
    pub async fn send_chat(
        &self,
        project_root: &str,
        agent_id: &str,
        message: &str,
        session_id: Option<&str>,
        images: Option<Vec<String>>,
    ) -> Result<Option<String>> {
        let mut body = json!({
            "project_root": project_root,
            "agent_id": agent_id,
            "message": message,
        });
        if let Some(sid) = session_id {
            body["session_id"] = json!(sid);
        }
        if let Some(imgs) = images {
            if !imgs.is_empty() {
                body["images"] = json!(imgs);
            }
        }
        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Chat request failed ({}): {}", status, text);
        }
        let resp_body: serde_json::Value = resp.json().await.unwrap_or_default();
        Ok(resp_body
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(String::from))
    }

    pub async fn approve_plan(
        &self,
        project_root: &str,
        agent_id: &str,
        session_id: Option<&str>,
        clear_context: bool,
    ) -> Result<()> {
        let mut body = json!({
            "project_root": project_root,
            "agent_id": agent_id,
            "clear_context": clear_context,
        });
        if let Some(sid) = session_id {
            body["session_id"] = json!(sid);
        }
        let resp = self
            .client
            .post(format!("{}/api/plan/approve", self.base_url))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Plan approve failed ({}): {}", status, text);
        }
        Ok(())
    }

    pub async fn reject_plan(
        &self,
        project_root: &str,
        agent_id: &str,
        session_id: Option<&str>,
    ) -> Result<()> {
        let mut body = json!({
            "project_root": project_root,
            "agent_id": agent_id,
        });
        if let Some(sid) = session_id {
            body["session_id"] = json!(sid);
        }
        let resp = self
            .client
            .post(format!("{}/api/plan/reject", self.base_url))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Plan reject failed ({}): {}", status, text);
        }
        Ok(())
    }

    /// Respond to an AskUser prompt (used for both generic AskUser and permission prompts).
    pub async fn respond_ask_user(&self, question_id: &str, selected: &str) -> Result<()> {
        let body = json!({
            "question_id": question_id,
            "answers": [{ "question_index": 0, "selected": [selected], "custom_text": null }]
        });
        let resp = self
            .client
            .post(format!("{}/api/ask-user-response", self.base_url))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("AskUser response failed ({}): {}", status, text);
        }
        Ok(())
    }

    /// Respond to an AskUser prompt with custom free-text ("Other" option).
    pub async fn respond_ask_user_custom(&self, question_id: &str, custom_text: &str) -> Result<()> {
        let body = json!({
            "question_id": question_id,
            "answers": [{ "question_index": 0, "selected": [], "custom_text": custom_text }]
        });
        let resp = self
            .client
            .post(format!("{}/api/ask-user-response", self.base_url))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("AskUser custom response failed ({}): {}", status, text);
        }
        Ok(())
    }

    /// Cancel an agent run by run_id.
    pub async fn cancel_run(&self, run_id: &str) -> Result<()> {
        let body = json!({ "run_id": run_id });
        let resp = self
            .client
            .post(format!("{}/api/agent-cancel", self.base_url))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Cancel run failed ({}): {}", status, text);
        }
        Ok(())
    }

    /// Fetch workspace state from the REST API (used for resync after reconnection or lag).
    pub async fn fetch_workspace_state(
        &self,
        project_root: &str,
        session_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut url = format!(
            "{}/api/workspace/state?project_root={}",
            self.base_url,
            encode_param(project_root)
        );
        if let Some(sid) = session_id {
            url.push_str(&format!("&session_id={}", encode_param(sid)));
        }
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("workspace state fetch failed: {}", resp.status());
        }
        Ok(resp.json().await?)
    }

    /// Fetch agent runs from the REST API (used for resync after reconnection or lag).
    pub async fn fetch_agent_runs(
        &self,
        project_root: &str,
        session_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut url = format!(
            "{}/api/agent-runs?project_root={}",
            self.base_url,
            encode_param(project_root)
        );
        if let Some(sid) = session_id {
            url.push_str(&format!("&session_id={}", encode_param(sid)));
        }
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("agent runs fetch failed: {}", resp.status());
        }
        Ok(resp.json().await?)
    }

    /// Fetch available skills from the server.
    pub async fn fetch_skills(&self) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .client
            .get(format!("{}/api/skills", self.base_url))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("skills fetch failed: {}", resp.status());
        }
        Ok(resp.json().await?)
    }

    /// Fetch available agents for a project.
    pub async fn fetch_agents(&self, project_root: &str) -> Result<Vec<serde_json::Value>> {
        let url = format!(
            "{}/api/agents?project_root={}",
            self.base_url,
            encode_param(project_root)
        );
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("agents fetch failed: {}", resp.status());
        }
        Ok(resp.json().await?)
    }

    /// Subscribe to SSE events. Returns an unbounded receiver that yields parsed UiSseMessage.
    /// The SSE connection runs in a background task with automatic reconnection
    /// using exponential backoff (1s → 2s → 4s → ... capped at 30s).
    pub fn subscribe_sse(&self) -> (mpsc::UnboundedReceiver<UiSseMessage>, AbortHandle) {
        let (tx, rx) = mpsc::unbounded_channel();
        let url = format!("{}/api/events", self.base_url);
        let client = self.client.clone();

        let handle = tokio::spawn(async move {
            let mut backoff_secs: u64 = 1;
            const MAX_BACKOFF: u64 = 30;

            loop {
                let resp = match client.get(&url).send().await {
                    Ok(r) => {
                        // Connected — reset backoff and notify
                        backoff_secs = 1;
                        let connected_msg = UiSseMessage {
                            id: String::new(),
                            seq: 0,
                            rev: 0,
                            ts_ms: 0,
                            kind: "connection".to_string(),
                            phase: Some("connected".to_string()),
                            text: None,
                            agent_id: None,
                            session_id: None,
                            project_root: None,
                            data: None,
                        };
                        if tx.send(connected_msg).is_err() {
                            return; // receiver dropped
                        }
                        r
                    }
                    Err(e) => {
                        warn!("SSE connect failed: {}", e);
                        Self::send_disconnected(&tx, &format!("Connect failed: {e}"));
                        tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                        backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF);
                        continue;
                    }
                };

                let mut stream = resp.bytes_stream();
                let mut buf = String::new();

                use futures_util::StreamExt;
                while let Some(chunk) = stream.next().await {
                    let chunk = match chunk {
                        Ok(c) => c,
                        Err(e) => {
                            warn!("SSE stream error: {}", e);
                            break;
                        }
                    };
                    buf.push_str(&String::from_utf8_lossy(&chunk));

                    // Parse SSE frames: lines starting with "data:" separated by blank lines.
                    while let Some(pos) = buf.find("\n\n") {
                        let frame = buf[..pos].to_string();
                        buf = buf[pos + 2..].to_string();

                        for line in frame.lines() {
                            let data = if let Some(d) = line.strip_prefix("data:") {
                                d.trim()
                            } else {
                                continue;
                            };
                            if data.is_empty() {
                                continue;
                            }
                            match serde_json::from_str::<UiSseMessage>(data) {
                                Ok(msg) => {
                                    if tx.send(msg).is_err() {
                                        return; // receiver dropped
                                    }
                                }
                                Err(_) => {
                                    // Skip malformed frames silently
                                }
                            }
                        }
                    }
                }

                // Stream ended — notify disconnected and retry
                Self::send_disconnected(&tx, "Stream ended");
                tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF);
            }
        });

        (rx, handle.abort_handle())
    }

    /// Fetch available models from the server.
    pub async fn fetch_models(&self) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .client
            .get(format!("{}/api/models", self.base_url))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("models fetch failed: {}", resp.status());
        }
        Ok(resp.json().await?)
    }

    /// Set the default model by reordering `routing.default_models` to put the
    /// chosen model first, then POST the updated config.
    pub async fn set_default_model(&self, model_id: &str) -> Result<()> {
        // GET current config
        let resp = self
            .client
            .get(format!("{}/api/config", self.base_url))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("config fetch failed: {}", resp.status());
        }
        let mut config: serde_json::Value = resp.json().await?;

        // Reorder routing.default_models: chosen model first, keep others as fallbacks
        let routing = config
            .as_object_mut()
            .and_then(|o| o.get_mut("routing"))
            .and_then(|r| r.as_object_mut());
        if let Some(routing) = routing {
            let existing: Vec<String> = routing
                .get("default_models")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let mut new_list = vec![model_id.to_string()];
            for m in &existing {
                if m != model_id {
                    new_list.push(m.clone());
                }
            }
            routing.insert(
                "default_models".to_string(),
                json!(new_list),
            );
        }

        // POST updated config
        let resp = self
            .client
            .post(format!("{}/api/config", self.base_url))
            .json(&config)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("config update failed ({}): {}", status, text);
        }
        Ok(())
    }

    fn send_disconnected(tx: &mpsc::UnboundedSender<UiSseMessage>, reason: &str) {
        let msg = UiSseMessage {
            id: String::new(),
            seq: 0,
            rev: 0,
            ts_ms: 0,
            kind: "connection".to_string(),
            phase: Some("disconnected".to_string()),
            text: Some(reason.to_string()),
            agent_id: None,
            session_id: None,
            project_root: None,
            data: None,
        };
        let _ = tx.send(msg);
    }
}
