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
    crate::provider::error::ProviderError::stream_event(format!("Responses API error: {}", msg))
        .into()
}

/// The reading of one Responses API stream. A response can hold several
/// `message` output items: a reasoning model's `phase: "commentary"`
/// preamble ("let me check…") before its `phase: "final_answer"`. Only
/// non-commentary items are the reply; each later item opens its own
/// paragraph, and so does each later content part of an item. A part's
/// text is spoken once (its deltas, or — when it sent none — its
/// `output_text.done`).
#[derive(Default)]
pub(super) struct ResponsesStream {
    /// Output indexes of commentary message items.
    commentary: std::collections::HashSet<usize>,
    /// Parts — (output index, content index) — whose text already reached
    /// the reply.
    spoken: std::collections::HashSet<(usize, usize)>,
    /// The part the reply's last text came from.
    last_spoken: Option<(usize, usize)>,
    /// Drops the model's own citation markers; keeps url_citation links.
    citations: super::citations::CitationFilter,
    /// Function-call item id → its output index, for an event that names
    /// the item but not its index.
    call_items: std::collections::HashMap<String, usize>,
    /// Each function call's arguments as sent on so far, by output index.
    call_args: std::collections::HashMap<usize, String>,
}

impl ResponsesStream {
    /// One SSE `data:` payload of a text turn → the chunks it carries.
    pub(super) fn text_event(&mut self, data: &str) -> Vec<Result<StreamChunk>> {
        let Ok(val) = serde_json::from_str::<Value>(data) else {
            return vec![];
        };
        self.common_event(&val).unwrap_or_default()
    }

    /// One SSE `data:` payload of a tool turn → the chunks it carries.
    pub(super) fn tool_event(&mut self, data: &str) -> Vec<Result<StreamChunk>> {
        let Ok(val) = serde_json::from_str::<Value>(data) else {
            return vec![];
        };
        if let Some(chunks) = self.common_event(&val) {
            return chunks;
        }
        let index = self.call_index(&val);
        match event_type(&val) {
            "response.function_call_arguments.delta" => {
                let delta = str_at(&val, "delta").unwrap_or_default();
                self.call_args.entry(index).or_default().push_str(&delta);
                tool_call(index, str_at(&val, "item_id"), None, Some(delta))
            }
            // The full arguments. Parallel calls stream no deltas at all —
            // this is the only place their arguments come.
            "response.function_call_arguments.done" => {
                let rest = self.rest_of_args(index, str_at(&val, "arguments"));
                tool_call(index, str_at(&val, "call_id"), str_at(&val, "name"), rest)
            }
            "response.output_item.added" => match function_call(&val) {
                Some(item) => {
                    if let Some(id) = str_at(item, "id") {
                        self.call_items.insert(id, index);
                    }
                    tool_call(index, str_at(item, "call_id"), str_at(item, "name"), None)
                }
                None => vec![],
            },
            "response.output_item.done" => match function_call(&val) {
                Some(item) => match self.rest_of_args(index, str_at(item, "arguments")) {
                    Some(rest) => tool_call(index, None, None, Some(rest)),
                    None => vec![],
                },
                None => vec![],
            },
            _ => vec![],
        }
    }

    /// The output index a tool event is about: its own `output_index`, else
    /// the index its `item_id` was added at.
    fn call_index(&self, val: &Value) -> usize {
        if let Some(index) = val.get("output_index").and_then(|v| v.as_u64()) {
            return index as usize;
        }
        str_at(val, "item_id")
            .and_then(|id| self.call_items.get(&id).copied())
            .unwrap_or(0)
    }

    /// What a call's full `arguments` add to what its deltas already sent:
    /// all of it when none streamed, nothing when they sent it all.
    fn rest_of_args(&mut self, index: usize, full: Option<String>) -> Option<String> {
        let full = full.filter(|f| !f.is_empty())?;
        let sent = self.call_args.entry(index).or_default();
        let Some(rest) = full.strip_prefix(sent.as_str()) else {
            tracing::warn!(
                "Responses API: call {index}'s full arguments differ from its deltas; keeping the deltas"
            );
            return None;
        };
        let rest = rest.to_string();
        *sent = full;
        (!rest.is_empty()).then_some(rest)
    }

