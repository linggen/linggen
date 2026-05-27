//! Anthropic Messages API client for Linggen.
//!
//! Streams text + tool-use against `https://api.anthropic.com/v1/messages`
//! using either an API key (`x-api-key` header) or a Claude Code OAuth
//! Bearer token (`Authorization: Bearer sk-ant-oat01-...`).
//!
//! Design parallels `openai.rs`:
//! - `AnthropicClient` is the stateless HTTP client.
//! - `chat_text_stream` / `chat_tool_stream` return an SSE event stream
//!   flattened into `StreamChunk` (Token / ToolCall / Usage).
//!
//! Message translation notes:
//! - Linggen's `ChatMessage` list may begin with one or more `role=system`
//!   messages. Anthropic takes a single top-level `system` string, so we
//!   concatenate them.
//! - Assistant messages with `tool_calls` become `assistant` messages whose
//!   `content` is a content-block array (text, then one `tool_use` block
//!   per call).
//! - Tool-result messages (`role=tool`) become `user` messages carrying a
//!   single `tool_result` content block, keyed by `tool_use_id`. Anthropic
//!   requires tool results in `user`, not a dedicated `tool` role.
//!
//! SSE event handling:
//! - `content_block_start` with `type=tool_use` → `ToolCallChunk { id, name }`.
//! - `content_block_delta` with `type=text_delta` → `StreamChunk::Token`.
//! - `content_block_delta` with `type=input_json_delta` → `ToolCallChunk
//!   { arguments_delta }`, keyed by the same `chunk_index` as the opener.
//! - `message_start` / `message_delta` usage → `StreamChunk::Usage`.

use crate::provider::models::{StreamChunk, ToolCallChunk, TokenUsage};
use crate::provider::claude_auth::{self, ANTHROPIC_OAUTH_BETA, ANTHROPIC_VERSION};
use crate::message::ChatMessage;
use anyhow::{Context, Result};
use futures_util::{Stream, StreamExt};
use reqwest::Client;
use std::collections::HashMap;

/// Default `max_tokens` when none is configured per-model. Anthropic requires
/// this field; 8192 matches the Claude 4.x SDK default.
const DEFAULT_MAX_TOKENS: u32 = 8192;

/// Auth mode for an `AnthropicClient`.
#[derive(Debug, Clone)]
enum AuthMode {
    /// Direct `x-api-key` — bring-your-own Anthropic API key.
    ApiKey(String),
    /// OAuth bearer — reuses Claude Code (CC Max) credentials. Token is
    /// re-read from the OS store on every request so refreshes by the
    /// `claude` CLI are picked up without restarting Linggen.
    ClaudeOAuth,
    /// No credentials — every call fails with a clear error rather than
    /// panicking. Exists so misconfigured models are obvious.
    Unset,
}

/// Convert a canonical tool def (`{type:"function", function:{name, description, parameters}}`)
/// into Anthropic's shape: `{ name, description, input_schema }`. The
/// parameter JSON Schema is passed through as-is — Claude reads JSON
/// Schema natively and respects optional-field semantics (no need for
/// the OpenAI-style strict-mode rewrite).
///
/// Single source of truth for the Anthropic wire shape; called from
/// both the live request path and the system-prompt export so users
/// inspecting the export see exactly what Claude receives.
pub fn wire_tool_def(canonical: &serde_json::Value) -> Option<serde_json::Value> {
    let func = canonical.get("function")?;
    Some(serde_json::json!({
        "name": func.get("name")?.clone(),
        "description": func.get("description").cloned().unwrap_or(serde_json::Value::Null),
        "input_schema": func
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({ "type": "object" })),
    }))
}

#[derive(Clone)]
pub struct AnthropicClient {
    http: Client,
    base_url: String,
    auth: AuthMode,
}

