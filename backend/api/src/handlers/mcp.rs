//! MCP (Model Context Protocol) HTTP/SSE Handler
//!
//! Provides MCP tools via HTTP endpoints so Cursor users can connect
//! without installing a local binary. Uses:
//! - `GET /mcp/sse` for Server-Sent Events (server -> client streaming)
//! - `POST /mcp/message` for client -> server MCP messages

use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use dashmap::DashMap;
use futures::stream::Stream;
use linggen_enhancement::{EnhancedPrompt, PromptEnhancer, PromptStrategy};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, info, warn};

use super::index::AppState;

// ============================================================================
// Configuration
// ============================================================================

const SSE_CHANNEL_SIZE: usize = 100;
const SSE_KEEPALIVE_SECS: u64 = 15;

// ============================================================================
// MCP Protocol Types
// ============================================================================

/// MCP JSON-RPC request envelope
#[derive(Debug, Deserialize)]
pub struct McpRequest {
    jsonrpc: String,
    id: Option<serde_json::Value>,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

/// MCP JSON-RPC response envelope
#[derive(Debug, Serialize)]
pub struct McpResponse {
    jsonrpc: String,
    id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<McpErrorData>,
}

#[derive(Debug, Serialize)]
struct McpErrorData {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

impl McpResponse {
    fn success(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Option<serde_json::Value>, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(McpErrorData {
                code,
                message,
                data: None,
            }),
        }
    }
}

// MCP error codes
const MCP_INVALID_REQUEST: i32 = -32600;
const MCP_METHOD_NOT_FOUND: i32 = -32601;
const MCP_INTERNAL_ERROR: i32 = -32603;

// ============================================================================
// MCP State (extends AppState)
// ============================================================================

/// MCP-specific shared state
pub struct McpState {
    /// Active SSE client connections: session_id -> sender
    pub clients: DashMap<String, mpsc::Sender<Result<Event, Infallible>>>,
    /// Optional access token for basic auth
    pub access_token: Option<String>,
    /// Connection counter for metrics
    connection_count: std::sync::atomic::AtomicU64,
    /// Request counter for metrics
    request_count: std::sync::atomic::AtomicU64,
}

impl McpState {
    pub fn new() -> Self {
        let access_token = std::env::var("LINGGEN_ACCESS_TOKEN").ok();
        if access_token.is_some() {
            info!("MCP access token: configured (requests will require authentication)");
        } else {
            info!("MCP access token: not configured (all requests allowed)");
        }

        Self {
            clients: DashMap::new(),
            access_token,
            connection_count: std::sync::atomic::AtomicU64::new(0),
            request_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn increment_connections(&self) -> u64 {
        self.connection_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
    }

    fn decrement_connections(&self) -> u64 {
        self.connection_count
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed)
            - 1
    }

    fn increment_requests(&self) -> u64 {
        self.request_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
    }

    pub fn get_stats(&self) -> (u64, u64) {
        (
            self.connection_count
                .load(std::sync::atomic::Ordering::Relaxed),
            self.request_count
                .load(std::sync::atomic::Ordering::Relaxed),
        )
    }

    /// Validate access token if configured
    fn validate_token(&self, headers: &HeaderMap) -> bool {
        match &self.access_token {
            None => true, // No token configured, allow all
            Some(expected) => {
                // Check X-Linggen-Token header
                if let Some(token) = headers.get("X-Linggen-Token") {
                    if let Ok(token_str) = token.to_str() {
                        return token_str == expected;
                    }
                }
                // Also check Authorization: Bearer <token>
                if let Some(auth) = headers.get(header::AUTHORIZATION) {
                    if let Ok(auth_str) = auth.to_str() {
                        if let Some(token) = auth_str.strip_prefix("Bearer ") {
                            return token == expected;
                        }
                    }
                }
                false
            }
        }
    }
}

impl Default for McpState {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Combined State for MCP handlers
// ============================================================================

pub struct McpAppState {
    pub app: Arc<AppState>,
    pub mcp: Arc<McpState>,
}

// ============================================================================
// SSE Query Parameters
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct SseQuery {
    /// Optional session ID; if not provided, server generates one
    session_id: Option<String>,
}

// ============================================================================
// Handlers
// ============================================================================

/// GET /mcp/sse - Establish SSE connection
pub async fn mcp_sse_handler(
    State(state): State<Arc<McpAppState>>,
    headers: HeaderMap,
    Query(query): Query<SseQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, &'static str)> {
    // Validate access token if configured
    if !state.mcp.validate_token(&headers) {
        warn!("SSE connection rejected: invalid or missing access token");
        return Err((StatusCode::UNAUTHORIZED, "Invalid or missing access token"));
    }

    // Generate or use provided session ID
    let session_id = query
        .session_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let conn_num = state.mcp.increment_connections();
    info!(
        "MCP SSE connection established: session_id={}, active_connections={}",
        session_id, conn_num
    );

    // Create channel for this client
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(SSE_CHANNEL_SIZE);

    // Store sender in state
    state.mcp.clients.insert(session_id.clone(), tx.clone());

    // Send initial endpoint event with session info
    let endpoint_event = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "endpoint",
        "params": {
            "session_id": session_id,
            "message_url": "/mcp/message"
        }
    });

