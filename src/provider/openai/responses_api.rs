//! The ChatGPT Responses API (`/responses`) — what a ChatGPT-OAuth client
//! speaks: the request body, the input items, and the SSE events it streams.

use super::wire::{prompt_cache_key, wire_tool_def};
use super::OpenAiClient;
use crate::message::ChatMessage;
use crate::provider::models::{StreamChunk, TokenUsage, ToolCallChunk};
use anyhow::Result;
use serde_json::Value;

/// ChatGPT Responses API requires tool call IDs starting with 'fc'.
/// Sanitize IDs that may come from other providers or legacy sessions.
fn ensure_fc_prefix(id: &str) -> String {
    if id.starts_with("fc") {
        id.to_string()
    } else {
        format!("fc_{id}")
    }
}

/// A text-only turn (no tools).
pub(super) fn text_request(
    client: &OpenAiClient,
    model: &str,
    messages: &[ChatMessage],
) -> reqwest::RequestBuilder {
    let url = format!("{}/responses", client.base_url);
    // Separate system instructions from input messages
    let (instructions, conversation) = responses_instructions(messages);
    let input_items: Vec<Value> = conversation.iter().map(responses_api_input_item).collect();
    let mut req = serde_json::json!({
        "model": model,
        "input": input_items,
        "stream": true,
        "store": false,
    });
    let cache_key = prompt_cache_key(&instructions);
    req["prompt_cache_key"] = Value::String(cache_key.clone());
    if !instructions.is_empty() {
        req["instructions"] = Value::String(instructions);
    }
    tracing::debug!("Responses API request to {}", url);
    client
        .http
        .post(url)
        .header("session_id", cache_key)
        .json(&req)
}