impl AnthropicClient {
    /// Client using a static API key (`sk-ant-api03-...`).
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        let http = build_http();
        let auth = match api_key {
            Some(k) if !k.is_empty() => AuthMode::ApiKey(k),
            _ => AuthMode::Unset,
        };
        Self {
            http,
            base_url: normalize_base(&base_url),
            auth,
        }
    }

    /// Client that reads CC Max OAuth tokens from the keychain on every
    /// request. Required for CC Max subscriptions where the user
    /// authenticates via the `claude` CLI.
    pub fn new_claude_oauth(base_url: String) -> Self {
        Self {
            http: build_http(),
            base_url: normalize_base(&base_url),
            auth: AuthMode::ClaudeOAuth,
        }
    }

    /// Attach auth + version headers.
    fn apply_auth(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        let rb = rb
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json");
        match &self.auth {
            AuthMode::ApiKey(key) => Ok(rb.header("x-api-key", key.as_str())),
            AuthMode::ClaudeOAuth => {
                let tokens = claude_auth::load().context(
                    "Anthropic model configured for Claude Code OAuth, but no valid tokens \
                     were found. Sign in with `claude` first.",
                )?;
                if !tokens.can_do_inference() {
                    anyhow::bail!(
                        "Claude Code OAuth token is missing the `user:inference` scope. \
                         Sign in with `claude` again to request inference access."
                    );
                }
                if tokens.is_expired() {
                    anyhow::bail!(
                        "Claude Code OAuth token is expired. Run `claude` once to refresh."
                    );
                }
                Ok(rb
                    .header(
                        "Authorization",
                        format!("Bearer {}", tokens.access_token),
                    )
                    .header("anthropic-beta", ANTHROPIC_OAUTH_BETA))
            }
            AuthMode::Unset => anyhow::bail!(
                "Anthropic client has no credentials. Set `api_key` or `auth_mode = \"claude_oauth\"` in the model config."
            ),
        }
    }

    // -----------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------

    pub async fn chat_text_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
    ) -> Result<impl Stream<Item = Result<StreamChunk>> + Send> {
        self.stream_messages(model, messages, vec![]).await
    }

    pub async fn chat_tool_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: Vec<serde_json::Value>,
    ) -> Result<impl Stream<Item = Result<StreamChunk>> + Send> {
        let anthro_tools: Vec<serde_json::Value> = tools
            .iter()
            .filter_map(wire_tool_def)
            .collect();
        self.stream_messages(model, messages, anthro_tools).await
    }

    /// Anthropic doesn't expose per-model `/models/{id}` context windows, so
    /// we let the agent fall back to its guessed default.
    pub async fn get_context_window(&self, _model: &str) -> Option<usize> {
        None
    }

    // -----------------------------------------------------------------
    // Shared streaming implementation
    // -----------------------------------------------------------------

    async fn stream_messages(
        &self,
        model: &str,
        messages: &[ChatMessage],
        anthro_tools: Vec<serde_json::Value>,
    ) -> Result<impl Stream<Item = Result<StreamChunk>> + Send> {
        let url = format!("{}/v1/messages", self.base_url);
        let translated = translate_messages(messages);

        let mut req = serde_json::json!({
            "model": model,
            "max_tokens": DEFAULT_MAX_TOKENS,
            "messages": translated.messages,
            "stream": true,
        });
        if let Some(system) = translated.system {
            req["system"] = system;
        }
        if !anthro_tools.is_empty() {
            req["tools"] = serde_json::Value::Array(anthro_tools);
        }

        tracing::info!(
            "Anthropic stream: model={} msgs={} tools={} chars={}",
            model,
            messages.len(),
            req.get("tools").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0),
            messages.iter().map(|m| m.content.len()).sum::<usize>(),
        );

        let rb = self.apply_auth(self.http.post(&url).json(&req))?;
        let resp = rb.send().await.context("Anthropic request failed")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("anthropic error ({}): {}", status, body);
        }

        // Single unfold owns: (pinned byte stream, rolling byte buffer,
        // per-turn block-index→tool state). On each poll it either parses
        // a buffered SSE event or pulls more bytes. The buffer is bytes
        // (not String) so that multi-byte UTF-8 codepoints split across
        // TCP/gzip chunk boundaries don't get replaced with U+FFFD mid-
        // token. We decode each complete event payload as UTF-8 only once
        // the `\n\n` terminator arrives, when the codepoint boundary is
        // guaranteed to be aligned.
        let byte_stream = Box::pin(resp.bytes_stream());
        let buf: Vec<u8> = Vec::new();
        let state = BlockState::default();

        let stream = futures_util::stream::unfold(
            (byte_stream, buf, state),
            |(mut byte_stream, mut buf, mut state)| async move {
                loop {
                    // Try to emit from already-buffered bytes first.
                    if let Some(item) = try_next_event(&mut buf, &mut state) {
                        return Some((item, (byte_stream, buf, state)));
                    }
                    // Otherwise pull more bytes from the wire.
                    match byte_stream.next().await {
                        Some(Ok(chunk)) => {
                            buf.extend_from_slice(&chunk);
                        }
                        Some(Err(e)) => {
                            return Some((
                                Err(anyhow::anyhow!("anthropic stream error: {}", e)),
                                (byte_stream, buf, state),
                            ));
                        }
                        None => return None,
                    }
                }
            },
        );

        Ok(stream)
    }
}