    /// The events text and tool turns read alike; `None` for any other.
    fn common_event(&mut self, val: &Value) -> Option<Vec<Result<StreamChunk>>> {
        let chunks = match event_type(val) {
            "response.output_item.added" => {
                self.note_item(val);
                return None; // a tool turn still reads function_call items
            }
            "response.output_text.delta" => self.text(val, "delta", false),
            "response.output_text.done" => self.text(val, "text", true),
            "response.output_text.annotation.added" => {
                self.citations.note_annotation(val);
                vec![]
            }
            "response.completed" => self.completed(val),
            "error" => vec![Err(error_event(val))],
            _ => return None,
        };
        Some(chunks)
    }

    /// The end of the response: the sources, if any came, then the usage.
    fn completed(&mut self, val: &Value) -> Vec<Result<StreamChunk>> {
        let sources = self.citations.finish();
        let sources = (!sources.is_empty()).then_some(StreamChunk::Token(sources));
        sources
            .into_iter()
            .chain(completed_usage(val))
            .map(Ok)
            .collect()
    }

    /// Remember a commentary message item, so its text stays out of the reply.
    fn note_item(&mut self, val: &Value) {
        let Some(item) = val.get("item") else { return };
        let is_message = item.get("type").and_then(|v| v.as_str()) == Some("message");
        let phase = item.get("phase").and_then(|v| v.as_str());
        if is_message && phase == Some("commentary") {
            self.commentary.insert(output_index(val));
        }
    }

    /// A part's text for the reply: nothing for commentary, nothing for a
    /// `done` whose part already streamed, a paragraph break before a new part.
    fn text(&mut self, val: &Value, key: &str, done: bool) -> Vec<Result<StreamChunk>> {
        let index = output_index(val);
        let part = (index, content_index(val));
        let text = val.get(key).and_then(|v| v.as_str()).unwrap_or("");
        if text.is_empty() || self.commentary.contains(&index) {
            return vec![];
        }
        if done && self.spoken.contains(&part) {
            return vec![];
        }
        let new_part = self.last_spoken.is_some_and(|last| last != part);
        let text = if new_part {
            format!("\n\n{text}")
        } else {
            text.to_string()
        };
        self.spoken.insert(part);
        self.last_spoken = Some(part);
        let text = self.citations.push(&text);
        if text.is_empty() {
            return vec![];
        }
        vec![Ok(StreamChunk::Token(text))]
    }
}

fn str_at(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|v| v.as_str()).map(String::from)
}

/// One tool-call chunk for the engine's accumulator.
fn tool_call(
    index: usize,
    id: Option<String>,
    name: Option<String>,
    arguments_delta: Option<String>,
) -> Vec<Result<StreamChunk>> {
    vec![Ok(StreamChunk::ToolCall(ToolCallChunk {
        index,
        id,
        name,
        arguments_delta,
        thought_signature: None,
    }))]
}

/// The event's `item`, when it is a function call.
fn function_call(val: &Value) -> Option<&Value> {
    val.get("item")
        .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("function_call"))
}

fn event_type(val: &Value) -> &str {
    val.get("type").and_then(|v| v.as_str()).unwrap_or("")
}

fn content_index(val: &Value) -> usize {
    val.get("content_index")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize
}

