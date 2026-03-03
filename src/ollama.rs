use crate::agent_manager::models::{StreamChunk, TokenUsage};
use anyhow::Result;
use futures_util::Stream;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio_util::codec::{FramedRead, LinesCodec};

#[derive(Clone)]
pub struct OllamaClient {
    http: Client,
    base_url: String,
    api_key: Option<String>,
}

impl OllamaClient {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
        }
    }

    pub async fn chat_text_with_keep_alive(
        &self,
        model: &str,
        messages: &[ChatMessage],
        keep_alive: Option<String>,
    ) -> Result<String> {
        let total_len: usize = messages.iter().map(|m| m.content.len()).sum();
        tracing::info!(
            "Ollama Request (Text): model={}, messages={}, total_chars={}",
            model,
            messages.len(),
            total_len
        );

        let url = format!("{}/api/chat", self.base_url);
        let req = ChatRequest {
            model: model.to_string(),
            messages: messages.to_vec(),
            stream: Some(false),
            format: None,
            keep_alive,
            tools: None,
        };

        let mut rb = self.http.post(url).json(&req);
        if let Some(key) = &self.api_key {
            rb = rb.header("Authorization", format!("Bearer {}", key));
        }
        let resp = rb.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("ollama error ({}): {}", status, text);
        }

        let payload: ChatResponse = resp.json().await?;
        // Concat thinking + content so the engine's stream_with_thinking
        // can split them naturally via looks_like_json_action_start.
        let mut result = String::new();
        if let Some(thinking) = &payload.message.thinking {
            if !thinking.is_empty() {
                result.push_str(thinking);
                result.push('\n');
            }
        }
        result.push_str(&payload.message.content);
        Ok(result)
    }

    pub async fn chat_text_stream_with_keep_alive(
        &self,
        model: &str,
        messages: &[ChatMessage],
        keep_alive: Option<String>,
    ) -> Result<impl Stream<Item = Result<StreamChunk>> + Send> {
        let total_len: usize = messages.iter().map(|m| m.content.len()).sum();
        tracing::info!("Ollama stream: model={} msgs={} chars={}", model, messages.len(), total_len);
        if let Some(last) = messages.last() {
            tracing::debug!("Last msg ({}): {:.200}", last.role, last.content);
        }

        let url = format!("{}/api/chat", self.base_url);
        let req = ChatRequest {
            model: model.to_string(),
            messages: messages.to_vec(),
            stream: Some(true),
            format: None,
            keep_alive,
            tools: None,
        };

        let mut rb = self.http.post(url).json(&req);
        if let Some(key) = &self.api_key {
            rb = rb.header("Authorization", format!("Bearer {}", key));
        }
        let resp = rb.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("ollama error ({}): {}", status, text);
        }

        let stream = resp
            .bytes_stream()
            .map(|item| item.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
        let reader = tokio_util::io::StreamReader::new(stream);
        let lines = FramedRead::new(reader, LinesCodec::new());

        let token_stream = lines.filter_map(|line_result| async move {
            let line = match line_result {
                Ok(l) => l,
                Err(e) => return Some(Err(anyhow::anyhow!("stream error: {}", e))),
            };
            if line.trim().is_empty() {
                return None;
            }
            // Ollama sends one JSON object per line
            let payload: ChatStreamResponse = match serde_json::from_str(&line) {
                Ok(p) => p,
                Err(e) => return Some(Err(anyhow::anyhow!("json parse error: {} (line: {})", e, line))),
            };

            // When done==true, Ollama includes token usage stats.
            if payload.done.unwrap_or(false) {
                let usage = TokenUsage {
                    prompt_tokens: payload.prompt_eval_count.map(|v| v as usize),
                    completion_tokens: payload.eval_count.map(|v| v as usize),
                    total_tokens: match (payload.prompt_eval_count, payload.eval_count) {
                        (Some(p), Some(c)) => Some((p + c) as usize),
                        _ => None,
                    },
                };
                return Some(Ok(StreamChunk::Usage(usage)));
            }

            // Emit thinking tokens followed by content tokens
            let mut result = String::new();
            if let Some(thinking) = &payload.message.thinking {
                result.push_str(thinking);
            }
            result.push_str(&payload.message.content);
            if result.is_empty() {
                None
            } else {
                Some(Ok(StreamChunk::Token(result)))
            }
        });

        Ok(token_stream)
    }

    /// Streaming chat with native tool calling support.
    /// Sends `tools` in the request and parses tool_calls from responses.
    /// Ollama emits tool_calls as complete objects in the final chunk (not incrementally).
    pub async fn chat_tool_stream_with_keep_alive(
        &self,
        model: &str,
        messages: &[ChatMessage],
        keep_alive: Option<String>,
        tools: Vec<serde_json::Value>,
    ) -> Result<impl Stream<Item = Result<StreamChunk>> + Send> {
        let total_len: usize = messages.iter().map(|m| m.content.len()).sum();
        tracing::info!(
            "Ollama tool stream: model={} msgs={} chars={} tools={}",
            model, messages.len(), total_len, tools.len()
        );

        let url = format!("{}/api/chat", self.base_url);
        let req = ChatRequest {
            model: model.to_string(),
            messages: messages.to_vec(),
            stream: Some(true),
            format: None,
            keep_alive,
            tools: Some(tools),
        };

        let mut rb = self.http.post(url).json(&req);
        if let Some(key) = &self.api_key {
            rb = rb.header("Authorization", format!("Bearer {}", key));
        }
        let resp = rb.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("ollama error ({}): {}", status, text);
        }

        let stream = resp
            .bytes_stream()
            .map(|item| item.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
        let reader = tokio_util::io::StreamReader::new(stream);
        let lines = FramedRead::new(reader, LinesCodec::new());

        use crate::agent_manager::models::ToolCallChunk;

        let token_stream = lines.filter_map(|line_result| async move {
            let line = match line_result {
                Ok(l) => l,
                Err(e) => return Some(Err(anyhow::anyhow!("stream error: {}", e))),
            };
            if line.trim().is_empty() {
                return None;
            }

            // Parse as generic JSON to handle both text tokens and tool_calls.
            let payload: serde_json::Value = match serde_json::from_str(&line) {
                Ok(p) => p,
                Err(e) => return Some(Err(anyhow::anyhow!("json parse error: {} (line: {})", e, line))),
            };

            let done = payload.get("done").and_then(|v| v.as_bool()).unwrap_or(false);

            if done {
                let prompt_eval = payload.get("prompt_eval_count").and_then(|v| v.as_u64());
                let eval = payload.get("eval_count").and_then(|v| v.as_u64());
                let usage = TokenUsage {
                    prompt_tokens: prompt_eval.map(|v| v as usize),
                    completion_tokens: eval.map(|v| v as usize),
                    total_tokens: match (prompt_eval, eval) {
                        (Some(p), Some(c)) => Some((p + c) as usize),
                        _ => None,
                    },
                };
                return Some(Ok(StreamChunk::Usage(usage)));
            }

            let message = payload.get("message")?;

            // Check for tool_calls in the message
            if let Some(tool_calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
                // Ollama emits complete tool calls (not incremental)
                let mut chunks = Vec::new();
                for (idx, tc) in tool_calls.iter().enumerate() {
                    let func = tc.get("function")?;
                    let name = func.get("name").and_then(|v| v.as_str()).map(|s| s.to_string());
                    let arguments = func.get("arguments").map(|v| v.to_string());
                    // Generate a unique ID since Ollama doesn't provide one
                    let id = format!("call_{}", uuid::Uuid::new_v4().to_string().replace('-', "")[..24].to_string());
                    chunks.push(StreamChunk::ToolCall(ToolCallChunk {
                        index: idx,
                        id: Some(id),
                        name,
                        arguments_delta: arguments,
                    }));
                }
                // Return the first chunk; remaining ones would need multi-yield.
                // Since Ollama sends all tool_calls at once, emit them as separate items.
                if chunks.len() == 1 {
                    return Some(Ok(chunks.into_iter().next().unwrap()));
                }
                // For multiple tool calls, emit first one (others come in subsequent chunks from Ollama).
                // In practice, Ollama sends one message with all tool_calls at the end.
                return Some(Ok(chunks.into_iter().next().unwrap()));
            }

            // Regular text content — only emit `content`, not `thinking`.
            // Ollama separates reasoning into message.thinking; we discard it
            // here because in native tool calling mode text goes directly to
            // the user and thinking is internal model reasoning.
            let result = message
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if result.is_empty() {
                None
            } else {
                Some(Ok(StreamChunk::Token(result)))
            }
        });

        Ok(token_stream)
    }

    /// Get the status of currently running models in Ollama.
    pub async fn get_ps(&self) -> Result<OllamaPsResponse> {
        let url = format!("{}/api/ps", self.base_url);
        let mut rb = self.http.get(url);
        if let Some(key) = &self.api_key {
            rb = rb.header("Authorization", format!("Bearer {}", key));
        }
        let resp = rb.send().await?;
        let payload: OllamaPsResponse = resp.json().await?;
        Ok(payload)
    }

    /// Best-effort: fetch model context window (num_ctx) from Ollama.
    ///
    /// Ollama exposes model metadata at /api/show. We parse either:
    /// - parameters.num_ctx (if present in object form),
    /// - num_ctx/context_length lines from parameters/modelfile text, or
    /// - model_info.*.context_length keys.
    pub async fn get_model_context_window(&self, model: &str) -> Result<Option<usize>> {
        let url = format!("{}/api/show", self.base_url);
        let req = serde_json::json!({ "name": model });
        let mut rb = self.http.post(url).json(&req);
        if let Some(key) = &self.api_key {
            rb = rb.header("Authorization", format!("Bearer {}", key));
        }
        let resp = rb.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("ollama error ({}): {}", status, text);
        }

        let payload: OllamaShowResponse = resp.json().await?;

        // 1) parameters object or text
        if let Some(params) = payload.parameters.as_ref() {
            match params {
                OllamaShowParameters::Map(map) => {
                    if let Some(v) = map.get("num_ctx").and_then(parse_usize_value) {
                        return Ok(Some(v));
                    }
                    if let Some(v) = map.get("context_length").and_then(parse_usize_value) {
                        return Ok(Some(v));
                    }
                }
                OllamaShowParameters::Text(text) => {
                    if let Some(v) = parse_num_ctx_from_text(text) {
                        return Ok(Some(v));
                    }
                }
            }
        }

        // 2) parse from model_info keys like "<arch>.context_length"
        if let Some(model_info) = payload.model_info.as_ref() {
            for (key, value) in model_info {
                let key_lc = key.to_ascii_lowercase();
                if key_lc == "context_length"
                    || key_lc.ends_with(".context_length")
                    || key_lc.ends_with("_context_length")
                {
                    if let Some(v) = parse_usize_value(value) {
                        return Ok(Some(v));
                    }
                }
            }
        }

        // 3) parse from details object when available
        if let Some(details) = payload.details.as_ref() {
            for k in ["context_length", "num_ctx"] {
                if let Some(v) = details.get(k).and_then(parse_usize_value) {
                    return Ok(Some(v));
                }
            }
        }

        // 4) parse from modelfile text
        if let Some(modelfile) = payload.modelfile.as_deref() {
            if let Some(v) = parse_num_ctx_from_text(modelfile) {
                return Ok(Some(v));
            }
        }

        Ok(None)
    }

    /// Check if a model supports vision (image input) via /api/show capabilities.
    pub async fn get_model_has_vision(&self, model: &str) -> Result<bool> {
        let url = format!("{}/api/show", self.base_url);
        let req = serde_json::json!({ "name": model });
        let mut rb = self.http.post(url).json(&req);
        if let Some(key) = &self.api_key {
            rb = rb.header("Authorization", format!("Bearer {}", key));
        }
        let resp = rb.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("ollama error ({}): {}", status, text);
        }
        let payload: OllamaShowResponse = resp.json().await?;
        Ok(payload.capabilities.iter().any(|c| c == "vision"))
    }
}