    let tx_clone = tx.clone();
    let session_id_clone = session_id.clone();
    tokio::spawn(async move {
        if let Err(e) = tx_clone
            .send(Ok(Event::default()
                .event("endpoint")
                .data(endpoint_event.to_string())))
            .await
        {
            warn!(
                "Failed to send endpoint event for session {}: {}",
                session_id_clone, e
            );
        }
    });

    // Clean up when stream ends
    let mcp_state = state.mcp.clone();
    let session_id_for_cleanup = session_id.clone();
    tokio::spawn(async move {
        // Wait for the channel to close (receiver dropped)
        tokio::time::sleep(Duration::from_secs(3600)).await;
        mcp_state.clients.remove(&session_id_for_cleanup);
        let conn_num = mcp_state.decrement_connections();
        info!(
            "MCP SSE connection cleaned up: session_id={}, active_connections={}",
            session_id_for_cleanup, conn_num
        );
    });

    let stream = ReceiverStream::new(rx);
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(SSE_KEEPALIVE_SECS))
            .text("ping"),
    ))
}

/// POST /mcp/message - Handle MCP messages
pub async fn mcp_message_handler(
    State(state): State<Arc<McpAppState>>,
    headers: HeaderMap,
    Json(request): Json<McpRequest>,
) -> impl IntoResponse {
    // Validate access token if configured
    if !state.mcp.validate_token(&headers) {
        warn!("MCP message rejected: invalid or missing access token");
        return (
            StatusCode::UNAUTHORIZED,
            Json(McpResponse::error(
                request.id,
                MCP_INVALID_REQUEST,
                "Invalid or missing access token".to_string(),
            )),
        );
    }

    let req_num = state.mcp.increment_requests();
    let start_time = std::time::Instant::now();

    info!(
        "MCP message received: method={}, id={:?}, request_num={}",
        request.method, request.id, req_num
    );

    // Validate JSON-RPC version
    if request.jsonrpc != "2.0" {
        return (
            StatusCode::BAD_REQUEST,
            Json(McpResponse::error(
                request.id,
                MCP_INVALID_REQUEST,
                "Invalid JSON-RPC version".to_string(),
            )),
        );
    }

    // Route to appropriate handler
    let response = match request.method.as_str() {
        "initialize" => handle_initialize(request.id.clone()).await,
        "initialized" => handle_initialized(request.id.clone()).await,
        "tools/list" => handle_tools_list(request.id.clone()).await,
        "tools/call" => handle_tools_call(request.id.clone(), &request.params, &state.app).await,
        "ping" => handle_ping(request.id.clone()).await,
        _ => McpResponse::error(
            request.id,
            MCP_METHOD_NOT_FOUND,
            format!("Method not found: {}", request.method),
        ),
    };

    let elapsed = start_time.elapsed();
    info!(
        "MCP message completed: method={}, duration={:?}",
        request.method, elapsed
    );

    (StatusCode::OK, Json(response))
}