/// Find the index of the next `\n\n` SSE event terminator in a byte buffer.
fn find_event_terminator(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n")
}

/// Pull the next SSE event from `buf` and convert it to a `StreamChunk`.
/// Returns `None` if `buf` doesn't yet contain a complete `\n\n`-terminated
/// event OR if the event doesn't map to a user-visible chunk (e.g. `ping`,
/// `content_block_stop`). The caller is expected to loop until either it
/// gets an item or the byte stream is drained.
fn try_next_event(
    buf: &mut Vec<u8>,
    state: &mut BlockState,
) -> Option<Result<StreamChunk>> {
    loop {
        let idx = find_event_terminator(buf)?;
        // Whole events are always UTF-8-clean (codepoint boundaries align
        // with event boundaries). If decode fails anyway, skip the block
        // rather than corrupt the stream.
        let raw = match std::str::from_utf8(&buf[..idx]) {
            Ok(s) => s.to_string(),
            Err(_) => {
                buf.drain(..idx + 2);
                continue;
            }
        };
        buf.drain(..idx + 2);
        let Some(ev) = parse_sse_block(&raw) else {
            continue;
        };
        if let Some(result) = handle_event(ev, state) {
            return Some(result);
        }
        // Event parsed but produced no chunk (e.g. ping). Keep looping.
    }
}

// ---------------------------------------------------------------------------
// Message translation: Linggen ChatMessage[] → Anthropic (system, messages[])
// ---------------------------------------------------------------------------

/// Output of `translate_messages`: the top-level `system` field (either a
/// bare string OR a content-block array, depending on whether any system
/// message carried `cache_control`) plus the conversation messages.
struct Translated {
    /// `None` when there were no system messages. Otherwise either a
    /// JSON string (no caching) or a content-block array (at least one
    /// system message had `cache_control`).
    system: Option<serde_json::Value>,
    messages: Vec<serde_json::Value>,
}

/// Build an Anthropic text content block, attaching `cache_control` when
/// the caller asked for it. Anthropic supports caching on `text`, `image`,
/// `tool_use`, and `tool_result` blocks — keeping this helper small lets
/// us reuse it across them.
fn text_block_with_cache(text: &str, cache_control: Option<&serde_json::Value>) -> serde_json::Value {
    let mut obj = serde_json::json!({ "type": "text", "text": text });
    if let Some(cc) = cache_control {
        obj["cache_control"] = cc.clone();
    }
    obj
}

