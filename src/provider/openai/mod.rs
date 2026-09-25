//! The OpenAI-compatible client: one HTTP client, two wires. A ChatGPT-OAuth
//! client speaks the Responses API ([`responses_api`]); every other one speaks
//! Chat Completions ([`chat_completions`]). Wire shapes live in [`wire`].

mod chat_completions;
mod responses_api;
mod strict_schema;
mod wire;

use wire::extract_error_message;
pub use wire::wire_tool_def;

use crate::provider::models::StreamChunk;
use anyhow::Result;
use futures_util::{Stream, StreamExt};
use reqwest::Client;

/// Reads one SSE `data:` payload of a turn into the chunks it carries. It may
/// keep state across the stream (the Responses API tracks output items).
type Parser = Box<dyn FnMut(&str) -> Vec<Result<StreamChunk>> + Send>;

/// A response's SSE lines read into chunks — one line can yield several
/// (batched tool call deltas from Gemini/Groq, a new item's paragraph break).
fn parse_sse(
    resp: reqwest::Response,
    mut parse: Parser,
) -> impl Stream<Item = Result<StreamChunk>> + Send {
    sse_lines(resp)
        .map(move |line_result| match line_result {
            Ok(line) => sse_data(&line).map(&mut parse).unwrap_or_default(),
            Err(e) => vec![Err(crate::provider::stream_read_error(e))],
        })
        .flat_map(futures_util::stream::iter)
}

/// The `data:` payload of one SSE line, or `None` for blank lines, `event:`
/// lines, non-data lines and the `[DONE]` sentinel.
fn sse_data(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with("event:") {
        return None;
    }
    let data = trimmed.strip_prefix("data: ")?.trim();
    (data != "[DONE]").then_some(data)
}

/// The SSE lines of a streamed response body.
fn sse_lines(
    resp: reqwest::Response,
) -> impl Stream<Item = Result<String, tokio_util::codec::LinesCodecError>> + Send {
    let byte_stream = resp
        .bytes_stream()
        .map(|item| item.map_err(std::io::Error::other));
    let reader = tokio_util::io::StreamReader::new(byte_stream);
    tokio_util::codec::FramedRead::new(reader, tokio_util::codec::LinesCodec::new())
}

#[derive(Clone)]
pub struct OpenAiClient {
    pub(super) http: Client,
    pub(super) base_url: String,
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
            let tokens = crate::provider::codex_auth::CodexAuthTokens::load(
                &crate::provider::codex_auth::codex_auth_file(),
            );
            if let Some(ref token) = tokens.access_token {
                rb = rb.header("Authorization", format!("Bearer {}", token));
            }
            if let Some(ref account_id) = tokens.account_id {
                rb = rb.header("ChatGPT-Account-Id", account_id);
            }
            // The Codex backend routes model slugs by originator+version;
            // with no originator, gpt-5.6-luna resolved to a missing
            // internal engine and 404'd ("Model not found"), and a version
            // older than a model's launch refuses it ("not supported when
            // using Codex with a ChatGPT account" — gpt-6-* under 0.144.1).
            // Identify as a current Codex CLI (openai/codex#31967); bump
            // the version with each built-in generation.
            rb = rb.header("originator", "codex_cli_rs");
            rb = rb.header("version", "0.155.1");
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
    fn with_app_header(
        &self,
        rb: reqwest::RequestBuilder,
        app: Option<&str>,
    ) -> reqwest::RequestBuilder {
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
        use crate::provider::error::{ProviderError, ProviderErrorKind};
        if self.codex_auth_live && status == reqwest::StatusCode::UNAUTHORIZED {
            return ProviderError::new(
                ProviderErrorKind::Auth,
                "AUTH_REQUIRED: ChatGPT session expired. Sign in with ChatGPT to continue."
                    .to_string(),
            )
            .into();
        }
        if self.linggen_account_live && status == reqwest::StatusCode::UNAUTHORIZED {
            return ProviderError::new(
                ProviderErrorKind::Auth,
                "AUTH_REQUIRED: linggen.dev sign-in missing or expired. Run `ling account login`."
                    .to_string(),
            )
            .into();
        }
        if status == reqwest::StatusCode::PAYMENT_REQUIRED {
            if let Some(msg) = extract_error_message(&text) {
                let msg = if self.linggen_account_live {
                    format!("BILLING_REQUIRED: {msg}")
                } else {
                    msg
                };
                return ProviderError::new(ProviderErrorKind::QuotaExhausted, msg).into();
            }
        }
        let truncated = if text.len() > 500 {
            format!(
                "{}… ({} chars)",
                &text[..text.floor_char_boundary(500)],
                text.len()
            )
        } else {
            text.clone()
        };
        ProviderError::http(
            status,
            &text,
            format!("openai error ({}): {}", status, truncated),
        )
        .into()
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
        let first = self.apply_auth(
            rb.try_clone()
                .ok_or_else(|| anyhow::anyhow!("request body not cloneable for auth retry"))?,
        );
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

    /// Check if a model supports reasoning effort control.
    pub(super) fn model_supports_reasoning(model: &str, is_gemini: bool) -> bool {
        let m = model.to_lowercase();
        // OpenAI reasoning models
        if m.contains("gpt-5")
            || m.contains("gpt-6")
            || m.contains("o3")
            || m.contains("o4")
            || m.contains("o1")
        {
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

    /// Apply reasoning effort to a request based on provider.
    /// - OpenAI (GPT-5, o3, o4-mini): `reasoning_effort` field
    /// - Gemini 2.5: `generationConfig.thinkingConfig.thinkingBudget`
    /// - Others: no-op (unknown params are silently ignored by most providers)
    pub(super) fn apply_reasoning_effort(
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

    /// POST a turn's request and hand back the response, or the provider's
    /// error if it refused.
    async fn send_turn(
        &self,
        rb: reqwest::RequestBuilder,
        app: Option<&str>,
    ) -> Result<reqwest::Response> {
        let resp = self
            .send_with_oauth_retry(self.with_app_header(rb, app))
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(self.provider_error(status, text));
        }
        Ok(resp)
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

        let (rb, parse): (_, Parser) = if self.uses_responses_api() {
            let mut stream = responses_api::ResponsesStream::default();
            (
                responses_api::text_request(self, model, messages),
                Box::new(move |data| stream.text_event(data)),
            )
        } else {
            (
                chat_completions::text_request(self, model, messages, reasoning_effort),
                Box::new(|data| {
                    chat_completions::parse_text_chunk(data)
                        .into_iter()
                        .collect()
                }),
            )
        };
        let resp = self.send_turn(rb, app).await?;
        Ok(parse_sse(resp, parse))
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

        let (rb, parse): (_, Parser) = if self.uses_responses_api() {
            let mut stream = responses_api::ResponsesStream::default();
            (
                responses_api::tool_request(self, model, messages, &tools, reasoning_effort),
                Box::new(move |data| stream.tool_event(data)),
            )
        } else {
            (
                chat_completions::tool_request(self, model, messages, tools, reasoning_effort),
                Box::new(chat_completions::parse_tool_chunk),
            )
        };
        let resp = self.send_turn(rb, app).await?;
        Ok(parse_sse(resp, parse))
    }
}