/// Handle initialize request
async fn handle_initialize(id: Option<serde_json::Value>) -> McpResponse {
    info!("Handling MCP initialize request");

    let result = serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "linggen",
            "version": env!("CARGO_PKG_VERSION")
        }
    });

    McpResponse::success(id, result)
}

/// Handle initialized notification
async fn handle_initialized(id: Option<serde_json::Value>) -> McpResponse {
    info!("MCP client initialized");
    McpResponse::success(id, serde_json::json!({}))
}

/// Handle tools/list request
async fn handle_tools_list(id: Option<serde_json::Value>) -> McpResponse {
    info!("Handling MCP tools/list request");

    let tools = get_tool_definitions();
    let result = serde_json::json!({
        "tools": tools
    });

    McpResponse::success(id, result)
}

/// Handle tools/call request
async fn handle_tools_call(
    id: Option<serde_json::Value>,
    params: &serde_json::Value,
    app_state: &Arc<AppState>,
) -> McpResponse {
    let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let tool_args = params.get("arguments").cloned().unwrap_or_default();

    info!("Handling MCP tools/call: name={}", tool_name);

    match execute_tool(tool_name, tool_args, app_state).await {
        Ok(content) => {
            let result = serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": content
                }]
            });
            McpResponse::success(id, result)
        }
        Err(e) => {
            error!("MCP tool execution failed: {}", e);
            McpResponse::error(id, MCP_INTERNAL_ERROR, e.to_string())
        }
    }
}

/// Handle ping request
async fn handle_ping(id: Option<serde_json::Value>) -> McpResponse {
    McpResponse::success(id, serde_json::json!({}))
}

/// Health check endpoint for MCP
pub async fn mcp_health_handler(State(state): State<Arc<McpAppState>>) -> impl IntoResponse {
    let (connections, requests) = state.mcp.get_stats();
    let response = serde_json::json!({
        "status": "ok",
        "active_connections": connections,
        "total_requests": requests
    });
    (StatusCode::OK, Json(response))
}

// ============================================================================
// Tool Definitions
// ============================================================================

const DEFAULT_LIMIT: usize = 5;

/// Get MCP tool definitions for tools/list response
fn get_tool_definitions() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "search_codebase",
            "description": "Search the Linggen knowledge base for relevant code snippets and documentation. Returns raw context chunks that match the query.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query to find relevant code and documentation"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of context chunks to retrieve (default: 5, max: 20)"
                    },
                    "strategy": {
                        "type": "string",
                        "description": "Prompt strategy: \"full_code\" (default), \"reference_only\", or \"architectural\""
                    },
                    "source_id": {
                        "type": "string",
                        "description": "Filter results to a specific source/project ID"
                    }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "enhance_prompt",
            "description": "Enhance a user prompt with relevant context from the Linggen knowledge base. Returns a fully enhanced prompt ready for AI assistants, including detected intent and applied preferences.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The user's original prompt to enhance with context"
                    },
                    "strategy": {
                        "type": "string",
                        "description": "Prompt strategy: \"full_code\" (includes full code), \"reference_only\" (file paths only), \"architectural\" (high-level overview)"
                    },
                    "source_id": {
                        "type": "string",
                        "description": "Filter context to a specific source/project ID"
                    }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "list_sources",
            "description": "List all indexed sources/projects in Linggen. Shows what codebases are available for searching, including their stats (file count, chunk count, size).",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        serde_json::json!({
            "name": "get_status",
            "description": "Get the current status of the Linggen backend service. Shows if the service is ready, initializing, or has errors.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
    ]
}

