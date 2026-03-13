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
    /// ChatGPT Account ID for OAuth mode (sent as `ChatGPT-Account-Id` header).
    chatgpt_account_id: Option<String>,
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
            chatgpt_account_id: None,
        }
    }

    /// Create a client configured for ChatGPT OAuth (subscription-based access).
    pub fn new_chatgpt_oauth(
        base_url: String,
        access_token: String,
        account_id: Option<String>,
    ) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: Some(access_token),
            chatgpt_account_id: account_id,
        }
    }

    /// Apply auth headers to a request builder.
    fn apply_auth(&self, mut rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(key) = &self.api_key {
            rb = rb.header("Authorization", format!("Bearer {}", key));
        }
        if let Some(account_id) = &self.chatgpt_account_id {
            rb = rb.header("ChatGPT-Account-Id", account_id);
        }
        rb
    }

    /// Whether this client uses the ChatGPT Responses API (OAuth mode).
    fn uses_responses_api(&self) -> bool {
        self.chatgpt_account_id.is_some()
    }

    /// Try to fetch context window size from the provider's models endpoint.
    /// Works for: Gemini (`inputTokenLimit`), OpenAI (`context_window` if present).
    /// Returns None if not available.
    pub async fn get_context_window(&self, model: &str) -> Option<usize> {
        // Try OpenAI-compatible /models/{id} endpoint
        let url = format!("{}/models/{}", self.base_url, model);
        let resp = self.apply_auth(self.http.get(&url)).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let json: serde_json::Value = resp.json().await.ok()?;
        // Gemini returns inputTokenLimit at top level
        if let Some(limit) = json.get("inputTokenLimit").and_then(|v| v.as_u64()) {
            return Some(limit as usize);
        }
        // Some OpenAI-compatible providers return context_window or context_length
        if let Some(limit) = json.get("context_window").and_then(|v| v.as_u64()) {
            return Some(limit as usize);
        }
        if let Some(limit) = json.get("context_length").and_then(|v| v.as_u64()) {
            return Some(limit as usize);
        }
        None
    }

    /// Streaming text chat completion (SSE format).
    /// Uses Responses API for ChatGPT OAuth, Chat Completions for standard API.
    pub async fn chat_text_stream(
        &self,
        model: &str,
        messages: &[crate::ollama::ChatMessage],
    ) -> Result<impl Stream<Item = Result<StreamChunk>> + Send> {
        let total_len: usize = messages.iter().map(|m| m.content.len()).sum();
        tracing::info!(
            "OpenAI stream: model={} msgs={} chars={}",
            model,
            messages.len(),
            total_len
        );
        if let Some(last) = messages.last() {
            tracing::debug!("Last msg ({}): {:.200}", last.role, last.content);
        }

        let rb = if self.uses_responses_api() {
            // ChatGPT Responses API format
            let url = format!("{}/responses", self.base_url);
            // Separate system instructions from input messages
            let mut instructions = String::new();
            let mut input_items: Vec<serde_json::Value> = Vec::new();
            for msg in messages {
                if msg.role == "system" {
                    if !instructions.is_empty() {
                        instructions.push('\n');
                    }
                    instructions.push_str(&msg.content);
                } else {
                    input_items.push(responses_api_input_item(msg));
                }
            }
            let mut req = serde_json::json!({
                "model": model,
                "input": input_items,
                "stream": true,
                "store": false,
            });
            if !instructions.is_empty() {
                req["instructions"] = serde_json::Value::String(instructions);
            }
            tracing::debug!("Responses API request to {}", url);
            self.apply_auth(self.http.post(url).json(&req))
        } else {
            // Standard Chat Completions format
            let url = format!("{}/chat/completions", self.base_url);
            let oai_messages: Vec<OaiMessage> =
                messages.iter().map(OaiMessage::from_chat).collect();
            let stream_options = if self.base_url.contains("googleapis.com") {
                None
            } else {
                Some(OaiStreamOptions { include_usage: true })
            };
            let req = OaiRequest {
                model: model.to_string(),
                messages: oai_messages,
                stream: true,
                stream_options,
                response_format: None,
                tools: None,
            };
            self.apply_auth(self.http.post(url).json(&req))
        };
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

        // Stream SSE lines
        let byte_stream = resp
            .bytes_stream()
            .map(|item| item.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
        let reader = tokio_util::io::StreamReader::new(byte_stream);
        let lines =
            tokio_util::codec::FramedRead::new(reader, tokio_util::codec::LinesCodec::new());

        use futures_util::StreamExt;
        let is_responses_api = self.uses_responses_api();
        let token_stream = lines.filter_map(move |line_result| async move {
            let line = match line_result {
                Ok(l) => l,
                Err(e) => return Some(Err(anyhow::anyhow!("stream error: {}", e))),
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }

            // Skip "event:" lines — we parse by data content
            if trimmed.starts_with("event:") {
                return None;
            }

            let data = match trimmed.strip_prefix("data: ") {
                Some(d) => d.trim(),
                None => return None,
            };
            if data == "[DONE]" {
                return None;
            }

            if is_responses_api {
                // Responses API SSE: parse generic JSON, look for delta text or usage
                let val: serde_json::Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => return None, // skip unparseable events
                };
                let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match event_type {
                    "response.output_text.delta" => {
                        let delta = val.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                        if delta.is_empty() {
                            None
                        } else {
                            Some(Ok(StreamChunk::Token(delta.to_string())))
                        }
                    }
                    "response.completed" => {
                        // Extract usage from response.completed event
                        if let Some(usage) = val.get("response").and_then(|r| r.get("usage")) {
                            let input = usage
                                .get("input_tokens")
                                .and_then(|v| v.as_u64())
                                .map(|v| v as usize);
                            let output = usage
                                .get("output_tokens")
                                .and_then(|v| v.as_u64())
                                .map(|v| v as usize);
                            Some(Ok(StreamChunk::Usage(TokenUsage {
                                prompt_tokens: input,
                                completion_tokens: output,
                                total_tokens: input.zip(output).map(|(a, b)| a + b),
                            })))
                        } else {
                            None
                        }
                    }
                    "error" => {
                        let msg = val
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown error");
                        Some(Err(anyhow::anyhow!("Responses API error: {}", msg)))
                    }
                    _ => None, // skip other event types
                }
            } else {
                // Standard Chat Completions SSE
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
            }
        });

        Ok(token_stream)
    }
    /// Streaming chat with native tool calling support (SSE format).
    /// Sends tool definitions and parses incremental tool_call deltas.
    /// Uses Responses API for ChatGPT OAuth, Chat Completions for standard API.
    pub async fn chat_tool_stream(
        &self,
        model: &str,
        messages: &[crate::ollama::ChatMessage],
        tools: Vec<serde_json::Value>,
    ) -> Result<impl Stream<Item = Result<StreamChunk>> + Send> {
        let total_len: usize = messages.iter().map(|m| m.content.len()).sum();
        tracing::info!(
            "OpenAI tool stream: model={} msgs={} chars={} tools={}",
            model,
            messages.len(),
            total_len,
            tools.len()
        );

        // ChatGPT Responses API requires tool call IDs starting with 'fc'.
        // Sanitize IDs that may come from other providers or legacy sessions.
        let ensure_fc_prefix = |id: &str| -> String {
            if id.starts_with("fc") {
                id.to_string()
            } else {
                format!("fc_{id}")
            }
        };

        let rb =
            if self.uses_responses_api() {
                // ChatGPT Responses API with tools
                let url = format!("{}/responses", self.base_url);
                let mut instructions = String::new();
                let mut input_items: Vec<serde_json::Value> = Vec::new();
                for msg in messages {
                    if msg.role == "system" {
                        if !instructions.is_empty() {
                            instructions.push('\n');
                        }
                        instructions.push_str(&msg.content);
                    } else if msg.role == "tool" {
                        // Tool result messages → function_call_output items
                        let call_id = msg
                            .tool_call_id
                            .as_deref()
                            .map(|id| ensure_fc_prefix(id))
                            .unwrap_or_default();
                        input_items.push(serde_json::json!({
                            "type": "function_call_output",
                            "call_id": call_id,
                            "output": msg.content,
                        }));
                    } else if msg.role == "assistant" && !msg.tool_calls.is_empty() {
                        // Assistant message with tool calls → emit text + function_call items
                        if !msg.content.is_empty() {
                            input_items.push(serde_json::json!({
                                "role": "assistant",
                                "content": msg.content,
                            }));
                        }
                        for tc in &msg.tool_calls {
                            let tc_id = ensure_fc_prefix(&tc.id);
                            input_items.push(serde_json::json!({
                                "type": "function_call",
                                "id": tc_id,
                                "call_id": tc_id,
                                "name": tc.function.name,
                                "arguments": match &tc.function.arguments {
                                    serde_json::Value::String(s) => s.clone(),
                                    other => serde_json::to_string(other).unwrap_or_default(),
                                },
                            }));
                        }
                    } else {
                        input_items.push(responses_api_input_item(msg));
                    }
                }

                // Convert OpenAI-style tool defs to Responses API function tools
                let resp_tools: Vec<serde_json::Value> = tools.iter().filter_map(|t| {
                let func = t.get("function")?;
                Some(serde_json::json!({
                    "type": "function",
                    "name": func.get("name")?,
                    "description": func.get("description").unwrap_or(&serde_json::Value::Null),
                    "parameters": func.get("parameters").unwrap_or(&serde_json::Value::Null),
                }))
            }).collect();

                let mut req = serde_json::json!({
                    "model": model,
                    "input": input_items,
                    "tools": resp_tools,
                    "stream": true,
                    "store": false,
                });
                if !instructions.is_empty() {
                    req["instructions"] = serde_json::Value::String(instructions);
                }
                tracing::debug!("Responses API tool request to {}", url);
                self.apply_auth(self.http.post(url).json(&req))
            } else {
                // Standard Chat Completions format
                let url = format!("{}/chat/completions", self.base_url);
                let oai_messages: Vec<OaiMessageWithTools> = messages
                    .iter()
                    .map(OaiMessageWithTools::from_chat)
                    .collect();
                let mut req = serde_json::json!({
                    "model": model,
                    "messages": oai_messages,
                    "stream": true,
                    "tools": tools,
                });
                // Only include stream_options for providers known to support it.
                // Gemini's OpenAI-compatible API doesn't support stream_options.
                if !self.base_url.contains("googleapis.com") {
                    req["stream_options"] = serde_json::json!({"include_usage": true});
                }
                self.apply_auth(self.http.post(url).json(&req))
            };

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
        let lines =
            tokio_util::codec::FramedRead::new(reader, tokio_util::codec::LinesCodec::new());

        use crate::agent_manager::models::ToolCallChunk;
        use futures_util::StreamExt;
        let is_responses_api = self.uses_responses_api();
        // Use map + flat_map so a single SSE line can yield multiple
        // StreamChunks (e.g. batched tool call deltas from Gemini/Groq).
        let token_stream = lines
            .map(move |line_result| {
                let line = match line_result {
                    Ok(l) => l,
                    Err(e) => return vec![Err(anyhow::anyhow!("stream error: {}", e))],
                };
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with("event:") {
                    return vec![];
                }
                let data = match trimmed.strip_prefix("data: ") {
                    Some(d) => d.trim(),
                    None => return vec![],
                };
                if data == "[DONE]" {
                    return vec![];
                }

                if is_responses_api {
                    let val: serde_json::Value = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(_) => return vec![],
                    };
                    let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    // tracing::debug!("Responses API event: {}", event_type);
                    match event_type {
                        "response.output_text.delta" => {
                            let delta = val.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                            if delta.is_empty() {
                                vec![]
                            } else {
                                vec![Ok(StreamChunk::Token(delta.to_string()))]
                            }
                        }
                        "response.function_call_arguments.delta" => {
                            let args_delta = val
                                .get("delta")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let item_id = val
                                .get("item_id")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            let output_index =
                                val.get("output_index")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0) as usize;
                            vec![Ok(StreamChunk::ToolCall(ToolCallChunk {
                                index: output_index,
                                id: item_id,
                                name: None,
                                arguments_delta: Some(args_delta),
                                thought_signature: None,
                            }))]
                        }
                        "response.function_call_arguments.done" => {
                            // Emit name/id only — deltas already accumulated the full args.
                            let call_id = val
                                .get("call_id")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            let name = val
                                .get("name")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            let output_index =
                                val.get("output_index")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0) as usize;
                            vec![Ok(StreamChunk::ToolCall(ToolCallChunk {
                                index: output_index,
                                id: call_id,
                                name,
                                arguments_delta: None,
                                thought_signature: None,
                            }))]
                        }
                        "response.output_item.added" => {
                            if let Some(item) = val.get("item") {
                                if item.get("type").and_then(|v| v.as_str())
                                    == Some("function_call")
                                {
                                    let call_id = item
                                        .get("call_id")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());
                                    let name = item
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());
                                    let output_index = val
                                        .get("output_index")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0)
                                        as usize;
                                    return vec![Ok(StreamChunk::ToolCall(ToolCallChunk {
                                        index: output_index,
                                        id: call_id,
                                        name,
                                        arguments_delta: None,
                                        thought_signature: None,
                                    }))];
                                }
                            }
                            vec![]
                        }
                        "response.completed" => {
                            if let Some(usage) = val.get("response").and_then(|r| r.get("usage")) {
                                let input = usage
                                    .get("input_tokens")
                                    .and_then(|v| v.as_u64())
                                    .map(|v| v as usize);
                                let output = usage
                                    .get("output_tokens")
                                    .and_then(|v| v.as_u64())
                                    .map(|v| v as usize);
                                vec![Ok(StreamChunk::Usage(TokenUsage {
                                    prompt_tokens: input,
                                    completion_tokens: output,
                                    total_tokens: input.zip(output).map(|(a, b)| a + b),
                                }))]
                            } else {
                                vec![]
                            }
                        }
                        "error" => {
                            let msg = val
                                .get("message")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown error");
                            vec![Err(anyhow::anyhow!("Responses API error: {}", msg))]
                        }
                        _ => vec![],
                    }
                } else {
                    // Standard Chat Completions SSE
                    let sanitized = sanitize_json(data);
                    let chunk: OaiStreamChunk = match serde_json::from_str(&sanitized) {
                        Ok(c) => c,
                        Err(e) => {
                            return vec![Err(anyhow::anyhow!(
                                "openai json parse error: {} (data: {})",
                                e,
                                data
                            ))];
                        }
                    };

                    if let Some(usage) = chunk.usage {
                        return vec![Ok(StreamChunk::Usage(TokenUsage {
                            prompt_tokens: usage.prompt_tokens.map(|v| v as usize),
                            completion_tokens: usage.completion_tokens.map(|v| v as usize),
                            total_tokens: usage.total_tokens.map(|v| v as usize),
                        }))];
                    }

                    // Extract Gemini thought_signature — check chunk level first, then choice level.
                    let chunk_level_sig = chunk.extra_content
                        .as_ref()
                        .and_then(|ec| ec.google.as_ref())
                        .and_then(|g| g.thought_signature.clone());

                    let Some(choice) = chunk.choices.into_iter().next() else {
                        tracing::trace!("SSE chunk with no choices: {}", &sanitized);
                        return vec![];
                    };

                    let thought_sig = chunk_level_sig.or_else(|| {
                        choice.extra_content
                            .as_ref()
                            .and_then(|ec| ec.google.as_ref())
                            .and_then(|g| g.thought_signature.clone())
                    });

                    // Emit ALL tool call deltas from this chunk (not just the first)
                    if let Some(tool_calls) = choice.delta.tool_calls {
                        let mut chunks: Vec<Result<StreamChunk>> = Vec::new();
                        for tc in tool_calls.into_iter() {
                            let name = tc.function.as_ref().and_then(|f| f.name.clone());
                            let args_delta =
                                tc.function.as_ref().and_then(|f| f.arguments.clone());
                            // Extract thought_signature from the tool call's own extra_content
                            // (Gemini puts it here), falling back to choice/chunk level.
                            let sig = tc.extra_content
                                .as_ref()
                                .and_then(|ec| ec.google.as_ref())
                                .and_then(|g| g.thought_signature.clone())
                                .or_else(|| thought_sig.clone());
                            if sig.is_some() && name.is_some() {
                                tracing::debug!("Gemini thought_signature captured for tool call '{}'", name.as_deref().unwrap_or("?"));
                            }
                            chunks.push(Ok(StreamChunk::ToolCall(ToolCallChunk {
                                index: tc.index,
                                id: tc.id,
                                name,
                                arguments_delta: args_delta,
                                thought_signature: sig,
                            })));
                        }
                        return chunks;
                    }

                    let content = choice.delta.content.unwrap_or_default();
                    if content.is_empty() {
                        vec![]
                    } else {
                        vec![Ok(StreamChunk::Token(content))]
                    }
                }
            })
            .flat_map(futures_util::stream::iter);

        Ok(token_stream)
    }
}