fn translate_messages(messages: &[ChatMessage]) -> Translated {
    // System collection: keep per-part cache_control so a single cache
    // breakpoint on (e.g.) the tail of a big preamble is preserved.
    let mut system_parts: Vec<(String, Option<serde_json::Value>)> = Vec::new();
    let mut out: Vec<serde_json::Value> = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                if !msg.content.is_empty() {
                    system_parts.push((msg.content.clone(), msg.cache_control.clone()));
                }
            }
            "tool" => {
                let tool_use_id = msg
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| "toolu_unknown".to_string());
                let mut tr = serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": msg.content,
                });
                if let Some(cc) = &msg.cache_control {
                    tr["cache_control"] = cc.clone();
                }
                out.push(serde_json::json!({
                    "role": "user",
                    "content": [tr],
                }));
            }
            "assistant" if !msg.tool_calls.is_empty() => {
                let mut content: Vec<serde_json::Value> = Vec::new();
                if !msg.content.is_empty() {
                    content.push(text_block_with_cache(&msg.content, None));
                }
                let last_idx = msg.tool_calls.len().saturating_sub(1);
                for (i, tc) in msg.tool_calls.iter().enumerate() {
                    // Arguments may be a JSON string (OpenAI-style accumulator
                    // output) or an already-parsed object. Accept both.
                    let input = match &tc.function.arguments {
                        serde_json::Value::String(s) => serde_json::from_str::<serde_json::Value>(s)
                            .unwrap_or_else(|_| serde_json::json!({})),
                        other => other.clone(),
                    };
                    let mut tu = serde_json::json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.function.name,
                        "input": input,
                    });
                    // Cache-control at the message level is attached to the
                    // last content block in the message, which is Anthropic's
                    // convention for "cache up to and including this point".
                    if i == last_idx {
                        if let Some(cc) = &msg.cache_control {
                            tu["cache_control"] = cc.clone();
                        }
                    }
                    content.push(tu);
                }
                out.push(serde_json::json!({
                    "role": "assistant",
                    "content": content,
                }));
            }
            "user" | "assistant" => {
                // Text-only messages: use the bare-string content form when
                // there's no cache_control and no images, since that's the
                // smaller/more readable payload. Otherwise promote to the
                // content-block array form.
                if msg.images.is_empty() && msg.cache_control.is_none() {
                    out.push(serde_json::json!({
                        "role": msg.role,
                        "content": msg.content,
                    }));
                } else {
                    let mut parts: Vec<serde_json::Value> = Vec::new();
                    if !msg.content.is_empty() {
                        parts.push(serde_json::json!({ "type": "text", "text": msg.content }));
                    }
                    for img in &msg.images {
                        parts.push(serde_json::json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/png",
                                "data": img,
                            },
                        }));
                    }
                    // Attach cache_control to the final block when present,
                    // matching Anthropic's "cache up through this block" rule.
                    if let Some(cc) = &msg.cache_control {
                        if let Some(last) = parts.last_mut() {
                            if let Some(obj) = last.as_object_mut() {
                                obj.insert("cache_control".to_string(), cc.clone());
                            }
                        }
                    }
                    out.push(serde_json::json!({
                        "role": msg.role,
                        "content": parts,
                    }));
                }
            }
            _ => {
                // Unknown roles (e.g. legacy "function") coerce to user so
                // they still reach context rather than being dropped.
                out.push(serde_json::json!({
                    "role": "user",
                    "content": msg.content,
                }));
            }
        }
    }

    // System assembly: if no part carries cache_control we can send a bare
    // string (simpler wire format). If any part does, we have to send the
    // content-block array form — that's the only shape Anthropic accepts
    // `cache_control` on for system.
    let system = if system_parts.is_empty() {
        None
    } else if system_parts.iter().all(|(_, cc)| cc.is_none()) {
        let joined: Vec<String> = system_parts.into_iter().map(|(t, _)| t).collect();
        Some(serde_json::Value::String(joined.join("\n\n")))
    } else {
        let blocks: Vec<serde_json::Value> = system_parts
            .iter()
            .map(|(text, cc)| text_block_with_cache(text, cc.as_ref()))
            .collect();
        Some(serde_json::Value::Array(blocks))
    };

    Translated {
        system,
        messages: out,
    }
}

