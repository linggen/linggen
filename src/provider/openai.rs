use crate::provider::models::{StreamChunk, TokenUsage};
use anyhow::Result;
use futures_util::Stream;
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Convert a canonical tool def (`{type:"function", function:{name, description, parameters}}`)
/// into the shape the OpenAI Responses API wants:
/// - flattened: top-level `name` / `description` / `parameters` / `strict`
/// - parameters strictified: every property in `required[]`, optionals widened
///   to nullable unions, `additionalProperties:false` at every nesting level
/// - `strict: true` set on the tool wrapper
///
/// Single source of truth for the OpenAI wire shape. Called from the live
/// request path in `chat_tool_stream` AND from `server/chat/admin.rs`'s
/// system-prompt export — both surfaces emit identical JSON so users
/// inspecting the export see exactly what the wire receives.
pub fn wire_tool_def(canonical: &serde_json::Value) -> Option<serde_json::Value> {
    let func = canonical.get("function")?;
    let params = func
        .get("parameters")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    // Per OpenAI's function-calling guide
    // (https://developers.openai.com/api/docs/guides/function-calling):
    // `strict: true` REQUIRES every property to be listed in `required[]`
    // with `additionalProperties: false`; optional params are expressed as
    // nullable types (`["string", "null"]`) but must STILL appear in
    // `required[]`. `oneOf`/`allOf` are rejected under strict and `anyOf`
    // needs each branch strictified. To opt out, "explicitly set `strict:
    // false`".
    //
    // We only opt a tool into strict when `strictify_for_openai` would NOT
    // have to change which fields are required — i.e. the schema is already
    // fully-required at every level (and composite-free). The moment a tool
    // has an optional field anywhere, strict's all-required rule distorts
    // the contract: every optional becomes required-nullable, and reasoning
    // models (gpt-5.x on the Responses API) react by null-filling every
    // field or emitting empty/degenerate calls (e.g. `[{}]`) — which then
    // fail the runtime "at least one non-empty" check and trip the
    // consecutive-empty-response bail. So any tool with optional fields —
    // PageUpdate, Read (file_path required, offset/limit optional), Grep,
    // … — is sent `strict:false` with its ORIGINAL schema, letting the
    // model omit the fields it isn't using. Strict stays on only for
    // genuinely all-required tools, where it adds `additionalProperties:
    // false` without touching `required`.
    if !crate::engine::tools::json_schema::is_fully_required(&params) {
        return Some(serde_json::json!({
            "type": "function",
            "name": func.get("name")?,
            "description": func.get("description").unwrap_or(&serde_json::Value::Null),
            "parameters": params,
            "strict": false,
        }));
    }

    let strict_params =
        crate::engine::tools::json_schema::strictify_for_openai(params);
    Some(serde_json::json!({
        "type": "function",
        "name": func.get("name")?,
        "description": func.get("description").unwrap_or(&serde_json::Value::Null),
        "parameters": strict_params,
        "strict": true,
    }))
}

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

/// Pull the `error.message` out of an OpenAI-shaped error body, if present.
fn extract_error_message(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    v.get("error")?.get("message")?.as_str().map(String::from)
}

#[derive(Clone)]
pub struct OpenAiClient {
    http: Client,
    base_url: String,
    api_key: Option<String>,
    /// ChatGPT Account ID for OAuth mode (sent as `ChatGPT-Account-Id` header).
    chatgpt_account_id: Option<String>,
    /// When true, reload token from codex_auth.json on each request (auto-refresh).
    codex_auth_live: bool,
    /// When true, resolve the linggen.dev account token on each request, so
    /// signing in/out is picked up without a daemon restart.
    linggen_account_live: bool,
}

