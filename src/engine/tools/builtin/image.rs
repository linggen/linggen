//! Pictures: screenshots and generated images.

use super::super::file_tools::CaptureScreenshotArgs;
use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

pub struct CaptureScreenshotTool;
#[async_trait]
impl Tool for CaptureScreenshotTool {
    fn name(&self) -> &'static str {
        "capture_screenshot"
    }
    fn description(&self) -> &'static str {
        "Capture a screenshot of a URL."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "URL to capture"},
                "delay_ms": {"type": "integer", "description": "Delay before capture in milliseconds"}
            },
            "required": ["url"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "capture_screenshot",
            "args": {"url": "string", "delay_ms": "number?"},
            "returns": "{url,base64}"
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: CaptureScreenshotArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for capture_screenshot: {}", e))?;
        tools.capture_screenshot(args).await
    }
}

/// One picture from a text prompt, drawn on this Mac by the local picture
/// lane, saved in the active skill's own `data/pictures/` folder and
/// returned as a URL the chat and the app page can show. Skills opt in
/// with `allowed-tools: [GenerateImage]`; the machine opts in through
/// the lane gate, so a Mac that cannot draw never sees the tool.
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
        "Draw one picture from a prompt with the local picture model and save it in this skill's data/pictures folder. \
         Returns its URL; show it with a markdown image. About 15 seconds a picture; give a reference picture \
         (a file inside the skill) to keep its pose and shape."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Edit
    }
    fn max_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs(300))
    }
    fn lane(&self) -> Option<&'static crate::runtime::Lane> {
        Some(&crate::runtime::PICTURES_LANE)
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string", "description": "What to draw, subject first, then the style"},
                "name": {"type": "string", "description": "File name without extension, e.g. fuzhu; a new name never overwrites an old picture"},
                "shape": {"type": "string", "enum": ["square", "landscape"], "description": "square 512×512 (default) or landscape 768×512"},
                "reference": {"type": "string", "description": "Optional path inside the skill folder of a picture to keep the pose and shape of"},
                "seed": {"type": "integer", "description": "Optional; the same prompt and seed draw the same picture"}
            },
            "required": ["prompt", "name"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "GenerateImage",
            "args": {"prompt": "string", "name": "string", "shape": "square|landscape?", "reference": "string?", "seed": "number?"},
            "returns": "{url,path,seconds,seed}",
            "notes": "Local picture model; ~15 s a picture. Show the url as a markdown image."
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
    pub name: String,
    #[serde(default)]
    pub shape: Option<String>,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub seed: Option<u64>,
}