// ---------------------------------------------------------------------------
// SSE parsing + event → StreamChunk
// ---------------------------------------------------------------------------

struct SseEvent {
    event: String,
    data: String,
}

fn parse_sse_block(block: &str) -> Option<SseEvent> {
    let mut event_name = String::new();
    let mut data_lines: Vec<&str> = Vec::new();
    for line in block.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("event:") {
            event_name = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start());
        }
    }
    if data_lines.is_empty() {
        return None;
    }
    Some(SseEvent {
        event: event_name,
        data: data_lines.join("\n"),
    })
}

/// Per-turn state: which content-block indices carry tool_use, and how to
/// map their server-assigned `index` field to the monotonic `chunk_index`
/// that the downstream accumulator uses to key argument deltas.
#[derive(Default)]
struct BlockState {
    tool_blocks: HashMap<u32, ToolBlock>,
}

struct ToolBlock {
    chunk_index: usize,
}

fn handle_event(event: SseEvent, state: &mut BlockState) -> Option<Result<StreamChunk>> {
    let data: serde_json::Value = serde_json::from_str(&event.data).ok()?;

    match event.event.as_str() {
        "content_block_start" => {
            let index = data.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let block = data.get("content_block")?;
            let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if block_type != "tool_use" {
                return None;
            }
            let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let name = block
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let chunk_index = state.tool_blocks.len();
            state.tool_blocks.insert(index, ToolBlock { chunk_index });
            Some(Ok(StreamChunk::ToolCall(ToolCallChunk {
                index: chunk_index,
                id: Some(id),
                name: Some(name),
                arguments_delta: None,
                thought_signature: None,
            })))
        }
        "content_block_delta" => {
            let index = data.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let delta = data.get("delta")?;
            let delta_type = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match delta_type {
                "text_delta" => {
                    let text = delta.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    if text.is_empty() {
                        None
                    } else {
                        Some(Ok(StreamChunk::Token(text.to_string())))
                    }
                }
                "input_json_delta" => {
                    let partial = delta
                        .get("partial_json")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if partial.is_empty() {
                        return None;
                    }
                    let block = state.tool_blocks.get(&index)?;
                    Some(Ok(StreamChunk::ToolCall(ToolCallChunk {
                        index: block.chunk_index,
                        id: None,
                        name: None,
                        arguments_delta: Some(partial),
                        thought_signature: None,
                    })))
                }
                _ => None,
            }
        }
        "content_block_stop" => None,
        "message_start" => {
            // `message_start` carries input_tokens only; output accumulates
            // and is reported in the final `message_delta`. Emit prompt
            // tokens exactly once here so the downstream accumulator
            // doesn't double-count if Anthropic ever echoes input_tokens
            // back in the delta frame.
            let usage = data.get("message").and_then(|m| m.get("usage"))?;
            let prompt_tokens = usage
                .get("input_tokens")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            prompt_tokens.map(|p| {
                Ok(StreamChunk::Usage(TokenUsage {
                    prompt_tokens: Some(p),
                    completion_tokens: None,
                    total_tokens: None,
                }))
            })
        }
        "message_delta" => {
            // Only surface output_tokens from the delta. Input tokens were
            // already emitted at message_start — echoing them here would
            // double-count in the agent's running total.
            let output_tokens = data
                .get("usage")
                .and_then(|u| u.get("output_tokens"))
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)?;
            Some(Ok(StreamChunk::Usage(TokenUsage {
                prompt_tokens: None,
                completion_tokens: Some(output_tokens),
                total_tokens: None,
            })))
        }
        "message_stop" | "ping" => None,
        "error" => {
            let msg = data
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("anthropic returned an error event");
            Some(Err(anyhow::anyhow!("{}", msg)))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_http() -> Client {
    Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| Client::new())
}

fn normalize_base(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    if trimmed.is_empty() {
        claude_auth::ANTHROPIC_API_BASE.to_string()
    } else {
        trimmed.to_string()
    }
}
