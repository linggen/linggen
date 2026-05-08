//! Chat message types shared by every model provider.
//!
//! `ChatMessage` is the canonical conversation-history type used across the
//! engine, server, and provider clients. `ToolCallMessage` and
//! `ToolCallFunction` describe an assistant-emitted tool call (native
//! function-calling format). Concrete provider clients are responsible for
//! mapping these into their wire payloads (`sanitize_for_ollama`, etc.).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// Cache control hint for API providers that support prompt caching.
    /// E.g. `{"type": "ephemeral"}` for Anthropic/OpenAI cache breakpoints.
    /// Skipped during serialization for providers that don't support it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<serde_json::Value>,
    /// Tool calls emitted by the assistant (native function calling mode).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallMessage>,
    /// The tool_call_id this message is a result for (role="tool" messages).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Function name for role="tool" messages (required by Gemini's OpenAI-compatible API).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    /// Strip fields that Ollama doesn't understand (cache_control, name,
    /// thinking from prior turns) to avoid payload noise and potential
    /// re-injection of model reasoning.
    pub fn sanitize_for_ollama(&self) -> Self {
        Self {
            role: self.role.clone(),
            content: self.content.clone(),
            thinking: None,
            images: self.images.clone(),
            cache_control: None,
            tool_calls: self.tool_calls.clone(),
            tool_call_id: self.tool_call_id.clone(),
            name: None,
        }
    }

    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            thinking: None,
            images: Vec::new(),
            cache_control: None,
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
        }
    }

    pub fn with_images(mut self, images: Vec<String>) -> Self {
        self.images = images;
        self
    }

    /// Create a tool result message with function name (required by Gemini's
    /// OpenAI-compatible API).
    pub fn tool_result_named(
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: "tool".to_string(),
            content: content.into(),
            thinking: None,
            images: Vec::new(),
            cache_control: None,
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
            name: Some(name.into()),
        }
    }

    /// Create an assistant message with tool calls (native function calling).
    pub fn assistant_with_tool_calls(calls: Vec<ToolCallMessage>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: String::new(),
            thinking: None,
            images: Vec::new(),
            cache_control: None,
            tool_calls: calls,
            tool_call_id: None,
            name: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallMessage {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(rename = "type", default, skip_serializing_if = "String::is_empty")]
    pub call_type: String, // "function"
    pub function: ToolCallFunction,
    /// Gemini thought signature — opaque token that must be echoed back in
    /// conversation history for Gemini 3+ models to work with tool calling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: serde_json::Value,
}