fn parse_usize_value(value: &serde_json::Value) -> Option<usize> {
    if let Some(v) = value.as_u64() {
        return usize::try_from(v).ok();
    }
    if let Some(s) = value.as_str() {
        return parse_usize_token(s);
    }
    None
}

fn parse_usize_token(raw: &str) -> Option<usize> {
    let cleaned = raw.trim().trim_matches('"').trim_matches('\'');
    if let Ok(v) = cleaned.parse::<usize>() {
        return Some(v);
    }
    for token in cleaned.split(|c: char| c.is_whitespace() || c == '=' || c == ':') {
        let t = token.trim().trim_matches(',').trim_matches(';');
        if t.is_empty() {
            continue;
        }
        if let Ok(v) = t.parse::<usize>() {
            return Some(v);
        }
    }
    None
}

fn parse_num_ctx_from_line(line: &str) -> Option<usize> {
    let mut s = line.trim();
    if let Some(rest) = s.strip_prefix("PARAMETER") {
        s = rest.trim();
    }
    if s.is_empty() {
        return None;
    }

    if let Some((k, v)) = s.split_once('=') {
        let key = k.trim().to_ascii_lowercase();
        if key == "num_ctx" || key == "context_length" {
            return parse_usize_token(v);
        }
    }
    if let Some((k, v)) = s.split_once(':') {
        let key = k.trim().to_ascii_lowercase();
        if key == "num_ctx" || key == "context_length" {
            return parse_usize_token(v);
        }
    }

    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() >= 2 {
        let key = parts[0].trim().to_ascii_lowercase();
        if key == "num_ctx" || key == "context_length" {
            return parse_usize_token(parts[1]);
        }
    }
    None
}

