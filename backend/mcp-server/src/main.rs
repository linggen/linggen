use anyhow::Result;
use rmcp::{
    handler::server::{tool::ToolRouter, ServerHandler},
    model::*,
    service::{serve_server, RequestContext, RoleServer},
    tool, tool_router,
    transport::async_rw::AsyncRwTransport,
    ErrorData as McpError,
};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

#[derive(Clone)]
struct SearchTool {
    api_url: String,
    tool_router: ToolRouter<Self>,
}

#[derive(Serialize)]
struct ApiSearchRequest {
    query: String,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct ApiChunk {
    content: String,
    document_id: String,
    source_id: String,
}

#[derive(Deserialize)]
struct ApiSearchResponse {
    results: Vec<ApiChunk>,
}

use rmcp::handler::server::wrapper::Parameters;
use rmcp::schemars::JsonSchema;

#[derive(Deserialize, JsonSchema)]
struct SearchParameters {
    /// The search query (may be modified by Cursor)
    query: String,

    /// Optional limit on the number of results
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<i64>,

    /// Original user prompt before any processing by Cursor
    #[serde(skip_serializing_if = "Option::is_none")]
    original_prompt: Option<String>,

    /// User intent if classified by Cursor (e.g., "debugging", "refactoring", "explaining")
    #[serde(skip_serializing_if = "Option::is_none")]
    user_intent: Option<String>,

    /// Current active file path
    #[serde(skip_serializing_if = "Option::is_none")]
    current_file: Option<String>,

    /// Current cursor position in the file (line:column)
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor_position: Option<String>,

    /// Any code the user has selected/highlighted
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_code: Option<String>,

    /// Previous conversation messages for context
    #[serde(skip_serializing_if = "Option::is_none")]
    conversation_context: Option<Vec<String>>,

    /// Additional metadata from Cursor
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<serde_json::Value>,
}

#[tool_router]
impl SearchTool {
    fn new(api_url: String) -> Self {
        Self {
            api_url,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Search the RememberMe knowledge base for code snippets and documentation. Provide as much context as possible for better results."
    )]
    async fn search_rememberme(
        &self,
        params: Parameters<SearchParameters>,
    ) -> Result<CallToolResult, McpError> {
        let query = params.0.query;
        let limit = params.0.limit;

        // Log all received context for debugging
        info!("MCP Search Request:");
        info!("  Query: {}", query);
        if let Some(ref original) = params.0.original_prompt {
            info!("  Original Prompt: {}", original);
        }
        if let Some(ref intent) = params.0.user_intent {
            info!("  User Intent: {}", intent);
        }
        if let Some(ref file) = params.0.current_file {
            info!("  Current File: {}", file);
        }
        if let Some(ref pos) = params.0.cursor_position {
            info!("  Cursor Position: {}", pos);
        }
        if let Some(ref code) = params.0.selected_code {
            info!("  Selected Code Length: {} chars", code.len());
        }
        if let Some(ref convo) = params.0.conversation_context {
            info!("  Conversation Messages: {}", convo.len());
        }

        let client = reqwest::Client::new();
        let limit = limit.map(|l| l as usize);

        let resp = client
            .post(format!("{}/api/search", self.api_url))
            .json(&ApiSearchRequest {
                query: query.clone(),
                limit,
            })
            .send()
            .await
            .map_err(|e| {
                McpError::internal_error(format!("Failed to send request: {}", e), None)
            })?;

        if !resp.status().is_success() {
            return Err(McpError::internal_error(
                format!("Search failed: {}", resp.status()),
                None,
            ));
        }

        let search_resp: ApiSearchResponse = resp.json().await.map_err(|e| {
            McpError::internal_error(format!("Failed to parse response: {}", e), None)
        })?;

        let mut output = String::new();

        // Include context info in the response header
        if let Some(ref file) = params.0.current_file {
            output.push_str(&format!("Context: Working in {}\n", file));
        }
        if let Some(ref intent) = params.0.user_intent {
            output.push_str(&format!("Intent: {}\n", intent));
        }
        if !output.is_empty() {
            output.push_str("\n");
        }

        if search_resp.results.is_empty() {
            output.push_str("No results found.");
        } else {
            output.push_str(&format!("Found {} results:\n\n", search_resp.results.len()));
            for (i, chunk) in search_resp.results.iter().enumerate() {
                output.push_str(&format!(
                    "Result {}:\nFile: {}\nSource: {}\nContent:\n{}\n\n---\n\n",
                    i + 1,
                    chunk.document_id,
                    chunk.source_id,
                    chunk.content
                ));
            }
        }

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }
}

impl ServerHandler for SearchTool {
    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let tool_context =
            rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.tool_router.call(tool_context).await
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools = self.tool_router.list_all();
        Ok(ListToolsResult {
            tools,
            next_cursor: None,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging to both stderr and a file
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/rememberme-mcp.log")
        .expect("Failed to open log file");

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter("info")
        .init();

    // Also log to file using eprintln (tracing already goes to stderr)
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/rememberme-mcp.log")
        .expect("Failed to open log file");
    writeln!(
        file,
        "\n=== MCP Server Starting at {} ===",
        chrono::Utc::now()
    )
    .ok();

    info!("Starting RememberMe MCP Server...");

    let api_url =
        std::env::var("REMEMBERME_API_URL").unwrap_or_else(|_| "http://localhost:3000".to_string());

    let tool = SearchTool::new(api_url);

    // Create transport
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let transport = AsyncRwTransport::new(stdin, stdout);

    serve_server(tool, transport).await?;

    Ok(())
}