/// A turn with native tool calling.
pub(super) fn tool_request(
    client: &OpenAiClient,
    model: &str,
    messages: &[ChatMessage],
    tools: &[Value],
    reasoning_effort: Option<&str>,
) -> reqwest::RequestBuilder {
    let url = format!("{}/responses", client.base_url);
    let (instructions, conversation) = responses_instructions(messages);
    let mut input_items: Vec<Value> = Vec::new();
    for msg in conversation {
        if msg.role == "tool" {
            // Tool result messages → function_call_output items
            let call_id = msg
                .tool_call_id
                .as_deref()
                .map(ensure_fc_prefix)
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
                        Value::String(s) => s.clone(),
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
    let resp_tools: Vec<Value> = tools.iter().filter_map(wire_tool_def).collect();

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
            && OpenAiClient::model_supports_reasoning(model, false)
        {
            req["reasoning"] = serde_json::json!({ "effort": e });
        }
    }
    let cache_key = prompt_cache_key(&instructions);
    req["prompt_cache_key"] = Value::String(cache_key.clone());
    if !instructions.is_empty() {
        req["instructions"] = Value::String(instructions);
    }
    tracing::debug!("Responses API tool request to {}", url);
    client
        .http
        .post(url)
        .header("session_id", cache_key)
        .json(&req)
}

/// The usage a `response.completed` event carries, if any.
fn completed_usage(val: &Value) -> Option<StreamChunk> {
    let usage = val.get("response").and_then(|r| r.get("usage"))?;
    let input = usage
        .get("input_tokens")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    let output = usage
        .get("output_tokens")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    Some(StreamChunk::Usage(TokenUsage {
        prompt_tokens: input,
        completion_tokens: output,
        total_tokens: input.zip(output).map(|(a, b)| a + b),
        cached_tokens: usage
            .get("input_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(|v| v.as_u64())
            .map(|v| v as usize),
    }))
}

/// An in-stream `error` event.
fn error_event(val: &Value) -> anyhow::Error {
    let msg = val
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown error");
    anyhow::anyhow!("Responses API error: {}", msg)
}

/// One SSE `data:` payload of a text turn → at most one chunk.
pub(super) fn parse_text_event(data: &str) -> Option<Result<StreamChunk>> {
    // Parse generic JSON, look for delta text or usage; skip unparseable events.
    let val: Value = serde_json::from_str(data).ok()?;
    let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match event_type {
        "response.output_text.delta" => {
            let delta = val.get("delta").and_then(|v| v.as_str()).unwrap_or("");
            (!delta.is_empty()).then(|| Ok(StreamChunk::Token(delta.to_string())))
        }
        "response.completed" => completed_usage(&val).map(Ok),
        "error" => Some(Err(error_event(&val))),
        _ => None, // skip other event types
    }
}

/// One SSE `data:` payload of a tool turn → the chunks it carries.
pub(super) fn parse_tool_event(data: &str) -> Vec<Result<StreamChunk>> {
    let Ok(val) = serde_json::from_str::<Value>(data) else {
        return vec![];
    };
    let str_at = |v: &Value, key: &str| v.get(key).and_then(|v| v.as_str()).map(String::from);
    let output_index = val
        .get("output_index")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let tool_call = |id: Option<String>, name: Option<String>, arguments_delta: Option<String>| {
        vec![Ok(StreamChunk::ToolCall(ToolCallChunk {
            index: output_index,
            id,
            name,
            arguments_delta,
            thought_signature: None,
        }))]
    };
    let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match event_type {
        "response.output_text.delta" => {
            let delta = val.get("delta").and_then(|v| v.as_str()).unwrap_or("");
            if delta.is_empty() {
                vec![]
            } else {
                vec![Ok(StreamChunk::Token(delta.to_string()))]
            }
        }
        "response.function_call_arguments.delta" => tool_call(
            str_at(&val, "item_id"),
            None,
            Some(str_at(&val, "delta").unwrap_or_default()),
        ),
        // Emit name/id only — deltas already accumulated the full args.
        "response.function_call_arguments.done" => {
            tool_call(str_at(&val, "call_id"), str_at(&val, "name"), None)
        }
        "response.output_item.added" => match val.get("item") {
            Some(item) if item.get("type").and_then(|v| v.as_str()) == Some("function_call") => {
                tool_call(str_at(item, "call_id"), str_at(item, "name"), None)
            }
            _ => vec![],
        },
        "response.completed" => completed_usage(&val).map(Ok).into_iter().collect(),
        "error" => vec![Err(error_event(&val))],
        _ => vec![],
    }
}

/// Build a Responses API input item from a ChatMessage, including images if present.
/// Responses API uses `input_image` content parts (not `image_url` like Chat Completions).
/// The Responses API's `instructions`: the system messages the request opens
/// with — nothing later. A system message inside the conversation (a turn's
/// memory recall, a reminder) stays where it is, as a `developer` item, so
/// the request's beginning is the same every turn and the prompt cache holds.
/// Merging it into `instructions` put a new top on every turn: 0 cached.
pub(super) fn responses_instructions(
    messages: &[crate::message::ChatMessage],
) -> (String, &[crate::message::ChatMessage]) {
    let start = messages
        .iter()
        .position(|m| m.role != "system")
        .unwrap_or(messages.len());
    let texts: Vec<&str> = messages[..start]
        .iter()
        .map(|m| m.content.as_str())
        .collect();
    (texts.join("\n"), &messages[start..])
}

/// A Responses API role: a system message inside the conversation is a
/// developer message there.
pub(super) fn responses_role(role: &str) -> &str {
    if role == "system" {
        "developer"
    } else {
        role
    }
}

pub(super) fn responses_api_input_item(msg: &crate::message::ChatMessage) -> serde_json::Value {
    if msg.images.is_empty() {
        serde_json::json!({
            "role": responses_role(&msg.role),
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
            "role": responses_role(&msg.role),
            "content": content_parts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_the_opening_system_messages_are_instructions() {
        use crate::message::ChatMessage;
        let turn = |recall: &str| {
            vec![
                ChatMessage::new("system", "You are Ling."),
                ChatMessage::new("user", "hi"),
                ChatMessage::new("assistant", "hello"),
                ChatMessage::new("system", recall),
                ChatMessage::new("user", "is NVDA too big?"),
            ]
        };
        let (first, second) = (
            turn("From memory: holds NVDA"),
            turn("From memory: something else"),
        );
        let (a, rest) = responses_instructions(&first);
        let (b, _) = responses_instructions(&second);
        assert_eq!(a, "You are Ling.");
        assert_eq!(a, b, "a turn's recall never changes the instructions");
        let items: Vec<_> = rest.iter().map(responses_api_input_item).collect();
        assert_eq!(
            items[2],
            json!({"role": "developer", "content": "From memory: holds NVDA"})
        );
        assert_eq!(
            items.len(),
            4,
            "the recall stays in place, between the turns"
        );
    }
}
