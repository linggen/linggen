//! Pictures: generated images.

use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

/// One picture from a text prompt — drawn by ChatGPT on the person's
/// sign-in, else by the local picture lane — kept with the conversation
/// and numbered like any picture an answer can show. In a skill it is also
/// saved in the skill's own `data/pictures/` for its page. Offered only
/// where something can draw (`pictures::offered`).
pub struct GenerateImageTool;
#[async_trait]
impl Tool for GenerateImageTool {
    fn name(&self) -> &'static str {
        "GenerateImage"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["generate_image"]
    }
    fn description(&self) -> &'static str {
        "Draw one picture from a prompt, when the person asks for a drawing, illustration or artwork. \
         Never use it to show what a real product, place or person looks like: find a real photo with ImageSearch or WebFetch. \
         The result gives the picture a number such as [turn3image1]; write that number in the answer to show it. \
         Takes up to a minute."
    }
    /// Like WebFetch: it writes no workspace file — the picture goes to the
    /// conversation's own folder (or the active skill's data).
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn max_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs(300))
    }
    fn offered(&self) -> Result<(), String> {
        crate::pictures::offered()
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string", "description": "What to draw, subject first, then the style"},
                "shape": {"type": "string", "enum": ["square", "landscape"], "description": "square (default) or landscape"},
                "name": {"type": "string", "description": "In a skill only: file name without extension for the skill's copy, e.g. fuzhu"},
                "reference": {"type": "string", "description": "In a skill only: path inside the skill folder of a picture to keep the pose and shape of"},
                "seed": {"type": "integer", "description": "Optional; on the local picture model the same prompt and seed draw the same picture"}
            },
            "required": ["prompt"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "GenerateImage",
            "args": {"prompt": "string", "shape": "square|landscape?", "name": "string?", "reference": "string?", "seed": "number?"},
            "returns": "the picture's number, e.g. [turn3image1]",
            "notes": "Only when the person asks for a drawing; never to show a real thing. Show it by writing its number."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: GenerateImageArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for GenerateImage: {}", e))?;
        tools.generate_image(args).await
    }
}

#[derive(serde::Deserialize)]
pub struct GenerateImageArgs {
    pub prompt: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub shape: Option<String>,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub seed: Option<u64>,
}