impl OpenAiClient {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .connect_timeout(std::time::Duration::from_secs(10))
            // The total timeout above does NOT govern bodies consumed via
            // bytes_stream() (reqwest gap) — a backend that accepts the
            // request and then goes silent wedges the run forever (observed
            // live 2026-07-10: 35 min on one Responses API call). read_timeout
            // bounds every socket read, streaming included; reasoning models'
            // silent thinking gaps stay well under it, and the resulting
            // error is transient-classified so the model fallback chain runs.
            .read_timeout(std::time::Duration::from_secs(180))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            chatgpt_account_id: None,
            codex_auth_live: false,
            linggen_account_live: false,
        }
    }

    /// Create a client configured for ChatGPT OAuth (subscription-based access).
    /// Reads fresh tokens from codex_auth.json on each request.
    pub fn new_chatgpt_oauth(
        base_url: String,
        access_token: String,
        account_id: Option<String>,
    ) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .connect_timeout(std::time::Duration::from_secs(10))
            // The total timeout above does NOT govern bodies consumed via
            // bytes_stream() (reqwest gap) — a backend that accepts the
            // request and then goes silent wedges the run forever (observed
            // live 2026-07-10: 35 min on one Responses API call). read_timeout
            // bounds every socket read, streaming included; reasoning models'
            // silent thinking gaps stay well under it, and the resulting
            // error is transient-classified so the model fallback chain runs.
            .read_timeout(std::time::Duration::from_secs(180))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: Some(access_token),
            chatgpt_account_id: account_id,
            codex_auth_live: true,
            linggen_account_live: false,
        }
    }

    /// Create a client for the Linggen Cloud proxy (linggen.dev/api/llm).
    /// The account token is resolved fresh on each request.
    pub fn new_linggen_account(base_url: String) -> Self {
        let mut client = Self::new(base_url, None);
        client.linggen_account_live = true;
        client
    }

    /// Apply auth headers to a request builder.
    fn apply_auth(&self, mut rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.codex_auth_live {
            // Read fresh token from disk so login/refresh is picked up immediately
            let tokens = crate::provider::codex_auth::CodexAuthTokens::load(&crate::provider::codex_auth::codex_auth_file());
            if let Some(ref token) = tokens.access_token {
                rb = rb.header("Authorization", format!("Bearer {}", token));
            }
            if let Some(ref account_id) = tokens.account_id {
                rb = rb.header("ChatGPT-Account-Id", account_id);
            }
        } else if self.linggen_account_live {
            if let Some((token, _)) = crate::account::resolve_token() {
                rb = rb.header("Authorization", format!("Bearer {}", token));
            }
        } else {
            if let Some(key) = &self.api_key {
                rb = rb.header("Authorization", format!("Bearer {}", key));
            }
            if let Some(account_id) = &self.chatgpt_account_id {
                rb = rb.header("ChatGPT-Account-Id", account_id);
            }
        }
        rb
    }

    /// Tag the request with the app product for per-app usage attribution on
    /// the Linggen Cloud proxy (it meters tokens per X-Linggen-App bucket).
    /// No-op for every other provider.
    fn with_app_header(&self, rb: reqwest::RequestBuilder, app: Option<&str>) -> reqwest::RequestBuilder {
        match app {
            Some(app) if self.linggen_account_live => rb.header("X-Linggen-App", app),
            _ => rb,
        }
    }

    /// Format a non-success provider response into a user-facing error.
    /// Payment errors carry the proxy's message (subscribe / trial CTA)
    /// verbatim — Linggen Cloud 402s get a `BILLING_REQUIRED:` prefix so the
    /// chat UI renders the subscribe card; live-auth 401s become sign-in CTAs.
    fn provider_error(&self, status: reqwest::StatusCode, text: String) -> anyhow::Error {
        if self.codex_auth_live && status == reqwest::StatusCode::UNAUTHORIZED {
            return anyhow::anyhow!(
                "AUTH_REQUIRED: ChatGPT session expired. Sign in with ChatGPT to continue."
            );
        }
        if self.linggen_account_live && status == reqwest::StatusCode::UNAUTHORIZED {
            return anyhow::anyhow!(
                "AUTH_REQUIRED: linggen.dev sign-in missing or expired. Run `ling account login`."
            );
        }
        if status == reqwest::StatusCode::PAYMENT_REQUIRED {
            if let Some(msg) = extract_error_message(&text) {
                if self.linggen_account_live {
                    return anyhow::anyhow!("BILLING_REQUIRED: {msg}");
                }
                return anyhow::anyhow!("{msg}");
            }
        }
        let truncated = if text.len() > 500 {
            format!("{}… ({} chars)", &text[..500], text.len())
        } else {
            text
        };
        anyhow::anyhow!("openai error ({}): {}", status, truncated)
    }

    /// Send a request, applying auth fresh each attempt. In ChatGPT OAuth
    /// mode a 401 means the access token expired — refresh it once using the
    /// stored refresh token and retry transparently. Only when the refresh
    /// itself fails (refresh token revoked/expired) does the 401 propagate,
    /// where the caller turns it into an `AUTH_REQUIRED:` sign-in CTA.
    async fn send_with_oauth_retry(
        &self,
        rb: reqwest::RequestBuilder,
    ) -> anyhow::Result<reqwest::Response> {
        let first = self
            .apply_auth(rb.try_clone().ok_or_else(|| {
                anyhow::anyhow!("request body not cloneable for auth retry")
            })?);
        let resp = first.send().await?;
        if resp.status() != reqwest::StatusCode::UNAUTHORIZED || !self.codex_auth_live {
            return Ok(resp);
        }
        if !self.try_refresh_codex_tokens().await {
            return Ok(resp);
        }
        // apply_auth re-reads codex_auth.json, so the retry uses the fresh token.
        Ok(self.apply_auth(rb).send().await?)
    }

    /// Attempt a one-shot refresh of the on-disk ChatGPT OAuth tokens.
    /// Returns true if a new token was fetched and saved.
    async fn try_refresh_codex_tokens(&self) -> bool {
        use crate::provider::codex_auth;
        let tokens = codex_auth::CodexAuthTokens::load(&codex_auth::codex_auth_file());
        if tokens.refresh_token.is_none() {
            return false;
        }
        match codex_auth::refresh_tokens(&self.http, &tokens).await {
            Ok(new) => match new.save(&codex_auth::codex_auth_file()) {
                Ok(()) => {
                    tracing::info!("Refreshed expired ChatGPT OAuth token after 401.");
                    true
                }
                Err(e) => {
                    tracing::warn!("Failed to save refreshed ChatGPT tokens: {}", e);
                    false
                }
            },
            Err(e) => {
                tracing::warn!("ChatGPT token refresh after 401 failed: {}", e);
                false
            }
        }
    }

    /// Whether this client uses the ChatGPT Responses API (OAuth mode).
    pub(crate) fn uses_responses_api(&self) -> bool {
        self.chatgpt_account_id.is_some()
    }

    /// Try to fetch context window size from the provider's models endpoint.
    /// Works for: Gemini (`inputTokenLimit`), OpenAI (`context_window` if present).
    /// Returns None if not available.
    pub async fn get_context_window(&self, model: &str) -> Option<usize> {
        // Try OpenAI-compatible /models/{id} endpoint. Best-effort metadata:
        // a backend that never answers must not hold up an agent turn for the
        // client's 180s read timeout, so cap the whole probe at 5s.
        let url = format!("{}/models/{}", self.base_url, model);
        let resp = self
            .apply_auth(self.http.get(&url))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .ok()?;
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

    /// Apply reasoning effort to a request based on provider.
    /// - OpenAI (GPT-5, o3, o4-mini): `reasoning_effort` field
    /// - Gemini 2.5: `generationConfig.thinkingConfig.thinkingBudget`
    /// - Others: no-op (unknown params are silently ignored by most providers)
    /// Check if a model supports reasoning effort control.
    fn model_supports_reasoning(model: &str, is_gemini: bool) -> bool {
        let m = model.to_lowercase();
        // OpenAI reasoning models
        if m.contains("gpt-5") || m.contains("o3") || m.contains("o4") || m.contains("o1") {
            return true;
        }
        // Gemini 2.5 thinking models
        if is_gemini && m.contains("2.5") {
            return true;
        }
        // DeepSeek reasoning
        if m.contains("deepseek-r") || m.contains("deepseek-reasoner") {
            return true;
        }
        false
    }

    fn apply_reasoning_effort(
        req: &mut serde_json::Value,
        effort: Option<&str>,
        is_gemini: bool,
        model: &str,
    ) {
        let Some(effort) = effort else { return };
        let effort_lower = effort.to_lowercase();
        if !["low", "medium", "high"].contains(&effort_lower.as_str()) {
            return;
        }

        // Only send to models that support it — avoid API errors on others
        if !Self::model_supports_reasoning(model, is_gemini) {
            return;
        }

        if is_gemini && model.to_lowercase().contains("2.5") {
            // Gemini uses thinkingBudget (token count): low=1024, medium=8192, high=32768
            let budget = match effort_lower.as_str() {
                "low" => 1024,
                "high" => 32768,
                _ => 8192, // medium
            };
            req["generationConfig"] = serde_json::json!({
                "thinkingConfig": { "thinkingBudget": budget }
            });
        } else {
            // OpenAI-compatible: reasoning_effort field (GPT-5, o3, o4-mini)
            req["reasoning_effort"] = serde_json::json!(effort_lower);
        }
    }

    /// Streaming text chat completion (SSE format).
    /// Uses Responses API for ChatGPT OAuth, Chat Completions for standard API.
    pub async fn chat_text_stream(
        &self,
        model: &str,
        messages: &[crate::message::ChatMessage],
        reasoning_effort: Option<&str>,
        app: Option<&str>,
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
            self.http.post(url).json(&req)
        } else {
            // Standard Chat Completions format
            let url = format!("{}/chat/completions", self.base_url);
            let oai_messages: Vec<OaiMessage> =
                messages.iter().map(OaiMessage::from_chat).collect();
            let is_gemini = self.base_url.contains("googleapis.com");
            let stream_options = if is_gemini {
                None
            } else {
                Some(OaiStreamOptions { include_usage: true })
            };
            let mut req = serde_json::json!({
                "model": model,
                "messages": oai_messages,
                "stream": true,
            });
            if let Some(opts) = stream_options {
                req["stream_options"] = serde_json::json!({"include_usage": opts.include_usage});
            }
            // Gemini 2.5 thinking models can exhaust their output budget on
            // internal reasoning and return empty responses.
            if is_gemini && model.contains("2.5") {
                req["max_completion_tokens"] = serde_json::json!(65536);
            }
            // Apply reasoning effort per provider
            Self::apply_reasoning_effort(&mut req, reasoning_effort, is_gemini, model);
            self.http.post(url).json(&req)
        };
        let resp = self.send_with_oauth_retry(self.with_app_header(rb, app)).await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(self.provider_error(status, text));
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
                Err(e) => return Some(Err(crate::provider::stream_read_error(e))),
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
        messages: &[crate::message::ChatMessage],
        tools: Vec<serde_json::Value>,
        reasoning_effort: Option<&str>,
        app: Option<&str>,
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

                // Convert OpenAI-style tool defs to Responses API function tools.
                // Single source of truth: `wire_tool_def` handles flattening +
                // strict-mode rewrite; admin's system-prompt export calls the
                // same helper so what the user sees in the export matches what
                // the wire receives.
                let resp_tools: Vec<serde_json::Value> = tools
                    .iter()
                    .filter_map(|t| wire_tool_def(t))
                    .collect();

                let mut req = serde_json::json!({
                    "model": model,
                    "input": input_items,
                    "tools": resp_tools,
                    "stream": true,
                    "store": false,
                });
                // Reasoning models (gpt-5.x, o-series) on the Responses API take
                // a `reasoning: { effort }` OBJECT — not the Chat Completions
                // `reasoning_effort` scalar. The Responses branch never applied
                // it, so the configured effort was silently dropped and the
                // model always ran at default effort. `max_output_tokens` is
                // deliberately left unset: OpenAI defaults it to the model
                // maximum, so — unlike Gemini's small thinking budget (which we
                // DO bump) — reasoning can't starve the visible output into an
                // empty response.
                if let Some(effort) = reasoning_effort {
                    let e = effort.to_lowercase();
                    if ["low", "medium", "high"].contains(&e.as_str())
                        && Self::model_supports_reasoning(model, false)
                    {
                        req["reasoning"] = serde_json::json!({ "effort": e });
                    }
                }
                if !instructions.is_empty() {
                    req["instructions"] = serde_json::Value::String(instructions);
                }
                tracing::debug!("Responses API tool request to {}", url);
                self.http.post(url).json(&req)
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
                let is_gemini = self.base_url.contains("googleapis.com");
                // Only include stream_options for providers known to support it.
                // Gemini's OpenAI-compatible API doesn't support stream_options.
                if !is_gemini {
                    req["stream_options"] = serde_json::json!({"include_usage": true});
                }
                // Gemini 2.5 thinking models can exhaust their output budget on
                // internal reasoning and return empty responses. Set a generous
                // max_completion_tokens so there's room for both thinking and output.
                if is_gemini && model.contains("2.5") {
                    req["max_completion_tokens"] = serde_json::json!(65536);
                }
                // Apply reasoning effort per provider
                Self::apply_reasoning_effort(&mut req, reasoning_effort, is_gemini, model);
                self.http.post(url).json(&req)
            };

        let resp = self.send_with_oauth_retry(self.with_app_header(rb, app)).await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(self.provider_error(status, text));
        }

        let byte_stream = resp
            .bytes_stream()
            .map(|item| item.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
        let reader = tokio_util::io::StreamReader::new(byte_stream);
        let lines =
            tokio_util::codec::FramedRead::new(reader, tokio_util::codec::LinesCodec::new());

        use crate::provider::models::ToolCallChunk;
        use futures_util::StreamExt;
        let is_responses_api = self.uses_responses_api();
        // Use map + flat_map so a single SSE line can yield multiple
        // StreamChunks (e.g. batched tool call deltas from Gemini/Groq).
        let token_stream = lines
            .map(move |line_result| {
                let line = match line_result {
                    Ok(l) => l,
                    Err(e) => return vec![Err(crate::provider::stream_read_error(e))],
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
fn responses_api_input_item(msg: &crate::message::ChatMessage) -> serde_json::Value {
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
    fn from_chat(msg: &crate::message::ChatMessage) -> Self {
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
    fn from_chat(msg: &crate::message::ChatMessage) -> Self {
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
struct OaiStreamOptions {
    include_usage: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wire_tool_def_non_strict_when_any_optional() {
        // Read: `path` required, `max_bytes` optional. A mixed schema must NOT
        // be strictified — strict's all-required rule would force max_bytes to
        // required-nullable, which makes reasoning models null-fill / emit
        // empty calls. Sent non-strict with the ORIGINAL schema instead.
        let canonical = json!({
            "function": {
                "name": "Read",
                "description": "Read a file",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "max_bytes": {"type": "integer"}
                    },
                    "required": ["path"]
                }
            }
        });
        let wire = wire_tool_def(&canonical).unwrap();
        assert_eq!(wire["strict"], json!(false));
        // Original schema passed through untouched.
        assert_eq!(wire["parameters"]["required"], json!(["path"]));
        assert!(wire["parameters"].get("additionalProperties").is_none());
    }

    #[test]
    fn wire_tool_def_strict_when_fully_required() {
        // Every property required → strict is faithful (strictify only adds
        // additionalProperties:false, never touches `required`).
        let canonical = json!({
            "function": {
                "name": "Echo",
                "description": "Echo text",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string"}
                    },
                    "required": ["text"]
                }
            }
        });
        let wire = wire_tool_def(&canonical).unwrap();
        assert_eq!(wire["strict"], json!(true));
        assert_eq!(wire["parameters"]["required"], json!(["text"]));
        assert_eq!(wire["parameters"]["additionalProperties"], json!(false));
    }

    #[test]
    fn wire_tool_def_non_strict_when_nested_optional() {
        // Top-level all-required, but a nested object has an optional field →
        // still non-strict, because strict would distort the nested contract.
        let canonical = json!({
            "function": {
                "name": "Nested",
                "description": "x",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "cfg": {
                            "type": "object",
                            "properties": {
                                "a": {"type": "string"},
                                "b": {"type": "string"}
                            },
                            "required": ["a"]
                        }
                    },
                    "required": ["cfg"]
                }
            }
        });
        let wire = wire_tool_def(&canonical).unwrap();
        assert_eq!(wire["strict"], json!(false));
    }

    #[test]
    fn wire_tool_def_non_strict_when_all_optional() {
        // PageUpdate-shaped: all params optional, "at least one of" semantics
        // that strict mode cannot express. Must go out non-strict so the
        // model can omit the sections it isn't changing.
        let canonical = json!({
            "function": {
                "name": "PageUpdate",
                "description": "Refresh the dashboard",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "body": {"type": "object"},
                        "top_bar": {"type": "object"},
                        "footer": {"type": "object"},
                        "body_patch": {"type": "array", "items": {"type": "object"}}
                    },
                    "required": []
                }
            }
        });
        let wire = wire_tool_def(&canonical).unwrap();
        assert_eq!(wire["strict"], json!(false));
        // Original schema is passed through untouched — required stays empty,
        // optionals are NOT widened to nullable, no additionalProperties:false.
        assert_eq!(wire["parameters"]["required"], json!([]));
        assert!(wire["parameters"].get("additionalProperties").is_none());
        assert_eq!(wire["parameters"]["properties"]["body"]["type"], json!("object"));
    }
}
