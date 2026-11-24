//! Chat Completions (`/chat/completions`) — the OpenAI-compatible wire every
//! non-ChatGPT client speaks (OpenAI API keys, Gemini, DeepSeek, Groq, the
//! Linggen Cloud proxy…): the request body and the SSE chunks it streams.

use super::wire::{parse_payload, OaiMessage, OaiMessageWithTools, OaiStreamChunk, OaiUsage};
use super::OpenAiClient;
use crate::message::ChatMessage;
use crate::provider::models::{StreamChunk, TokenUsage, ToolCallChunk};
use anyhow::Result;
use serde_json::Value;

/// Gemini's OpenAI-compatible endpoint, which needs a few extra knobs.
fn is_gemini(client: &OpenAiClient) -> bool {
    OpenAiClient::gemini_wire(&client.base_url)
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
        cache_write_tokens: None,
        prompt_tokens: usage.prompt_tokens.map(|v| v as usize),
        completion_tokens: usage.completion_tokens.map(|v| v as usize),
        total_tokens: usage.total_tokens.map(|v| v as usize),
        // OpenAI and Gemini's compatible endpoint say it in the prompt
        // details; DeepSeek counts hits at the top of `usage`.
        cached_tokens: usage
            .prompt_tokens_details
            .as_ref()
            .and_then(|d| d.cached_tokens)
            .or(usage.prompt_cache_hit_tokens)
            .map(|v| v as usize),
    })
}

/// One SSE `data:` payload of a text turn → its token or its usage, then
/// [`StreamChunk::Cut`] when the reply stopped at the output limit.
pub(super) fn parse_text_chunk(data: &str) -> Vec<Result<StreamChunk>> {
    let chunk: OaiStreamChunk = match parse_payload(data) {
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
            return vec![Err(anyhow::anyhow!(
                "openai json parse error: {} (data: {})",
                e,
                truncated
            ))];
        }
    };

    // Content first: usage may ride on a content chunk (Gemini puts
    // it on every chunk once asked), and a dropped token is worse
    // than a dropped count. The final chunk carries no content and
    // the last usage, and that is the one taken.
    let usage = chunk.usage;
    let choice = chunk.choices.into_iter().next();
    let cut = choice.as_ref().is_some_and(|c| c.was_cut());
    let content = choice.and_then(|c| c.delta.content).unwrap_or_default();
    let said = if !content.is_empty() {
        Some(StreamChunk::Token(content))
    } else {
        usage.map(usage_chunk)
    };
    said.into_iter()
        .chain(cut.then_some(StreamChunk::Cut))
        .map(Ok)
        .collect()
}

/// One SSE `data:` payload of a tool turn → the chunks it carries.
pub(super) fn parse_tool_chunk(data: &str) -> Vec<Result<StreamChunk>> {
    let mut chunk: OaiStreamChunk = match parse_payload(data) {
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
    // The cut, when there is one, comes after the deltas its chunk carries.
    let cut = chunk.choices.first().is_some_and(|c| c.was_cut());
    let cut = cut.then_some(Ok(StreamChunk::Cut));
    let deltas: Vec<Result<StreamChunk>> = (|| {
        // Extract Gemini thought_signature — check chunk level first, then choice level.
        let chunk_level_sig = chunk
            .extra_content
            .as_ref()
            .and_then(|ec| ec.google.as_ref())
            .and_then(|g| g.thought_signature.clone());

        let Some(choice) = chunk.choices.into_iter().next() else {
            tracing::trace!("SSE chunk with no choices: {}", data);
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
    usage.into_iter().chain(deltas).chain(cut).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_args(chunks: Vec<Result<StreamChunk>>) -> String {
        chunks
            .into_iter()
            .filter_map(|c| match c.expect("parses") {
                StreamChunk::ToolCall(tc) => tc.arguments_delta,
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_numpy_slice_in_text_is_left_as_written() {
        let data = r#"{"choices":[{"index":0,"delta":{"content":"y = x[:, 0] and d = {a:}"}}]}"#;
        match parse_text_chunk(data).into_iter().next() {
            Some(Ok(StreamChunk::Token(t))) => assert_eq!(t, "y = x[:, 0] and d = {a:}"),
            other => panic!("expected a token, got {other:?}"),
        }
    }

    #[test]
    fn a_numpy_slice_in_tool_arguments_is_left_as_written() {
        let data = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"Write","arguments":"{\"content\":\"y = x[:, 0]\"}"}}]}}]}"#;
        assert_eq!(tool_args(parse_tool_chunk(data)), r#"{"content":"y = x[:, 0]"}"#);
    }

    #[test]
    fn a_missing_value_outside_strings_is_repaired_and_strings_kept() {
        // Gemini's malformed shape: `"function":,` — a key with no value.
        let data = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","extra":,"function":{"name":"Write","arguments":"{\"content\":\"x[:, 0]\"}","more":}}]}}]}"#;
        assert_eq!(tool_args(parse_tool_chunk(data)), r#"{"content":"x[:, 0]"}"#);
    }
}

#[cfg(test)]
mod cut_tests {
    use super::*;

    fn is_cut(chunks: &[Result<StreamChunk>]) -> bool {
        chunks.iter().any(|c| matches!(c, Ok(StreamChunk::Cut)))
    }

    #[test]
    fn finish_reason_length_reads_as_cut_and_keeps_the_last_delta() {
        let tool = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"a\":"}}]},"finish_reason":"length"}]}"#;
        let chunks = parse_tool_chunk(tool);
        assert!(is_cut(&chunks));
        assert!(chunks.iter().any(|c| matches!(c, Ok(StreamChunk::ToolCall(_)))));
        let text = r#"{"choices":[{"index":0,"delta":{"content":"and then"},"finish_reason":"length"}]}"#;
        let chunks: Vec<_> = parse_text_chunk(text).into_iter().collect();
        assert!(is_cut(&chunks));
        assert!(chunks.iter().any(|c| matches!(c, Ok(StreamChunk::Token(t)) if t == "and then")));
        let stop = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
        assert!(!is_cut(&parse_tool_chunk(stop)));
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn usage_of(data: &str) -> TokenUsage {
        parse_text_chunk(data)
            .into_iter()
            .find_map(|c| match c.expect("parses") {
                StreamChunk::Usage(u) => Some(u),
                _ => None,
            })
            .expect("a usage chunk")
    }

    /// OpenAI's shape — and Gemini's compatible endpoint's, which reports
    /// the same field.
    #[test]
    fn the_cached_part_is_read_from_the_prompt_details() {
        let u = usage_of(r#"{"choices":[],"usage":{"prompt_tokens":1000,"completion_tokens":50,"total_tokens":1050,"prompt_tokens_details":{"cached_tokens":800}}}"#);
        assert_eq!((u.prompt_tokens, u.cached_tokens), (Some(1000), Some(800)));
    }

    /// DeepSeek counts its cache at the top of `usage`, as hit and miss.
    #[test]
    fn deepseeks_cache_hit_tokens_are_the_cached_part() {
        let u = usage_of(r#"{"choices":[],"usage":{"prompt_tokens":1000,"completion_tokens":50,"total_tokens":1050,"prompt_cache_hit_tokens":900,"prompt_cache_miss_tokens":100}}"#);
        assert_eq!((u.prompt_tokens, u.cached_tokens), (Some(1000), Some(900)));
    }
}