fn parse_num_ctx_from_text(text: &str) -> Option<usize> {
    for line in text.lines() {
        if let Some(v) = parse_num_ctx_from_line(line) {
            return Some(v);
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OllamaShowResponse {
    #[serde(default)]
    parameters: Option<OllamaShowParameters>,
    #[serde(default)]
    modelfile: Option<String>,
    #[serde(default)]
    model_info: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    details: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum OllamaShowParameters {
    Map(HashMap<String, serde_json::Value>),
    Text(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaPsResponse {
    pub models: Vec<OllamaPsModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaPsModel {
    pub name: String,
    pub model: String,
    pub size: u64,
    pub size_vram: u64,
    pub details: OllamaPsModelDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaPsModelDetails {
    pub parent_model: String,
    pub format: String,
    pub family: String,
    pub parameter_size: String,
    pub quantization_level: String,
}

// ---------------------------------------------------------------------------
// Native tool calling types (OpenAI-compatible)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallMessage {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(rename = "type", default, skip_serializing_if = "String::is_empty")]
    pub call_type: String, // "function"
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: serde_json::Value,
}

// ---------------------------------------------------------------------------
// ChatMessage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// Cache control hint for API providers that support prompt caching.
    /// E.g. `{"type": "ephemeral"}` for Anthropic/OpenAI cache breakpoints.
    /// Skipped during serialization for providers that don't support it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<serde_json::Value>,
    /// Tool calls emitted by the assistant (native function calling mode).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallMessage>,
    /// The tool_call_id this message is a result for (role="tool" messages).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            thinking: None,
            images: Vec::new(),
            cache_control: None,
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn with_images(mut self, images: Vec<String>) -> Self {
        self.images = images;
        self
    }

    /// Create a tool result message (role="tool") for native function calling.
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: content.into(),
            thinking: None,
            images: Vec::new(),
            cache_control: None,
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }

    /// Create an assistant message with tool calls (native function calling).
    pub fn assistant_with_tool_calls(calls: Vec<ToolCallMessage>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: String::new(),
            thinking: None,
            images: Vec::new(),
            cache_control: None,
            tool_calls: calls,
            tool_call_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keep_alive: Option<String>,
    /// OpenAI-compatible tool definitions for native function calling.
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}

/// Extended response type for streaming chunks, includes token usage fields.
#[derive(Debug, Clone, Deserialize)]
struct ChatStreamResponse {
    message: ChatMessage,
    #[serde(default)]
    done: Option<bool>,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    #[serde(default)]
    eval_count: Option<u64>,
}