// ============================================================================
// Tool Implementations
// ============================================================================

#[derive(Deserialize)]
struct SearchParams {
    query: String,
    limit: Option<i64>,
    strategy: Option<String>,
    source_id: Option<String>,
}

#[derive(Deserialize)]
struct EnhanceParams {
    query: String,
    strategy: Option<String>,
    source_id: Option<String>,
}

/// Execute a tool by name with given arguments
async fn execute_tool(
    name: &str,
    args: serde_json::Value,
    app_state: &Arc<AppState>,
) -> anyhow::Result<String> {
    match name {
        "search_codebase" => execute_search_codebase(args, app_state).await,
        "enhance_prompt" => execute_enhance_prompt(args, app_state).await,
        "list_sources" => execute_list_sources(app_state).await,
        "get_status" => execute_get_status(app_state).await,
        _ => anyhow::bail!("Unknown tool: {}", name),
    }
}

async fn execute_search_codebase(
    args: serde_json::Value,
    app_state: &Arc<AppState>,
) -> anyhow::Result<String> {
    let params: SearchParams = serde_json::from_value(args)?;

    // Validate and clamp limit
    let limit = params
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
        "=== MCP TOOL: search_codebase ===\n  Query: {:?}\n  Limit: {}\n  Strategy: {:?}\n  Source ID: {:?}",
        params.query, limit, params.strategy, params.source_id
    );

    let response =
        call_enhance_internal(app_state, &params.query, params.strategy, params.source_id).await?;

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
        "=== MCP TOOL RESPONSE: search_codebase ===\n  Chunks returned: {}\n  Output length: {} chars",
        response.context_chunks.len().min(limit),
        output.len()
    );

    Ok(output)
}