fn output_index(val: &Value) -> usize {
    val.get("output_index")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize
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
                "image_url": format!(
                            "data:{};base64,{}",
                            crate::pictures::kind::media_type_of_base64(img),
                            img
                        ),
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

    /// The reply text and tool chunks a stream of events reads into.
    fn read(events: &[Value], tool_turn: bool) -> (String, Vec<ToolCallChunk>) {
        let mut stream = ResponsesStream::default();
        let (mut text, mut calls) = (String::new(), vec![]);
        for event in events {
            let data = event.to_string();
            let chunks = if tool_turn {
                stream.tool_event(&data)
            } else {
                stream.text_event(&data)
            };
            for chunk in chunks {
                match chunk.unwrap() {
                    StreamChunk::Token(t) => text.push_str(&t),
                    StreamChunk::ToolCall(c) => calls.push(c),
                    StreamChunk::Usage(_) => {}
                }
            }
        }
        (text, calls)
    }

    fn message_added(index: u64, id: &str, phase: Option<&str>) -> Value {
        let mut item = json!({"type": "message", "id": id, "role": "assistant", "content": []});
        if let Some(phase) = phase {
            item["phase"] = json!(phase);
        }
        json!({"type": "response.output_item.added", "output_index": index, "item": item})
    }

    fn delta(index: u64, id: &str, text: &str) -> Value {
        json!({"type": "response.output_text.delta", "output_index": index,
               "item_id": id, "content_index": 0, "delta": text})
    }

    fn text_done(index: u64, id: &str, text: &str) -> Value {
        json!({"type": "response.output_text.done", "output_index": index,
               "item_id": id, "content_index": 0, "text": text})
    }

    #[test]
    fn a_commentary_preamble_stays_out_of_the_reply() {
        let events = [
            json!({"type": "response.output_item.added", "output_index": 0,
                   "item": {"type": "reasoning", "id": "rs_1"}}),
            message_added(1, "msg_a", Some("commentary")),
            delta(1, "msg_a", "我再核对一下"),
            delta(1, "msg_a", "眼下的所在与可走之路。"),
            text_done(1, "msg_a", "我再核对一下眼下的所在与可走之路。"),
            message_added(2, "msg_b", Some("final_answer")),
            delta(2, "msg_b", "你立在青石"),
            delta(2, "msg_b", "渡口。"),
            text_done(2, "msg_b", "你立在青石渡口。"),
        ];
        for tool_turn in [false, true] {
            assert_eq!(read(&events, tool_turn).0, "你立在青石渡口。");
        }
    }

    #[test]
    fn two_reply_items_are_two_paragraphs() {
        let events = [
            message_added(0, "msg_a", None),
            delta(0, "msg_a", "First."),
            message_added(1, "msg_b", None),
            delta(1, "msg_b", "Second."),
        ];
        assert_eq!(read(&events, false).0, "First.\n\nSecond.");
    }

    #[test]
    fn a_done_event_after_deltas_is_not_spoken_twice() {
        let events = [
            message_added(0, "msg_a", Some("final_answer")),
            delta(0, "msg_a", "Hello, "),
            delta(0, "msg_a", "world."),
            text_done(0, "msg_a", "Hello, world."),
            json!({"type": "response.completed",
                   "response": {"usage": {"input_tokens": 3, "output_tokens": 4}}}),
        ];
        assert_eq!(read(&events, false).0, "Hello, world.");
        // An item that streamed no deltas is spoken from its done event.
        let done_only = [
            message_added(0, "msg_a", None),
            text_done(0, "msg_a", "Whole."),
        ];
        assert_eq!(read(&done_only, false).0, "Whole.");
    }

    #[test]
    fn a_second_content_part_sent_only_as_done_is_its_own_paragraph() {
        let part_done = |content: u64, text: &str| {
            json!({"type": "response.output_text.done", "output_index": 0,
                   "item_id": "msg_a", "content_index": content, "text": text})
        };
        let events = [
            message_added(0, "msg_a", None),
            delta(0, "msg_a", "First."),
            text_done(0, "msg_a", "First."),
            part_done(1, "Second."),
        ];
        assert_eq!(read(&events, false).0, "First.\n\nSecond.");
    }

    /// gpt-6-luna's own citation markers, split across deltas as a stream
    /// sends them, reach the reply as the plain numbers alone.
    #[test]
    fn citation_markers_reach_the_reply_as_plain_numbers() {
        let events = [
            delta(0, "msg_a", "可以在 Canada 申请。\u{e200}cite\u{e202}turn2"),
            delta(0, "msg_a", "search1\u{e202}turn2search2\u{e201}"),
            delta(0, "msg_a", "下一步是填表。"),
            json!({"type": "response.completed", "response": {}}),
        ];
        assert_eq!(
            read(&events, false).0,
            "可以在 Canada 申请。[turn2search1][turn2search2]下一步是填表。"
        );
    }

    #[test]
    fn a_single_reply_reads_as_before() {
        let events = [delta(0, "msg_a", "Hi"), delta(0, "msg_a", " there")];
        assert_eq!(read(&events, false).0, "Hi there");
        assert_eq!(read(&events, true).0, "Hi there");
    }

    #[test]
    fn tool_calls_read_as_before_beside_commentary() {
        let events = [
            message_added(0, "msg_a", Some("commentary")),
            delta(0, "msg_a", "Checking the map."),
            json!({"type": "response.output_item.added", "output_index": 1,
                   "item": {"type": "function_call", "id": "fc_1", "call_id": "call_1",
                            "name": "Look", "arguments": ""}}),
            json!({"type": "response.function_call_arguments.delta", "output_index": 1,
                   "item_id": "fc_1", "delta": "{\"at\":"}),
            json!({"type": "response.function_call_arguments.delta", "output_index": 1,
                   "item_id": "fc_1", "delta": "1}"}),
            json!({"type": "response.function_call_arguments.done", "output_index": 1,
                   "item_id": "fc_1", "call_id": "call_1", "name": "Look",
                   "arguments": "{\"at\":1}"}),
        ];
        let (text, calls) = read(&events, true);
        assert_eq!(text, "");
        assert!(calls.iter().all(|c| c.index == 1));
        assert_eq!(calls[0].id.as_deref(), Some("call_1"));
        assert_eq!(calls[0].name.as_deref(), Some("Look"));
        let args: String = calls
            .iter()
            .filter_map(|c| c.arguments_delta.clone())
            .collect();
        assert_eq!(args, "{\"at\":1}");
    }

    /// Tool chunks folded by index the way the engine folds them:
    /// index → (id, name, arguments).
    fn assemble(calls: &[ToolCallChunk]) -> Vec<(usize, String, String, String)> {
        let mut by_index = std::collections::BTreeMap::<usize, (String, String, String)>::new();
        for c in calls {
            let e = by_index.entry(c.index).or_default();
            if let Some(id) = &c.id {
                e.0 = id.clone();
            }
            if let Some(name) = &c.name {
                e.1 = name.clone();
            }
            if let Some(d) = &c.arguments_delta {
                e.2.push_str(d);
            }
        }
        by_index
            .into_iter()
            .map(|(i, (id, name, args))| (i, id, name, args))
            .collect()
    }

    fn call_added(index: u64, item: &str, call: &str, name: &str) -> Value {
        json!({"type": "response.output_item.added", "output_index": index,
               "item": {"type": "function_call", "id": item, "call_id": call,
                        "name": name, "arguments": "", "status": "in_progress"}})
    }

    fn args_delta(index: u64, item: &str, delta: &str) -> Value {
        json!({"type": "response.function_call_arguments.delta", "output_index": index,
               "item_id": item, "delta": delta})
    }

    fn args_done(index: u64, item: &str, args: &str) -> Value {
        json!({"type": "response.function_call_arguments.done", "output_index": index,
               "item_id": item, "arguments": args})
    }

    fn call_done(index: u64, item: &str, call: &str, name: &str, args: &str) -> Value {
        json!({"type": "response.output_item.done", "output_index": index,
               "item": {"type": "function_call", "id": item, "call_id": call,
                        "name": name, "arguments": args, "status": "completed"}})
    }

    /// Recorded from gpt-6.1-sol on the Codex backend (2026-10-26): parallel
    /// calls stream no argument deltas — the arguments come only whole, in
    /// each call's `.done` events. Reading only deltas gave both calls `{}`.
    #[test]
    fn parallel_calls_without_deltas_keep_their_own_arguments() {
        let events = [
            call_added(0, "fc_a", "call_a", "WebSearch"),
            args_done(0, "fc_a", "{\"query\": \"pressurized portafilter basket\"}"),
            call_done(
                0,
                "fc_a",
                "call_a",
                "WebSearch",
                "{\"query\": \"pressurized portafilter basket\"}",
            ),
            call_added(1, "fc_b", "call_b", "ImageSearch"),
            args_done(1, "fc_b", "{\"query\": \"pressurized basket photo\"}"),
            call_done(
                1,
                "fc_b",
                "call_b",
                "ImageSearch",
                "{\"query\": \"pressurized basket photo\"}",
            ),
        ];
        let (_, calls) = read(&events, true);
        assert_eq!(
            assemble(&calls),
            vec![
                (
                    0,
                    "call_a".into(),
                    "WebSearch".into(),
                    "{\"query\": \"pressurized portafilter basket\"}".into()
                ),
                (
                    1,
                    "call_b".into(),
                    "ImageSearch".into(),
                    "{\"query\": \"pressurized basket photo\"}".into()
                ),
            ]
        );
    }

    /// Three calls whose deltas interleave; one names its item but not its
    /// index. Each keeps its own arguments, and the closing `.done` events
    /// add nothing the deltas already sent.
    #[test]
    fn interleaved_parallel_deltas_keep_their_own_arguments() {
        let events = [
            call_added(0, "fc_a", "call_a", "memory_search"),
            call_added(1, "fc_b", "call_b", "memory_search"),
            call_added(2, "fc_c", "call_c", "WebSearch"),
            args_delta(0, "fc_a", "{\"query\":"),
            args_delta(2, "fc_c", "{\"query\":\"c"),
            json!({"type": "response.function_call_arguments.delta",
                   "item_id": "fc_b", "delta": "{\"query\":\"b\"}"}),
            args_delta(0, "fc_a", "\"a\"}"),
            args_done(1, "fc_b", "{\"query\":\"b\"}"),
            args_done(0, "fc_a", "{\"query\":\"a\"}"),
            // `c` stopped short in its deltas: its `.done` carries the rest.
            args_done(2, "fc_c", "{\"query\":\"c\"}"),
            call_done(0, "fc_a", "call_a", "memory_search", "{\"query\":\"a\"}"),
            call_done(1, "fc_b", "call_b", "memory_search", "{\"query\":\"b\"}"),
            call_done(2, "fc_c", "call_c", "WebSearch", "{\"query\":\"c\"}"),
        ];
        let (_, calls) = read(&events, true);
        let args: Vec<String> = assemble(&calls).into_iter().map(|c| c.3).collect();
        assert_eq!(
            args,
            vec![
                "{\"query\":\"a\"}",
                "{\"query\":\"b\"}",
                "{\"query\":\"c\"}"
            ]
        );
    }

    /// A single streamed call reads exactly as before: the deltas, then the
    /// `.done` events add nothing.
    #[test]
    fn a_single_streamed_call_is_not_doubled_by_its_done_events() {
        let events = [
            call_added(0, "fc_a", "call_a", "WebSearch"),
            args_delta(0, "fc_a", "{\"query\":\"press"),
            args_delta(0, "fc_a", "urized\"}"),
            args_done(0, "fc_a", "{\"query\":\"pressurized\"}"),
            call_done(
                0,
                "fc_a",
                "call_a",
                "WebSearch",
                "{\"query\":\"pressurized\"}",
            ),
        ];
        let (_, calls) = read(&events, true);
        let deltas: Vec<_> = calls
            .iter()
            .filter_map(|c| c.arguments_delta.clone())
            .collect();
        assert_eq!(deltas, vec!["{\"query\":\"press", "urized\"}"]);
        assert_eq!(assemble(&calls)[0].3, "{\"query\":\"pressurized\"}");
    }
}
