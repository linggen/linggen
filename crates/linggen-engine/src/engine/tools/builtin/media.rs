//! Finding real pictures and videos: `ImageSearch` and `VideoSearch`, each
//! a search of its own beside `WebSearch` (run in parallel with it, never
//! inside it), each find numbered for the answer to show.

use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Deserialize)]
struct QueryArgs {
    #[serde(alias = "q")]
    query: String,
}

fn query_of(tool: &str, args: Value) -> Result<String> {
    let args: QueryArgs = serde_json::from_value(args)
        .map_err(|e| anyhow::anyhow!("invalid args for {tool}: {e}"))?;
    Ok(args.query)
}

fn query_schema(what: &str) -> Value {
    json!({
        "type": "object",
        "properties": {"query": {"type": "string", "description": what}},
        "required": ["query"]
    })
}

pub struct ImageSearchTool;
#[async_trait]
impl Tool for ImageSearchTool {
    fn name(&self) -> &'static str {
        "ImageSearch"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["image_search"]
    }
    fn description(&self) -> &'static str {
        "Find real pictures of something (a product, tool, part, place, or how it looks) on \
         web pages, each with what it shows and the page it is from. Each picture has a number \
         such as [turn3image1]: writing it in the answer shows that picture there, so put it in \
         the paragraph it illustrates. Run it alongside WebSearch, not instead of it. Same \
         sign-in and quota as WebSearch."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn max_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs(30))
    }
    fn args_schema(&self) -> Value {
        query_schema("What the pictures should show, as a search query")
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "ImageSearch",
            "args": {"query": "string"},
            "returns": "{pictures:[{id,caption,page}]}",
            "notes": "Real pictures of a thing from web pages; show one by writing its number."
        })
    }
    async fn execute(&self, _tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let query = query_of("ImageSearch", call.args)?;
        let pictures = crate::engine::image_search::image_search(&query)
            .await
            .map_err(super::web::search_failed)?;
        Ok(ToolResult::PictureSearchResults {
            query,
            pictures,
            sheet: String::new(),
        })
    }
}

pub struct VideoSearchTool;
#[async_trait]
impl Tool for VideoSearchTool {
    fn name(&self) -> &'static str {
        "VideoSearch"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["video_search"]
    }
    fn description(&self) -> &'static str {
        "Find videos (YouTube) about something, each with its title, channel and length. Each \
         video has a number such as [turn4video1]: writing it in the answer shows a card the \
         person can play. Worth it for how-to and technique questions, where watching beats \
         reading. Same sign-in and quota as WebSearch."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn max_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs(30))
    }
    fn args_schema(&self) -> Value {
        query_schema("What the videos should be about, as a search query")
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "VideoSearch",
            "args": {"query": "string"},
            "returns": "{videos:[{id,title,channel,duration}]}",
            "notes": "Videos about a thing; show one by writing its number."
        })
    }
    async fn execute(&self, _tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let query = query_of("VideoSearch", call.args)?;
        let videos = crate::engine::video_search::video_search(&query)
            .await
            .map_err(super::web::search_failed)?;
        Ok(ToolResult::VideoSearchResults { query, videos })
    }
}