// --- Responses API helpers ---

/// Build a Responses API input item from a ChatMessage, including images if present.
/// Responses API uses `input_image` content parts (not `image_url` like Chat Completions).
fn responses_api_input_item(msg: &crate::ollama::ChatMessage) -> serde_json::Value {
    if msg.images.is_empty() {
        serde_json::json!({
            "role": msg.role,
            "content": msg.content,
        })
    } else {
        let mut content_parts = vec![serde_json::json!({
            "type": "input_text",
            "text": msg.content,
        })];
        for img in &msg.images {
            content_parts.push(serde_json::json!({
                "type": "input_image",
                "image_url": format!("data:image/png;base64,{}", img),
            }));
        }
        serde_json::json!({
            "role": msg.role,
            "content": content_parts,
        })
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
    /// Gemini extra_content with thought_signature (must be echoed back for Gemini 3+).
    #[serde(skip_serializing_if = "Option::is_none")]
    extra_content: Option<serde_json::Value>,
}

impl OaiMessage {
    fn from_chat(msg: &crate::ollama::ChatMessage) -> Self {
        // In text mode (no native tools), convert tool-related messages to
        // plain roles so the API doesn't see orphaned tool_call_id/tool_calls.
        let role = if msg.role == "tool" {
            // Tool results become user messages — avoids 400 errors from
            // orphaned role="tool" without matching tool_calls in history.
            "user".to_string()
        } else {
            msg.role.clone()
        };
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
        Self { role, content }
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
                extra_content: None,
            };
        }

        // Assistant messages with tool_calls
        if msg.role == "assistant" && !msg.tool_calls.is_empty() {
            // Gemini 3+ requires thought_signature on tool calls.
            // Use the real signature if captured, otherwise use the documented
            // dummy value that bypasses the validator (for legacy history or
            // when the stream didn't include one).
            let first_sig = msg.tool_calls.iter()
                .find_map(|tc| tc.thought_signature.clone());

            let tc: Vec<serde_json::Value> = msg
                .tool_calls
                .iter()
                .enumerate()
                .map(|(i, tc)| {
                    // OpenAI API requires `arguments` to be a JSON string, not an object.
                    let args_str = match &tc.function.arguments {
                        serde_json::Value::String(s) => s.clone(),
                        other => serde_json::to_string(other).unwrap_or_default(),
                    };
                    let mut obj = serde_json::json!({
                        "id": tc.id,
                        "type": tc.call_type,
                        "function": {
                            "name": tc.function.name,
                            "arguments": args_str
                        }
                    });
                    // Attach thought_signature to first tool call (Gemini spec)
                    if i == 0 {
                        let sig = tc.thought_signature.as_deref()
                            .or(first_sig.as_deref())
                            .unwrap_or("skip_thought_signature_validator");
                        obj["extra_content"] = serde_json::json!({
                            "google": { "thought_signature": sig }
                        });
                    }
                    obj
                })
                .collect();
            // Also set message-level extra_content (some providers read it here).
            let sig = first_sig.as_deref()
                .unwrap_or("skip_thought_signature_validator");
            let extra_content = Some(serde_json::json!({
                "google": { "thought_signature": sig }
            }));
            return Self {
                role: "assistant".to_string(),
                content: if msg.content.is_empty() {
                    None
                } else {
                    Some(OaiContent::Text(msg.content.clone()))
                },
                tool_calls: Some(tc),
                tool_call_id: None,
                name: None,
                extra_content,
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
            extra_content: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct OaiRequest {
    model: String,
    messages: Vec<OaiMessage>,
    stream: bool,
    /// Request token usage in the final streaming chunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<OaiStreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<OaiResponseFormat>,
    /// OpenAI-compatible tool definitions for native function calling.
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Serialize)]
struct OaiStreamOptions {
    include_usage: bool,
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
    /// Gemini extra_content at chunk level (alternative location for thought_signature).
    #[serde(default)]
    extra_content: Option<OaiExtraContent>,
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
    /// Gemini extra_content (contains thought_signature for tool calls).
    #[serde(default)]
    extra_content: Option<OaiExtraContent>,
}

/// Gemini-specific extra_content on choices (OpenAI-compatible endpoint).
#[derive(Debug, Deserialize)]
struct OaiExtraContent {
    #[serde(default)]
    google: Option<OaiGoogleExtra>,
}

#[derive(Debug, Deserialize)]
struct OaiGoogleExtra {
    #[serde(default)]
    thought_signature: Option<String>,
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
    /// Gemini thought_signature lives here (on each tool_call in the delta).
    #[serde(default)]
    extra_content: Option<OaiExtraContent>,
}

#[derive(Debug, Deserialize)]
struct OaiStreamToolCallFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}
