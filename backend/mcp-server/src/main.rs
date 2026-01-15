use anyhow::Result;
use rmcp::{
    handler::server::tool::ToolRouter, model::*, service::ServiceExt, tool, tool_handler,
    tool_router, transport::stdio, ErrorData as McpError,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::info;

const REQUEST_TIMEOUT_SECS: u64 = 30;
const DEFAULT_LIMIT: usize = 5;

#[derive(Clone)]
struct LinggenTool {
    api_url: String,
    client: reqwest::Client,
    tool_router: ToolRouter<Self>,
}

// --- API Request/Response Types ---

#[derive(Serialize)]
struct ApiEnhanceRequest {
    query: String,
    strategy: Option<String>,
    source_id: Option<String>,
}

// Intent can be either a string or an object with intent_type and confidence
#[derive(Deserialize)]
#[serde(untagged)]
enum ApiIntent {
    Simple(String),
    Detailed {
        intent_type: String,
        confidence: f64,
    },
}

impl ApiIntent {
    fn intent_type(&self) -> &str {
        match self {
            ApiIntent::Simple(s) => s,
            ApiIntent::Detailed { intent_type, .. } => intent_type,
        }
    }

    fn confidence(&self) -> f64 {
        match self {
            ApiIntent::Simple(_) => 1.0, // Default confidence for simple intent
            ApiIntent::Detailed { confidence, .. } => *confidence,
        }
    }
}

#[derive(Deserialize)]
struct ApiContextMeta {
    source_id: String,
    #[allow(dead_code)]
    document_id: String,
    file_path: String,
}

#[derive(Deserialize)]
struct ApiEnhanceResponse {
    #[allow(dead_code)]
    original_query: String,
    enhanced_prompt: String,
    intent: ApiIntent,
    context_chunks: Vec<String>,
    #[serde(default)]
    context_metadata: Vec<ApiContextMeta>,
    preferences_applied: bool,
}

// --- Resources API Types ---

#[derive(Deserialize)]
struct ApiSourceStats {
    chunk_count: i64,
    file_count: i64,
    total_size_bytes: i64,
}

#[derive(Deserialize)]
struct ApiResource {
    id: String,
    name: String,
    resource_type: String,
    path: String,
    enabled: bool,
    #[serde(default)]
    stats: Option<ApiSourceStats>,
}

#[derive(Deserialize)]
struct ApiListResourcesResponse {
    resources: Vec<ApiResource>,
}

#[derive(Deserialize)]
struct ApiStatusResponse {
    status: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    progress: Option<String>,
}

// --- MCP Tool Parameters ---

use rmcp::handler::server::wrapper::Parameters;
use rmcp::schemars::JsonSchema;

#[derive(Deserialize, JsonSchema)]
struct SearchParameters {
    /// The search query to find relevant code and documentation
    query: String,

    /// Maximum number of context chunks to retrieve (default: 5, max: 20)
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<i64>,

    /// Prompt strategy: "full_code" (default), "reference_only", or "architectural"
    #[serde(skip_serializing_if = "Option::is_none")]
    strategy: Option<String>,

    /// Filter results to a specific source/project ID
    #[serde(skip_serializing_if = "Option::is_none")]
    source_id: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct EnhanceParameters {
    /// The user's original prompt to enhance with context
    query: String,

    /// Prompt strategy: "full_code" (includes full code), "reference_only" (file paths only), "architectural" (high-level overview)
    #[serde(skip_serializing_if = "Option::is_none")]
    strategy: Option<String>,

    /// Filter context to a specific source/project ID
    #[serde(skip_serializing_if = "Option::is_none")]
    source_id: Option<String>,
}

#[tool_router]
impl LinggenTool {
    fn new(api_url: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            api_url,
            client,
            tool_router: Self::tool_router(),
        }
    }

    /// Call the enhance API and return the response
    async fn call_enhance_api(
        &self,
        query: &str,
        strategy: Option<String>,
        source_id: Option<String>,
    ) -> Result<ApiEnhanceResponse, McpError> {
        info!(
            ">>> API Request: POST /api/enhance | query={:?}, strategy={:?}, source_id={:?}",
            query, strategy, source_id
        );

        let resp = self
            .client
            .post(format!("{}/api/enhance", self.api_url))
            .json(&ApiEnhanceRequest {
                query: query.to_string(),
                strategy,
                source_id,
            })
            .send()
            .await
            .map_err(|e| {
                tracing::error!("<<< API Error: Failed to send request: {}", e);
                McpError::internal_error(format!("Failed to send request: {}", e), None)
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::error!("<<< API Error: status={}, body={}", status, body);
            return Err(McpError::internal_error(
                format!("API request failed ({}): {}", status, body),
                None,
            ));
        }

        let response: ApiEnhanceResponse = resp.json().await.map_err(|e| {
            tracing::error!("<<< API Error: Failed to parse response: {}", e);
            McpError::internal_error(format!("Failed to parse response: {}", e), None)
        })?;

        info!(
            "<<< API Response: intent={}, chunks={}, preferences_applied={}",
            response.intent.intent_type(),
            response.context_chunks.len(),
            response.preferences_applied
        );

        Ok(response)
    }

    #[tool(
        description = "Search the Linggen knowledge base for relevant code snippets and documentation. Returns raw context chunks that match the query."
    )]
    async fn search_codebase(
        &self,
        params: Parameters<SearchParameters>,
    ) -> Result<CallToolResult, McpError> {
        let query = &params.0.query;

        // Validate and clamp limit
        let limit = params
            .0
            .limit
            .map(|l| {
                if l <= 0 {
                    DEFAULT_LIMIT
                } else {
                    l.min(20) as usize
                }
            })
            .unwrap_or(DEFAULT_LIMIT);

        info!(
            "=== TOOL CALL: search_codebase ===\n  Query from LLM: {:?}\n  Limit: {}\n  Strategy: {:?}\n  Source ID: {:?}",
            query, limit, params.0.strategy, params.0.source_id
        );

        let response = self
            .call_enhance_api(query, params.0.strategy.clone(), params.0.source_id.clone())
            .await?;

        // Build output with just the raw context chunks
        let mut output = String::new();

        if response.context_chunks.is_empty() {
            output.push_str("No relevant code found for your query.");
        } else {
            let chunks_to_show = response.context_chunks.len().min(limit);
            output.push_str(&format!(
                "Found {} relevant code chunks:\n\n",
                chunks_to_show
            ));

            for (i, chunk) in response.context_chunks.iter().take(limit).enumerate() {
                // Include metadata if available
                let meta = response.context_metadata.get(i);
                if let Some(m) = meta {
                    output.push_str(&format!(
                        "--- Chunk {} [{}] ---\nFile: {}\n\n{}\n\n",
                        i + 1,
                        m.source_id,
                        m.file_path,
                        chunk
                    ));
                } else {
                    output.push_str(&format!("--- Chunk {} ---\n{}\n\n", i + 1, chunk));
                }
            }
        }

        info!(
            "=== TOOL RESPONSE: search_codebase ===\n  Chunks returned: {}\n  Output length: {} chars",
            response.context_chunks.len().min(limit),
            output.len()
        );

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(
        description = "Enhance a user prompt with relevant context from the Linggen knowledge base. Returns a fully enhanced prompt ready for AI assistants, including detected intent and applied preferences."
    )]
    async fn enhance_prompt(
        &self,
        params: Parameters<EnhanceParameters>,
    ) -> Result<CallToolResult, McpError> {
        let query = &params.0.query;

        info!(
            "=== TOOL CALL: enhance_prompt ===\n  Query from LLM: {:?}\n  Strategy: {:?}\n  Source ID: {:?}",
            query, params.0.strategy, params.0.source_id
        );

        let response = self
            .call_enhance_api(query, params.0.strategy.clone(), params.0.source_id.clone())
            .await?;

        // Build rich output with all enhancement details
        let mut output = String::new();

        // Header with intent info
        output.push_str(&format!(
            "## Enhanced Prompt\n\n**Detected Intent:** {} (confidence: {:.0}%)\n",
            response.intent.intent_type(),
            response.intent.confidence() * 100.0
        ));

        if response.preferences_applied {
            output.push_str("**User Preferences:** Applied\n");
        }

        output.push_str(&format!(
            "**Context Chunks:** {}\n\n",
            response.context_chunks.len()
        ));

        // The enhanced prompt itself
        output.push_str("---\n\n");
        output.push_str(&response.enhanced_prompt);
        output.push_str("\n\n---\n\n");

        // Context sources for reference
        if !response.context_metadata.is_empty() {
            output.push_str("### Context Sources\n\n");
            for (i, meta) in response.context_metadata.iter().enumerate() {
                output.push_str(&format!(
                    "{}. `{}` ({})\n",
                    i + 1,
                    meta.file_path,
                    meta.source_id
                ));
            }
        }

        info!(
            "=== TOOL RESPONSE: enhance_prompt ===\n  Intent: {}\n  Chunks: {}\n  Output length: {} chars",
            response.intent.intent_type(),
            response.context_chunks.len(),
            output.len()
        );

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(
        description = "List all indexed sources/projects in Linggen. Shows what codebases are available for searching, including their stats (file count, chunk count, size)."
    )]
    async fn list_sources(&self) -> Result<CallToolResult, McpError> {
        info!("=== TOOL CALL: list_sources ===");

        info!(">>> API Request: GET /api/resources");

        let resp = self
            .client
            .get(format!("{}/api/resources", self.api_url))
            .send()
            .await
            .map_err(|e| {
                McpError::internal_error(format!("Failed to send request: {}", e), None)
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(McpError::internal_error(
                format!("API request failed ({}): {}", status, body),
                None,
            ));
        }

        let resources: ApiListResourcesResponse = resp.json().await.map_err(|e| {
            tracing::error!("<<< API Error: Failed to parse response: {}", e);
            McpError::internal_error(format!("Failed to parse response: {}", e), None)
        })?;

        info!(
            "<<< API Response: {} sources found",
            resources.resources.len()
        );

        let mut output = String::new();

        if resources.resources.is_empty() {
            output.push_str("No sources indexed in Linggen.\n\n");
            output.push_str("To add a source, use the Linggen web UI at http://localhost:5173");
        } else {
            output.push_str(&format!(
                "## Indexed Sources ({} total)\n\n",
                resources.resources.len()
            ));

            for resource in &resources.resources {
                output.push_str(&format!("### {}\n", resource.name));
                output.push_str(&format!("- **ID:** `{}`\n", resource.id));
                output.push_str(&format!("- **Type:** {}\n", resource.resource_type));
                output.push_str(&format!("- **Path:** `{}`\n", resource.path));
                output.push_str(&format!(
                    "- **Enabled:** {}\n",
                    if resource.enabled { "Yes" } else { "No" }
                ));

                if let Some(stats) = &resource.stats {
                    output.push_str(&format!("- **Files:** {}\n", stats.file_count));
                    output.push_str(&format!("- **Chunks:** {}\n", stats.chunk_count));
                    let size_mb = stats.total_size_bytes as f64 / 1_048_576.0;
                    output.push_str(&format!("- **Size:** {:.2} MB\n", size_mb));
                }

                output.push_str("\n");
            }
        }

        info!(
            "=== TOOL RESPONSE: list_sources ===\n  Sources: {}\n  Output length: {} chars",
            resources.resources.len(),
            output.len()
        );

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(
        description = "Get the current status of the Linggen backend service. Shows if the service is ready, initializing, or has errors."
    )]
    async fn get_status(&self) -> Result<CallToolResult, McpError> {
        info!("=== TOOL CALL: get_status ===");
        info!(">>> API Request: GET /api/status");

        let resp = self
            .client
            .get(format!("{}/api/status", self.api_url))
            .send()
            .await
            .map_err(|e| {
                McpError::internal_error(format!("Failed to send request: {}", e), None)
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(McpError::internal_error(
                format!("API request failed ({}): {}", status, body),
                None,
            ));
        }

        let status: ApiStatusResponse = resp.json().await.map_err(|e| {
            tracing::error!("<<< API Error: Failed to parse response: {}", e);
            McpError::internal_error(format!("Failed to parse response: {}", e), None)
        })?;

        info!(
            "<<< API Response: status={}, message={:?}, progress={:?}",
            status.status, status.message, status.progress
        );

        let mut output = String::new();
        output.push_str("## Linggen Status\n\n");
        output.push_str(&format!("**Status:** {}\n", status.status));

        if let Some(msg) = &status.message {
            output.push_str(&format!("**Message:** {}\n", msg));
        }

        if let Some(progress) = &status.progress {
            output.push_str(&format!("**Progress:** {}\n", progress));
        }

        info!(
            "=== TOOL RESPONSE: get_status ===\n  Status: {}\n  Output length: {} chars",
            status.status,
            output.len()
        );

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }
}

// Use the tool_handler macro to implement ServerHandler with proper tool routing
#[tool_handler]
impl rmcp::ServerHandler for LinggenTool {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2025_06_18,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "linggen-mcp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                ..Default::default()
            },
            instructions: Some(
                "Linggen MCP Server - Search and enhance prompts with your codebase context."
                    .to_string(),
            ),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Set up logging to both stderr and a file
    use std::io::Write;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // Open log file once
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/linggen-mcp.log")
        .expect("Failed to open log file");

    // Create a layer that writes to stderr
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(false);

    // Create a layer that writes to the log file
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::sync::Mutex::new(log_file))
        .with_ansi(false);

    // Combine layers with an env filter
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new("info"))
        .with(stderr_layer)
        .with(file_layer)
        .init();

    // Write startup marker to log
    {
        let mut marker_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/linggen-mcp.log")
            .expect("Failed to open log file");
        writeln!(
            marker_file,
            "\n=== MCP Server Starting at {} ===",
            chrono::Utc::now()
        )
        .ok();
    }

    info!("Starting Linggen MCP Server...");

    // Basic CLI argument parsing
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--version" || arg == "-V" {
            println!("mcp-server {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
    }

    let api_url =
        std::env::var("LINGGEN_API_URL").unwrap_or_else(|_| "http://localhost:8787".to_string());

    info!("API URL: {}", api_url);

    let tool = LinggenTool::new(api_url);

    // Use the recommended serve pattern with stdio transport
    let service = tool.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("Error starting server: {}", e);
    })?;

    info!("Server started, waiting for requests...");

    // Keep the server running until the client disconnects
    let result = service.waiting().await;

    // Log shutdown
    info!("Server shutting down...");
    {
        use std::io::Write;
        let mut marker_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/linggen-mcp.log")
            .expect("Failed to open log file");
        writeln!(
            marker_file,
            "=== MCP Server Stopped at {} ===\n",
            chrono::Utc::now()
        )
        .ok();
    }
    info!("Server stopped");

    result?;
    Ok(())
}