async fn execute_enhance_prompt(
    args: serde_json::Value,
    app_state: &Arc<AppState>,
) -> anyhow::Result<String> {
    let params: EnhanceParams = serde_json::from_value(args)?;

    info!(
        "=== MCP TOOL: enhance_prompt ===\n  Query: {:?}\n  Strategy: {:?}\n  Source ID: {:?}",
        params.query, params.strategy, params.source_id
    );

    let response =
        call_enhance_internal(app_state, &params.query, params.strategy, params.source_id).await?;

    // Build rich output with all enhancement details
    let mut output = String::new();

    // Header with intent info
    output.push_str(&format!(
        "## Enhanced Prompt\n\n**Detected Intent:** {}\n",
        response.intent
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
        "=== MCP TOOL RESPONSE: enhance_prompt ===\n  Intent: {}\n  Chunks: {}\n  Output length: {} chars",
        response.intent,
        response.context_chunks.len(),
        output.len()
    );

    Ok(output)
}

async fn execute_list_sources(app_state: &Arc<AppState>) -> anyhow::Result<String> {
    info!("=== MCP TOOL: list_sources ===");

    let sources = app_state
        .metadata_store
        .get_sources()
        .map_err(|e| anyhow::anyhow!("Failed to get sources: {}", e))?;

    let mut output = String::new();

    if sources.is_empty() {
        output.push_str("No sources indexed in Linggen.\n\n");
        output.push_str("To add a source, use the Linggen web UI.");
    } else {
        output.push_str(&format!("## Indexed Sources ({} total)\n\n", sources.len()));

        for source in &sources {
            output.push_str(&format!("### {}\n", source.name));
            output.push_str(&format!("- **ID:** `{}`\n", source.id));
            output.push_str(&format!("- **Type:** {:?}\n", source.source_type));
            output.push_str(&format!("- **Path:** `{}`\n", source.path));
            output.push_str(&format!(
                "- **Enabled:** {}\n",
                if source.enabled { "Yes" } else { "No" }
            ));

            if let (Some(files), Some(chunks), Some(size)) = (
                source.file_count,
                source.chunk_count,
                source.total_size_bytes,
            ) {
                output.push_str(&format!("- **Files:** {}\n", files));
                output.push_str(&format!("- **Chunks:** {}\n", chunks));
                let size_mb = size as f64 / 1_048_576.0;
                output.push_str(&format!("- **Size:** {:.2} MB\n", size_mb));
            }

            output.push_str("\n");
        }
    }

    info!(
        "=== MCP TOOL RESPONSE: list_sources ===\n  Sources: {}\n  Output length: {} chars",
        sources.len(),
        output.len()
    );

    Ok(output)
}

async fn execute_get_status(app_state: &Arc<AppState>) -> anyhow::Result<String> {
    info!("=== MCP TOOL: get_status ===");

    // Check if model is initialized
    let model_initialized = app_state
        .metadata_store
        .get_setting("model_initialized")
        .unwrap_or(None)
        .map(|v| v == "true")
        .unwrap_or(false);

    // Check for error state
    let is_error = app_state
        .metadata_store
        .get_setting("model_initialized")
        .unwrap_or(None)
        .map(|v| v == "error")
        .unwrap_or(false);

    let (status, message, progress) = if is_error {
        let error_msg = app_state
            .metadata_store
            .get_setting("init_error")
            .unwrap_or(None)
            .unwrap_or_else(|| "Model initialization failed".to_string());
        ("error".to_string(), Some(error_msg), None)
    } else if !model_initialized {
        let progress = app_state
            .metadata_store
            .get_setting("init_progress")
            .unwrap_or(None);
        let msg = progress
            .clone()
            .unwrap_or_else(|| "Initializing...".to_string());
        ("initializing".to_string(), Some(msg), progress)
    } else {
        ("ready".to_string(), None, None)
    };

    let mut output = String::new();
    output.push_str("## Linggen Status\n\n");
    output.push_str(&format!("**Status:** {}\n", status));

    if let Some(msg) = &message {
        output.push_str(&format!("**Message:** {}\n", msg));
    }

    if let Some(prog) = &progress {
        output.push_str(&format!("**Progress:** {}\n", prog));
    }

    info!(
        "=== MCP TOOL RESPONSE: get_status ===\n  Status: {}\n  Output length: {} chars",
        status,
        output.len()
    );

    Ok(output)
}

// ============================================================================
// Internal Enhancement Call (reuses existing logic)
// ============================================================================

/// Call the enhancement pipeline directly (no HTTP round-trip)
async fn call_enhance_internal(
    app_state: &Arc<AppState>,
    query: &str,
    strategy: Option<String>,
    source_id: Option<String>,
) -> anyhow::Result<EnhancedPrompt> {
    // Get user preferences
    let preferences = app_state
        .metadata_store
        .get_preferences()
        .map_err(|e| anyhow::anyhow!("Failed to load preferences: {}", e))?;

    // Get LLM instance if available
    let llm = linggen_llm::LLMSingleton::get().await;

    // Create enhancer
    let enhancer = PromptEnhancer::new(
        app_state.embedding_model.clone(),
        app_state.vector_store.clone(),
        llm,
    );

    // Get source profile if source_id is provided
    let profile = if let Some(source_id) = &source_id {
        app_state
            .metadata_store
            .get_source_profile(source_id)
            .map_err(|e| anyhow::anyhow!("Failed to load source profile: {}", e))?
    } else {
        storage::SourceProfile::default()
    };

    // Parse strategy
    let strategy = match strategy.as_deref() {
        Some("reference_only") => PromptStrategy::ReferenceOnly,
        Some("architectural") => PromptStrategy::Architectural,
        _ => PromptStrategy::FullCode,
    };

    // Run enhancement pipeline
    let result = enhancer
        .enhance(query, &preferences, &profile, strategy)
        .await
        .map_err(|e| anyhow::anyhow!("Enhancement failed: {}", e))?;

    Ok(result)
}
