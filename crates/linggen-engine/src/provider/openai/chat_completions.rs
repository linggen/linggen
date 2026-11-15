//! Chat Completions (`/chat/completions`) — the OpenAI-compatible wire every
//! non-ChatGPT client speaks (OpenAI API keys, Gemini, DeepSeek, Groq, the
//! Linggen Cloud proxy…): the request body and the SSE chunks it streams.

use super::wire::{sanitize_json, OaiMessage, OaiMessageWithTools, OaiStreamChunk, OaiUsage};
use super::OpenAiClient;
use crate::message::ChatMessage;
use crate::provider::models::{StreamChunk, TokenUsage, ToolCallChunk};
use anyhow::Result;
use serde_json::Value;

/// Gemini's OpenAI-compatible endpoint, which needs a few extra knobs.
fn is_gemini(client: &OpenAiClient) -> bool {
    client.base_url.contains("googleapis.com")
}

/// A text-only turn (no tools).
pub(super) fn text_request(
    client: &OpenAiClient,
    model: &str,
    messages: &[ChatMessage],
    reasoning_effort: Option<&str>,
) -> reqwest::RequestBuilder {
    let url = format!("{}/chat/completions", client.base_url);
    let oai_messages: Vec<OaiMessage> = messages.iter().map(OaiMessage::from_chat).collect();
    let is_gemini = is_gemini(client);
    let mut req = serde_json::json!({
        "model": model,
        "messages": oai_messages,
        "stream": true,
        // Usage in the final chunk — every provider's meter needs it.
        "stream_options": { "include_usage": true },
    });
    // Gemini 2.5 thinking models can exhaust their output budget on
    // internal reasoning and return empty responses.
    if is_gemini && model.contains("2.5") {
        req["max_completion_tokens"] = serde_json::json!(65536);
    }
    // Apply reasoning effort per provider
    OpenAiClient::apply_reasoning_effort(&mut req, reasoning_effort, is_gemini, model);
    client.http.post(url).json(&req)
}

/// A turn with native tool calling.
pub(super) fn tool_request(
    client: &OpenAiClient,
    model: &str,
    messages: &[ChatMessage],
    tools: Vec<Value>,
    reasoning_effort: Option<&str>,
) -> reqwest::RequestBuilder {
    let url = format!("{}/chat/completions", client.base_url);
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
    let is_gemini = is_gemini(client);
    // Usage in the final chunk — every provider's meter needs it.
    req["stream_options"] = serde_json::json!({"include_usage": true});
    // Gemini 2.5 thinking models can exhaust their output budget on
    // internal reasoning and return empty responses. Set a generous
    // max_completion_tokens so there's room for both thinking and output.
    if is_gemini && model.contains("2.5") {
        req["max_completion_tokens"] = serde_json::json!(65536);
    }
    // Apply reasoning effort per provider
    OpenAiClient::apply_reasoning_effort(&mut req, reasoning_effort, is_gemini, model);
    client.http.post(url).json(&req)
}

fn usage_chunk(usage: OaiUsage) -> StreamChunk {
    StreamChunk::Usage(TokenUsage {
        prompt_tokens: usage.prompt_tokens.map(|v| v as usize),
        completion_tokens: usage.completion_tokens.map(|v| v as usize),
        total_tokens: usage.total_tokens.map(|v| v as usize),
        cached_tokens: usage
            .prompt_tokens_details
            .as_ref()
            .and_then(|d| d.cached_tokens)
            .map(|v| v as usize),
    })
}

/// One SSE `data:` payload of a text turn → at most one chunk.
pub(super) fn parse_text_chunk(data: &str) -> Option<Result<StreamChunk>> {
    let sanitized = sanitize_json(data);
    let chunk: OaiStreamChunk = match serde_json::from_str(&sanitized) {
        Ok(c) => c,
        Err(e) => {
            let truncated = if data.len() > 300 {
                format!(
                    "{}… ({} chars)",
                    &data[..data.floor_char_boundary(300)],
                    data.len()
                )
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

    // Content first: usage may ride on a content chunk (Gemini puts
    // it on every chunk once asked), and a dropped token is worse
    // than a dropped count. The final chunk carries no content and
    // the last usage, and that is the one taken.
    let usage = chunk.usage;
    let content = chunk
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.delta.content)
        .unwrap_or_default();
    if !content.is_empty() {
        return Some(Ok(StreamChunk::Token(content)));
    }
    usage.map(|usage| Ok(usage_chunk(usage)))
}

/// One SSE `data:` payload of a tool turn → the chunks it carries.
pub(super) fn parse_tool_chunk(data: &str) -> Vec<Result<StreamChunk>> {
    let sanitized = sanitize_json(data);
    let mut chunk: OaiStreamChunk = match serde_json::from_str(&sanitized) {
        Ok(c) => c,
        Err(e) => {
            return vec![Err(anyhow::anyhow!(
                "openai json parse error: {} (data: {})",
                e,
                data
            ))];
        }
    };

    // Usage may ride on a chunk that also carries a tool call or
    // content — Gemini puts it on every chunk once asked for it.
    // Emit it beside the deltas, never instead of them (seen
    // live: three "empty responses" while the meter counted).
    let usage = chunk.usage.take().map(|u| Ok(usage_chunk(u)));
    let deltas: Vec<Result<StreamChunk>> = (|| {
        // Extract Gemini thought_signature — check chunk level first, then choice level.
        let chunk_level_sig = chunk
            .extra_content
            .as_ref()
            .and_then(|ec| ec.google.as_ref())
            .and_then(|g| g.thought_signature.clone());

        let Some(choice) = chunk.choices.into_iter().next() else {
            tracing::trace!("SSE chunk with no choices: {}", &sanitized);
            return vec![];
        };

        let thought_sig = chunk_level_sig.or_else(|| {
            choice
                .extra_content
                .as_ref()
                .and_then(|ec| ec.google.as_ref())
                .and_then(|g| g.thought_signature.clone())
        });

        // Emit ALL tool call deltas from this chunk (not just the first)
        if let Some(tool_calls) = choice.delta.tool_calls {
            let mut chunks: Vec<Result<StreamChunk>> = Vec::new();
            for tc in tool_calls.into_iter() {
                let name = tc.function.as_ref().and_then(|f| f.name.clone());
                let args_delta = tc.function.as_ref().and_then(|f| f.arguments.clone());
                // Extract thought_signature from the tool call's own extra_content
                // (Gemini puts it here), falling back to choice/chunk level.
                let sig = tc
                    .extra_content
                    .as_ref()
                    .and_then(|ec| ec.google.as_ref())
                    .and_then(|g| g.thought_signature.clone())
                    .or_else(|| thought_sig.clone());
                if sig.is_some() && name.is_some() {
                    tracing::debug!(
                        "Gemini thought_signature captured for tool call '{}'",
                        name.as_deref().unwrap_or("?")
                    );
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
    })();
    usage.into_iter().chain(deltas).collect()
}
