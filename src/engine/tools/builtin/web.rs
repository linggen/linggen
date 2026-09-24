//! The web: search and fetch.

use super::super::delegation::{WebFetchArgs, WebSearchArgs};
use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct WebSearchTool;
#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &'static str {
        "WebSearch"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["web_search"]
    }
    fn description(&self) -> &'static str {
        "Search the web (Tavily: the user's own key from Settings when saved, \
         otherwise Linggen Cloud, which needs linggen.dev sign-in and is \
         metered against the account's monthly pool). Returns titles, URLs, \
         and snippets. If it reports a key, sign-in, or quota error, do not \
         retry — tell the user instead."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"},
                "max_results": {"type": "integer", "description": "Maximum results (default: 5, max: 10)"}
            },
            "required": ["query"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "WebSearch",
            "args": {"query": "string", "max_results": "number?"},
            "returns": "{results:[{title,url,snippet}]}",
            "notes": "Search the web. Default 5 results, max 10. Uses the Tavily key from Settings, else linggen.dev sign-in."
        })
    }
    async fn execute(&self, _tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: WebSearchArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for WebSearch: {}", e))?;
        let max = args.max_results.unwrap_or(5).min(10);
        let results = match crate::engine::web_search::web_search(&args.query, max).await {
            Ok(results) => results,
            Err(err) => {
                let msg = err.to_string();
                let code = if msg.contains("AUTH_REQUIRED") {
                    "auth_required"
                } else if msg.contains("failed to reach") {
                    "network"
                } else {
                    "provider_http"
                };
                crate::telemetry::global().error("search", code);
                return Err(err);
            }
        };
        Ok(ToolResult::WebSearchResults {
            query: args.query,
            results,
        })
    }
}

pub struct WebFetchTool;
#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &'static str {
        "WebFetch"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["web_fetch"]
    }
    fn description(&self) -> &'static str {
        "Fetch a URL and return its content as text. HTML tags are stripped. Default max 100KB."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "URL to fetch"},
                "max_bytes": {"type": "integer", "description": "Maximum bytes to return (default: 100000)"}
            },
            "required": ["url"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "WebFetch",
            "args": {"url": "string", "max_bytes": "number?"},
            "returns": "{url,content,content_type,truncated}",
            "notes": "Fetch a URL and return its content as text. HTML is stripped of tags. Default max 100KB."
        })
    }
    async fn execute(&self, _tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: WebFetchArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for WebFetch: {}", e))?;
        let result = crate::engine::web_fetch::fetch_url(&args.url, args.max_bytes).await?;
        Ok(ToolResult::WebFetchContent {
            url: result.url,
            content: result.content,
            content_type: result.content_type,
            truncated: result.truncated,
        })
    }
}
