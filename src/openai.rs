use crate::agent_manager::models::{StreamChunk, TokenUsage};
use anyhow::Result;
use futures_util::Stream;
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Fix malformed JSON from some providers (e.g. Gemini sends `"function":,` instead of `"function":null,`).
fn sanitize_json(data: &str) -> std::borrow::Cow<'_, str> {
    // Match `":,` or `":}` patterns (value missing after colon)
    if data.contains(":,") || data.contains(":}") {
        let mut out = data.replace(":,", ":null,");
        out = out.replace(":}", ":null}");
        std::borrow::Cow::Owned(out)
    } else {
        std::borrow::Cow::Borrowed(data)
    }
}

#[derive(Clone)]
pub struct OpenAiClient {
    http: Client,
    base_url: String,
    api_key: Option<String>,
}

impl OpenAiClient {
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

    /// Streaming text chat completion (SSE format).
    pub async fn chat_text_stream(
        &self,
        model: &str,
        messages: &[crate::ollama::ChatMessage],
    ) -> Result<impl Stream<Item = Result<StreamChunk>> + Send> {
        let total_len: usize = messages.iter().map(|m| m.content.len()).sum();
        tracing::info!("OpenAI stream: model={} msgs={} chars={}", model, messages.len(), total_len);
        if let Some(last) = messages.last() {
            tracing::debug!("Last msg ({}): {:.200}", last.role, last.content);
        }

        let url = format!("{}/chat/completions", self.base_url);
        let oai_messages: Vec<OaiMessage> = messages.iter().map(OaiMessage::from_chat).collect();
        let req = OaiRequest {
            model: model.to_string(),
            messages: oai_messages,
            stream: true,
            response_format: None,
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
            let truncated = if text.len() > 500 {
                format!("{}… ({} chars)", &text[..500], text.len())
            } else {
                text
            };
            anyhow::bail!("openai error ({}): {}", status, truncated);
        }

        // OpenAI streams SSE: "data: {...}\n\n" lines, terminated by "data: [DONE]"
        let byte_stream = resp
            .bytes_stream()
            .map(|item| item.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
        let reader = tokio_util::io::StreamReader::new(byte_stream);
        let lines = tokio_util::codec::FramedRead::new(reader, tokio_util::codec::LinesCodec::new());

        use futures_util::StreamExt;
        let token_stream = lines.filter_map(|line_result| async move {
            let line = match line_result {
                Ok(l) => l,
                Err(e) => return Some(Err(anyhow::anyhow!("stream error: {}", e))),
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            let data = match trimmed.strip_prefix("data: ") {
                Some(d) => d.trim(),
                None => return None,
            };
            if data == "[DONE]" {
                return None;
            }
            let sanitized = sanitize_json(data);
            let chunk: OaiStreamChunk = match serde_json::from_str(&sanitized) {
                Ok(c) => c,
                Err(e) => {
                    let truncated = if data.len() > 300 {
                        format!("{}… ({} chars)", &data[..300], data.len())
                    } else {
                        data.to_string()
                    };
                    return Some(Err(anyhow::anyhow!(
                        "openai json parse error: {} (data: {})",
                        e,
                        truncated
                    )));
                }
            };

            // Check for usage data (some providers include it in the final chunk).
            if let Some(usage) = chunk.usage {
                return Some(Ok(StreamChunk::Usage(TokenUsage {
                    prompt_tokens: usage.prompt_tokens.map(|v| v as usize),
                    completion_tokens: usage.completion_tokens.map(|v| v as usize),
                    total_tokens: usage.total_tokens.map(|v| v as usize),
                })));
            }

            let content = chunk
                .choices
                .into_iter()
                .next()
                .and_then(|c| c.delta.content)
                .unwrap_or_default();
            if content.is_empty() {
                None
            } else {
                Some(Ok(StreamChunk::Token(content)))
            }
        });

        Ok(token_stream)
    }
    /// Streaming chat with native tool calling support (SSE format).
    /// Sends tool definitions and parses incremental tool_call deltas.
    pub async fn chat_tool_stream(
        &self,
        model: &str,
        messages: &[crate::ollama::ChatMessage],
        tools: Vec<serde_json::Value>,
    ) -> Result<impl Stream<Item = Result<StreamChunk>> + Send> {
        let total_len: usize = messages.iter().map(|m| m.content.len()).sum();
        tracing::info!(
            "OpenAI tool stream: model={} msgs={} chars={} tools={}",
            model, messages.len(), total_len, tools.len()
        );

        let url = format!("{}/chat/completions", self.base_url);
        let oai_messages: Vec<OaiMessageWithTools> = messages.iter().map(OaiMessageWithTools::from_chat).collect();

        // Build request with tools
        let req = serde_json::json!({
            "model": model,
            "messages": oai_messages,
            "stream": true,
            "tools": tools,
        });

        let mut rb = self.http.post(url).json(&req);
        if let Some(key) = &self.api_key {
            rb = rb.header("Authorization", format!("Bearer {}", key));
        }
        let resp = rb.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let truncated = if text.len() > 500 {
                format!("{}… ({} chars)", &text[..500], text.len())
            } else {
                text
            };
            anyhow::bail!("openai error ({}): {}", status, truncated);
        }

        let byte_stream = resp
            .bytes_stream()
            .map(|item| item.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
        let reader = tokio_util::io::StreamReader::new(byte_stream);
        let lines = tokio_util::codec::FramedRead::new(reader, tokio_util::codec::LinesCodec::new());

        use crate::agent_manager::models::ToolCallChunk;
        use futures_util::StreamExt;
        let token_stream = lines.filter_map(|line_result| async move {
            let line = match line_result {
                Ok(l) => l,
                Err(e) => return Some(Err(anyhow::anyhow!("stream error: {}", e))),
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            let data = match trimmed.strip_prefix("data: ") {
                Some(d) => d.trim(),
                None => return None,
            };
            if data == "[DONE]" {
                return None;
            }
            let sanitized = sanitize_json(data);
            let chunk: OaiStreamChunk = match serde_json::from_str(&sanitized) {
                Ok(c) => c,
                Err(e) => {
                    return Some(Err(anyhow::anyhow!(
                        "openai json parse error: {} (data: {})",
                        e, data
                    )));
                }
            };

            // Check for usage data
            if let Some(usage) = chunk.usage {
                return Some(Ok(StreamChunk::Usage(TokenUsage {
                    prompt_tokens: usage.prompt_tokens.map(|v| v as usize),
                    completion_tokens: usage.completion_tokens.map(|v| v as usize),
                    total_tokens: usage.total_tokens.map(|v| v as usize),
                })));
            }

            let choice = chunk.choices.into_iter().next()?;

            // Check for tool_call deltas
            if let Some(tool_calls) = choice.delta.tool_calls {
                // Emit ToolCall chunks for each delta
                for tc in tool_calls {
                    let name = tc.function.as_ref().and_then(|f| f.name.clone());
                    let args_delta = tc.function.as_ref().and_then(|f| f.arguments.clone());
                    return Some(Ok(StreamChunk::ToolCall(ToolCallChunk {
                        index: tc.index,
                        id: tc.id,
                        name,
                        arguments_delta: args_delta,
                    })));
                }
                return None;
            }

            // Regular content
            let content = choice.delta.content.unwrap_or_default();
            if content.is_empty() {
                None
            } else {
                Some(Ok(StreamChunk::Token(content)))
            }
        });

        Ok(token_stream)
    }
}

// --- Wire types ---

#[derive(Debug, Serialize)]
struct OaiMessage {
    role: String,
    content: OaiContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum OaiContent {
    Text(String),
    Parts(Vec<OaiContentPart>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum OaiContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: OaiImageUrl },
}

#[derive(Debug, Serialize)]
struct OaiImageUrl {
    url: String,
}

/// An OAI message with optional tool_calls (for assistant messages in native mode).
#[derive(Debug, Serialize)]
struct OaiMessageWithTools {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<OaiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    /// Function name for role="tool" messages (required by Gemini's OpenAI-compatible API).
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

impl OaiMessage {
    fn from_chat(msg: &crate::ollama::ChatMessage) -> Self {
        let content = if msg.images.is_empty() {
            OaiContent::Text(msg.content.clone())
        } else {
            let mut parts = vec![OaiContentPart::Text {
                text: msg.content.clone(),
            }];
            for img in &msg.images {
                parts.push(OaiContentPart::ImageUrl {
                    image_url: OaiImageUrl {
                        url: format!("data:image/png;base64,{}", img),
                    },
                });
            }
            OaiContent::Parts(parts)
        };
        Self {
            role: msg.role.clone(),
            content,
        }
    }
}

impl OaiMessageWithTools {
    /// Convert a ChatMessage to an OAI message with tool calling support.
    fn from_chat(msg: &crate::ollama::ChatMessage) -> Self {
        // role="tool" messages are tool results
        if msg.role == "tool" {
            return Self {
                role: "tool".to_string(),
                content: Some(OaiContent::Text(msg.content.clone())),
                tool_calls: None,
                tool_call_id: msg.tool_call_id.clone(),
                name: msg.name.clone(),
            };
        }

        // Assistant messages with tool_calls
        if msg.role == "assistant" && !msg.tool_calls.is_empty() {
            let tc: Vec<serde_json::Value> = msg.tool_calls.iter().map(|tc| {
                // OpenAI API requires `arguments` to be a JSON string, not an object.
                let args_str = match &tc.function.arguments {
                    serde_json::Value::String(s) => s.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                serde_json::json!({
                    "id": tc.id,
                    "type": tc.call_type,
                    "function": {
                        "name": tc.function.name,
                        "arguments": args_str
                    }
                })
            }).collect();
            return Self {
                role: "assistant".to_string(),
                content: if msg.content.is_empty() { None } else { Some(OaiContent::Text(msg.content.clone())) },
                tool_calls: Some(tc),
                tool_call_id: None,
                name: None,
            };
        }

        // Regular messages
        let content = if msg.images.is_empty() {
            OaiContent::Text(msg.content.clone())
        } else {
            let mut parts = vec![OaiContentPart::Text {
                text: msg.content.clone(),
            }];
            for img in &msg.images {
                parts.push(OaiContentPart::ImageUrl {
                    image_url: OaiImageUrl {
                        url: format!("data:image/png;base64,{}", img),
                    },
                });
            }
            OaiContent::Parts(parts)
        };
        Self {
            role: msg.role.clone(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct OaiRequest {
    model: String,
    messages: Vec<OaiMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<OaiResponseFormat>,
    /// OpenAI-compatible tool definitions for native function calling.
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Serialize)]
struct OaiResponseFormat {
    r#type: String,
}

#[derive(Debug, Deserialize)]
struct OaiStreamChunk {
    choices: Vec<OaiStreamChoice>,
    #[serde(default)]
    usage: Option<OaiUsage>,
}

#[derive(Debug, Deserialize)]
struct OaiUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OaiStreamChoice {
    delta: OaiStreamDelta,
}

#[derive(Debug, Deserialize)]
struct OaiStreamDelta {
    content: Option<String>,
    /// Tool call deltas from native function calling.
    /// OpenAI streams these incrementally: first chunk has id+name, subsequent chunks have argument fragments.
    #[serde(default)]
    tool_calls: Option<Vec<OaiStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OaiStreamToolCall {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<OaiStreamToolCallFunction>,
}

#[derive(Debug, Deserialize)]
struct OaiStreamToolCallFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}
