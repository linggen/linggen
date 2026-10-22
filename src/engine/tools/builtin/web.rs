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
         metered against the account's monthly pool). Returns titles, URLs \
         and snippets, each with a number such as [turn2search1]: writing a \
         number in the answer shows that source as a link. Text only — a \
         page's pictures come from WebFetch. If it reports a key, sign-in, \
         or quota error, do not retry — tell the user instead."
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
            Err(err) => return Err(search_failed(err)),
        };
        Ok(ToolResult::WebSearchResults {
            query: args.query,
            results,
        })
    }
}

/// Counts a failed search, and tells the model what still works when the
/// search service could not be reached: a timeout is not the web being down.
fn search_failed(err: anyhow::Error) -> anyhow::Error {
    let code = search_error_code(&err.to_string());
    crate::telemetry::global().error("search", code);
    match code {
        "network" => anyhow::anyhow!("{err:#}. {SEARCH_UNREACHABLE}"),
        _ => err,
    }
}

const SEARCH_UNREACHABLE: &str =
    "Search did not answer, but WebFetch still opens pages directly — fetch a page you know, such as the maker's site.";

fn search_error_code(msg: &str) -> &'static str {
    if msg.contains("AUTH_REQUIRED") {
        return "auth_required";
    }
    if msg.contains("failed to reach") {
        return "network";
    }
    "provider_http"
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
        "Fetch a URL and return its content as text. HTML tags are stripped. Default max 100KB. \
         Also returns the page's pictures (its main image, and pictures in the text with their \
         captions). The page and each picture come with numbers such as [turn2search1] or \
         [turn2image1]: writing a number in the answer shows that source as a link, or that \
         picture itself — worth it when the person asks what something looks like or a step \
         is clearer seen, so fetch a good page for it (the maker's or a product page)."
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
            "returns": "{url,content,content_type,truncated,pictures}",
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
            title: result.title,
            pictures: result.pictures,
            id: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreachable_search_points_at_webfetch() {
        let err = anyhow::anyhow!("operation timed out").context("failed to reach Tavily");
        let said = format!("{:#}", search_failed(err));
        assert!(
            said.starts_with("failed to reach Tavily: operation timed out. "),
            "{said}"
        );
        assert!(said.ends_with(SEARCH_UNREACHABLE), "{said}");

        let refused = anyhow::anyhow!("Tavily refused the key saved in Settings: bad (401)");
        assert_eq!(
            format!("{:#}", search_failed(refused)),
            "Tavily refused the key saved in Settings: bad (401)"
        );
    }
}
