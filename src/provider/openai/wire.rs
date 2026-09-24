//! OpenAI wire shapes: tool definitions as the wire takes them, the Chat
//! Completions message/stream types, and the prompt-cache key.

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
    if !super::strict_schema::is_fully_required(&params) {
        return Some(serde_json::json!({
            "type": "function",
            "name": func.get("name")?,
            "description": func.get("description").unwrap_or(&serde_json::Value::Null),
            "parameters": params,
            "strict": false,
        }));
    }

    let strict_params = super::strict_schema::strictify_for_openai(params);
    Some(serde_json::json!({
        "type": "function",
        "name": func.get("name")?,
        "description": func.get("description").unwrap_or(&serde_json::Value::Null),
        "parameters": strict_params,
        "strict": true,
    }))
}

/// Fix malformed JSON from some providers (e.g. Gemini sends `"function":,` instead of `"function":null,`).
pub(super) fn sanitize_json(data: &str) -> std::borrow::Cow<'_, str> {
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
pub(super) fn extract_error_message(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    v.get("error")?.get("message")?.as_str().map(String::from)
}

// --- Wire types ---

#[derive(Debug, Serialize)]
pub(super) struct OaiMessage {
    pub(super) role: String,
    pub(super) content: OaiContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum OaiContent {
    Text(String),
    Parts(Vec<OaiContentPart>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub(super) enum OaiContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: OaiImageUrl },
}

#[derive(Debug, Serialize)]
pub(super) struct OaiImageUrl {
    pub(super) url: String,
}

/// An OAI message with optional tool_calls (for assistant messages in native mode).
#[derive(Debug, Serialize)]
pub(super) struct OaiMessageWithTools {
    pub(super) role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) content: Option<OaiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tool_calls: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tool_call_id: Option<String>,
    /// Function name for role="tool" messages (required by Gemini's OpenAI-compatible API).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) name: Option<String>,
    /// Gemini extra_content with thought_signature (must be echoed back for Gemini 3+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) extra_content: Option<serde_json::Value>,
}

impl OaiMessage {
    pub(super) fn from_chat(msg: &crate::message::ChatMessage) -> Self {
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
    pub(super) fn from_chat(msg: &crate::message::ChatMessage) -> Self {
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
            let first_sig = msg
                .tool_calls
                .iter()
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
                        let sig = tc
                            .thought_signature
                            .as_deref()
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
            let sig = first_sig
                .as_deref()
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

#[derive(Debug, Deserialize)]
pub(super) struct OaiStreamChunk {
    pub(super) choices: Vec<OaiStreamChoice>,
    #[serde(default)]
    pub(super) usage: Option<OaiUsage>,
    /// Gemini extra_content at chunk level (alternative location for thought_signature).
    #[serde(default)]
    pub(super) extra_content: Option<OaiExtraContent>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OaiPromptTokensDetails {
    #[serde(default)]
    pub(super) cached_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OaiUsage {
    #[serde(default)]
    pub(super) prompt_tokens: Option<u64>,
    #[serde(default)]
    pub(super) prompt_tokens_details: Option<OaiPromptTokensDetails>,
    #[serde(default)]
    pub(super) completion_tokens: Option<u64>,
    #[serde(default)]
    pub(super) total_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OaiStreamChoice {
    pub(super) delta: OaiStreamDelta,
    /// Gemini extra_content (contains thought_signature for tool calls).
    #[serde(default)]
    pub(super) extra_content: Option<OaiExtraContent>,
}

/// Gemini-specific extra_content on choices (OpenAI-compatible endpoint).
#[derive(Debug, Deserialize)]
pub(super) struct OaiExtraContent {
    #[serde(default)]
    pub(super) google: Option<OaiGoogleExtra>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OaiGoogleExtra {
    #[serde(default)]
    pub(super) thought_signature: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OaiStreamDelta {
    pub(super) content: Option<String>,
    /// Tool call deltas from native function calling.
    /// OpenAI streams these incrementally: first chunk has id+name, subsequent chunks have argument fragments.
    #[serde(default)]
    pub(super) tool_calls: Option<Vec<OaiStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OaiStreamToolCall {
    #[serde(default)]
    pub(super) index: usize,
    #[serde(default)]
    pub(super) id: Option<String>,
    #[serde(default)]
    pub(super) function: Option<OaiStreamToolCallFunction>,
    /// Gemini thought_signature lives here (on each tool_call in the delta).
    #[serde(default)]
    pub(super) extra_content: Option<OaiExtraContent>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OaiStreamToolCallFunction {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) arguments: Option<String>,
}

/// Routes requests that share a system prompt to the same cache: a turn's
/// calls and the next-prompt fork after it (`engine/suggestion.rs`). Sent as
/// `prompt_cache_key` and the `session_id` header, both where Codex CLI puts
/// its conversation id, and shaped like one (a UUID).
///
/// A stable hash (SHA-256), not `DefaultHasher`: the key must mean the same
/// thing after a restart or a new build, or a warm cache goes cold for it.
pub(super) fn prompt_cache_key(instructions: &str) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(instructions.as_bytes());
    let hex =
        |r: std::ops::Range<usize>| d[r].iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        hex(0..4),
        hex(4..6),
        hex(6..8),
        hex(8..10),
        hex(10..16)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_cache_key_follows_the_instructions() {
        assert_eq!(
            prompt_cache_key("You are Ling."),
            prompt_cache_key("You are Ling.")
        );
        assert_ne!(
            prompt_cache_key("You are Ling."),
            prompt_cache_key("You are Yinyue.")
        );
        assert_eq!(prompt_cache_key("x").len(), 36, "shaped like a UUID");
        // SHA-256("x") — the same key on every build and every restart.
        assert_eq!(
            prompt_cache_key("x"),
            "2d711642-b726-b044-0162-7ca9fbac32f5"
        );
    }

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
        assert_eq!(
            wire["parameters"]["properties"]["body"]["type"],
            json!("object")
        );
    }
}
