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

    // Listen for broadcast events from AppState
    let mut broadcast_rx = state.app.broadcast_tx.subscribe();
    let tx_broadcast = tx.clone();
    let session_id_broadcast = session_id.clone();
    tokio::spawn(async move {
        while let Ok(msg) = broadcast_rx.recv().await {
            let event = Event::default().event("notification").data(msg.to_string());
            if let Err(e) = tx_broadcast.send(Ok(event)).await {
                warn!(
                    "Failed to send broadcast event to session {}: {}",
                    session_id_broadcast, e
                );
                break; // Client likely disconnected
            }
        }
    });

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
    info!("📋 [MCP] Handling MCP tools/list request");

    let tools = get_tool_definitions();
    info!("📋 [MCP] Returning {} tool definitions", tools.len());

    // Log all tool names for debugging
    let tool_names: Vec<String> = tools
        .iter()
        .filter_map(|t| {
            t.get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    info!("📋 [MCP] Available tools: {:?}", tool_names);

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
    info!("🔧 [MCP] handle_tools_call - params: {:?}", params);

    let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let tool_args = params.get("arguments").cloned().unwrap_or_default();

    info!(
        "🔧 [MCP] Handling MCP tools/call: name={}, args={:?}",
        tool_name, tool_args
    );

    match execute_tool(tool_name, tool_args, app_state).await {
        Ok(content) => {
            info!(
                "✅ [MCP] Tool '{}' succeeded, content length: {} bytes",
                tool_name,
                content.len()
            );
            let result = serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": content
                }]
            });
            McpResponse::success(id, result)
        }
        Err(e) => {
            error!("❌ [MCP] Tool '{}' execution failed: {}", tool_name, e);
            error!("❌ [MCP] Error details: {:?}", e);
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
            "name": "list_library_packs",
            "description": "List all library packs (skills/policies) available in the global Linggen library. Returns JSON with pack metadata (id, name, folder, timestamps, etc.).",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        serde_json::json!({
            "name": "get_library_pack",
            "description": "Get a library pack by pack_id. Returns JSON containing the pack file path, parsed frontmatter (info), and full markdown content.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pack_id": {
                        "type": "string",
                        "description": "The pack id from frontmatter (e.g. 'security-policy', 'rust-rules')"
                    }
                },
                "required": ["pack_id"]
            }
        }),
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
                    },
                    "exclude_source_id": {
                        "type": "string",
                        "description": "Exclude results from a specific source/project ID"
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
                    },
                    "exclude_source_id": {
                        "type": "string",
                        "description": "Exclude results from a specific source/project ID"
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
        serde_json::json!({
            "name": "memory_search_semantic",
            "description": "Semantic (vector) search across memories using LanceDB. Returns structured JSON with source_id, file_path, title, and matching snippets. Best for finding conceptually related memories. Note: You may see references to memories in code as '//linggen memory: <ID>'. Use memory_fetch_by_meta to retrieve those specific memories.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language query for semantic search"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results (default: 10, max: 50)"
                    },
                    "source_id": {
                        "type": "string",
                        "description": "Optional: filter to a specific source/project ID"
                    }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "memory_fetch_by_meta",
            "description": "Fetch the full content of a specific memory using any metadata field (anchor) defined in its frontmatter. Common keys include 'id' (from code comments like //linggen memory: <ID>), 'title', or any custom key added by the user.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "The metadata key to search for (e.g., 'id', 'title', 'feature')"
                    },
                    "value": {
                        "type": "string",
                        "description": "The value to match exactly"
                    }
                },
                "required": ["key", "value"]
            }
        }),
        serde_json::json!({
            "name": "query_codebase",
            "description": "Query the Linggen vector database and return matching chunks with source_id and file/document identifiers (useful for extensions and manual workflows).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The query text to search for"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of chunks to return (default: 3, max: 20)"
                    },
                    "exclude_source_id": {
                        "type": "string",
                        "description": "Exclude results from a specific source/project ID"
                    }
                },
                "required": ["query"]
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
    exclude_source_id: Option<String>,
}

#[derive(Deserialize)]
struct EnhanceParams {
    query: String,
    strategy: Option<String>,
    source_id: Option<String>,
    exclude_source_id: Option<String>,
}

#[derive(Deserialize)]
struct QueryParams {
    query: String,
    limit: Option<i64>,
    exclude_source_id: Option<String>,
}

#[derive(Deserialize)]
struct MemorySearchSemanticParams {
    query: String,
    limit: Option<i64>,
    source_id: Option<String>,
}

#[derive(Deserialize)]
struct MemoryFetchByMetaParams {
    key: String,
    value: String,
}

#[derive(Deserialize)]
struct GetLibraryPackParams {
    pack_id: String,
}

/// Execute a tool by name with given arguments
async fn execute_tool(
    name: &str,
    args: serde_json::Value,
    app_state: &Arc<AppState>,
) -> anyhow::Result<String> {
    info!(
        "🔧 [MCP] execute_tool called: name={}, args={:?}",
        name, args
    );

    let result = match name {
        "list_library_packs" => execute_list_library_packs(app_state).await,
        "get_library_pack" => execute_get_library_pack(args, app_state).await,
        "search_codebase" => execute_search_codebase(args, app_state).await,
        "enhance_prompt" => execute_enhance_prompt(args, app_state).await,
        "list_sources" => execute_list_sources(app_state).await,
        "get_status" => execute_get_status(app_state).await,
        "query_codebase" => execute_query_codebase(args, app_state).await,
        "memory_search_semantic" => execute_memory_search_semantic(args, app_state).await,
        "memory_fetch_by_meta" => execute_memory_fetch_by_meta(args, app_state).await,
        _ => {
            error!("❌ [MCP] Unknown tool requested: {}", name);
            anyhow::bail!("Unknown tool: {}", name)
        }
    };

    match &result {
        Ok(_) => info!("✅ [MCP] Tool '{}' executed successfully", name),
        Err(e) => error!("❌ [MCP] Tool '{}' failed: {}", name, e),
    }

    result
}

fn extract_frontmatter_json(content: &str) -> Option<serde_json::Value> {
    if !content.starts_with("---") {
        return None;
    }

    let mut parts = content.splitn(3, "---");
    parts.next(); // skip empty before first ---
    let frontmatter = parts.next()?;

    let yaml_val: serde_yaml::Value = serde_yaml::from_str(frontmatter).ok()?;
    serde_json::to_value(yaml_val).ok()
}

fn find_pack_by_id(root: &std::path::Path, target_id: &str) -> Option<std::path::PathBuf> {
    // The target_id is now the relative path from the library root
    let pack_path = root.join(target_id);

    // Security check: ensure the resolved path is still within the library root
    if let Ok(canonical_root) = root.canonicalize() {
        if let Ok(canonical_path) = pack_path.canonicalize() {
            if !canonical_path.starts_with(&canonical_root) {
                return None;
            }
        }
    }

    if pack_path.exists() && pack_path.is_file() {
        Some(pack_path)
    } else {
        None
    }
}

async fn execute_list_library_packs(app_state: &Arc<AppState>) -> anyhow::Result<String> {
    let library_root = app_state.library_path.as_path();
    let mut packs: Vec<serde_json::Value> = Vec::new();

    fn scan_dir(
        dir: &std::path::Path,
        packs: &mut Vec<serde_json::Value>,
        library_path: &std::path::Path,
    ) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with('.') {
                            continue;
                        }
                        if name == "official" && dir != library_path {
                            continue;
                        }
                    }
                    scan_dir(&path, packs, library_path);
                } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Some(mut meta) = extract_frontmatter_json(&content) {
                            let rel_path = path
                                .strip_prefix(library_path)
                                .unwrap_or(&path)
                                .to_string_lossy()
                                .to_string();

                            let is_official = rel_path.starts_with("official/");

                            if let Some(obj) = meta.as_object_mut() {
                                obj.insert("id".to_string(), serde_json::json!(rel_path));
                                obj.insert("read_only".to_string(), serde_json::json!(is_official));

                                // Add full relative folder path info
                                if let Some(parent) = path.parent() {
                                    let rel_folder = parent
                                        .strip_prefix(library_path)
                                        .unwrap_or(parent)
                                        .to_string_lossy()
                                        .to_string();

                                    if !rel_folder.is_empty() {
                                        obj.insert(
                                            "folder".to_string(),
                                            serde_json::json!(rel_folder),
                                        );
                                    }
                                }

                                // timestamps (best-effort)
                                if let Ok(metadata) = std::fs::metadata(&path) {
                                    if let Ok(created) = metadata.created() {
                                        obj.insert(
                                            "created_at".to_string(),
                                            serde_json::json!(
                                                chrono::DateTime::<chrono::Utc>::from(created)
                                            ),
                                        );
                                    }
                                    if let Ok(modified) = metadata.modified() {
                                        obj.insert(
                                            "updated_at".to_string(),
                                            serde_json::json!(
                                                chrono::DateTime::<chrono::Utc>::from(modified)
                                            ),
                                        );
                                    }
                                }

                                obj.insert(
                                    "path".to_string(),
                                    serde_json::json!(path.to_string_lossy()),
                                );
                            }

                            packs.push(meta);
                        }
                    }
                }
            }
        }
    }

    scan_dir(library_root, &mut packs, library_root);

    let out = serde_json::json!({ "packs": packs });
    Ok(serde_json::to_string_pretty(&out)?)
}

async fn execute_get_library_pack(
    args: serde_json::Value,
    app_state: &Arc<AppState>,
) -> anyhow::Result<String> {
    let params: GetLibraryPackParams = serde_json::from_value(args)?;
    let library_root = app_state.library_path.as_path();

    let pack_path = find_pack_by_id(library_root, &params.pack_id)
        .ok_or_else(|| anyhow::anyhow!("Pack not found: {}", params.pack_id))?;

    let content = std::fs::read_to_string(&pack_path)?;
    let meta = extract_frontmatter_json(&content);
    let folder = pack_path
        .parent()
        .and_then(|p| p.strip_prefix(library_root).ok())
        .map(|p| p.to_string_lossy().to_string());

    let out = serde_json::json!({
        "pack_id": params.pack_id,
        "path": pack_path.to_string_lossy(),
        "folder": folder,
        "meta": meta,
        "content": content
    });

    Ok(serde_json::to_string_pretty(&out)?)
}

async fn execute_query_codebase(
    args: serde_json::Value,
    app_state: &Arc<AppState>,
) -> anyhow::Result<String> {
    let params: QueryParams = serde_json::from_value(args)?;

    // Clamp limit
    let limit = params
        .limit
        .map(|l| if l <= 0 { 3usize } else { (l.min(20)) as usize })
        .unwrap_or(3usize);

    // Embed query
    let model_guard = app_state.embedding_model.read().await;
    let model = model_guard
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Embedding model is initializing"))?;
    let embedding = model.embed(&params.query)?;

    // Retrieve more than needed, then filter, then take top N
    let mut chunks = app_state
        .vector_store
        .search(embedding, Some(&params.query), limit.max(10))
        .await?;

    if let Some(excluded) = params.exclude_source_id.as_deref() {
        chunks.retain(|c| c.source_id != excluded);
    }

    let chunks = chunks.into_iter().take(limit).collect::<Vec<_>>();

    if chunks.is_empty() {
        return Ok("No relevant chunks found for your query.".to_string());
    }

    // Format as readable text with source + file/document id
    let mut output = String::new();
    output.push_str(&format!("Found {} matching chunks:\n\n", chunks.len()));
    for (i, c) in chunks.iter().enumerate() {
        output.push_str(&format!(
            "--- Chunk {} [{}] ---\nFile: {}\n\n{}\n\n",
            i + 1,
            c.source_id,
            c.document_id,
            c.content
        ));
    }

    Ok(output)
}

async fn execute_memory_search_semantic(
    args: serde_json::Value,
    app_state: &Arc<AppState>,
) -> anyhow::Result<String> {
    info!(
        "🔍 [MCP] memory.search_semantic called with args: {:?}",
        args
    );

    let params: MemorySearchSemanticParams = serde_json::from_value(args.clone()).map_err(|e| {
        error!(
            "❌ [MCP] Failed to parse memory.search_semantic params: {}",
            e
        );
        e
    })?;

    // Clamp limit
    let limit = params
        .limit
        .map(|l| {
            if l <= 0 {
                10usize
            } else {
                (l.min(50)) as usize
            }
        })
        .unwrap_or(10usize);

    info!(
        "=== MCP TOOL: memory.search_semantic ===\n  Query: {:?}\n  Limit: {}\n  Source ID: {:?}",
        params.query, limit, params.source_id
    );

    // Call shared implementation from memory_semantic module
    info!("📞 [MCP] Calling shared search_memories_semantic function...");
    let results = crate::handlers::memory_semantic::search_memories_semantic(
        app_state,
        &params.query,
        limit,
        params.source_id.as_deref(),
    )
    .await
    .map_err(|e| {
        error!("❌ [MCP] search_memories_semantic failed: {}", e);
        e
    })?;

    info!(
        "✅ [MCP] search_memories_semantic returned {} results",
        results.len()
    );

    // Return as JSON string
    let json_output = serde_json::json!({
        "results": results,
        "count": results.len()
    });

    info!(
        "=== MCP TOOL RESPONSE: memory.search_semantic ===\n  Results: {}",
        results.len()
    );

    let json_string = serde_json::to_string_pretty(&json_output)?;
    info!(
        "📤 [MCP] Returning JSON response ({} bytes)",
        json_string.len()
    );

    Ok(json_string)
}

async fn execute_memory_fetch_by_meta(
    args: serde_json::Value,
    app_state: &Arc<AppState>,
) -> anyhow::Result<String> {
    let params: MemoryFetchByMetaParams = serde_json::from_value(args)?;

    // 1. Try to find the file path in LanceDB using the metadata
    // Map 'id' or 'memory_id' to 'file_path' or check them as fallback
    let search_key = if params.key == "id" || params.key == "memory_id" {
        "file_path"
    } else {
        &params.key
    };

    let search_value = if (params.key == "id" || params.key == "memory_id")
        && !params.value.starts_with("memory/")
    {
        format!("memory/{}", params.value)
    } else {
        params.value.clone()
    };

    let file_path = app_state
        .internal_index_store
        .find_path_by_meta(search_key, &search_value)
        .await?;

    let mem = match file_path {
        Some(path) => {
            // Found via index! Read from path
            // The path in LanceDB is relative to .linggen/
            // We need to resolve it relative to the memory store's base dir (which is .linggen/memory)
            // Wait, memory_store.memory_dir() returns .linggen/memory
            // If path is "memory/my-design.md", then joining with memory_dir() might be wrong if memory_dir is already inside .linggen/memory

            // Let's check where memory_store.memory_dir() points.
            // Actually, we can just use the project root if we know it.
            // But execute_memory_fetch_by_meta doesn't have source_id.

            // Let's look at how we normally read memories.
            let full_path = app_state
                .memory_store
                .memory_dir()
                .join(path.strip_prefix("memory/").unwrap_or(&path));
            app_state.memory_store.read_from_path(&full_path)?
        }
        None => {
            // Fallback: Try directly by filename in the memory dir
            if params.key == "id" || params.key == "memory_id" {
                app_state.memory_store.read(&params.value)?
            } else {
                anyhow::bail!("Memory not found with {}='{}'", params.key, params.value);
            }
        }
    };

    let mut output = String::new();
    output.push_str(&format!("## Memory: {}\n", mem.meta.title));
    if !mem.meta.tags.is_empty() {
        output.push_str(&format!("Tags: {}\n", mem.meta.tags.join(", ")));
    }
    output.push_str("\n---\n\n");
    output.push_str(&mem.body);

    Ok(output)
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

    let response = call_enhance_internal(
        app_state,
        &params.query,
        params.strategy,
        params.source_id,
        params.exclude_source_id,
    )
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

    let response = call_enhance_internal(
        app_state,
        &params.query,
        params.strategy,
        params.source_id,
        params.exclude_source_id,
    )
    .await?;

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
        .get_setting("embedding_model_initialized")
        .unwrap_or(None)
        .map(|v| v == "true")
        .unwrap_or(false);

    // Check for error state
    let is_error = app_state
        .metadata_store
        .get_setting("embedding_model_initialized")
        .unwrap_or(None)
        .map(|v| v == "error")
        .unwrap_or(false);

    let (status, message, progress) = if is_error {
        let error_msg = app_state
            .metadata_store
            .get_setting("embedding_init_error")
            .unwrap_or(None)
            .unwrap_or_else(|| "Model initialization failed".to_string());
        ("error".to_string(), Some(error_msg), None)
    } else if !model_initialized {
        let progress = app_state
            .metadata_store
            .get_setting("embedding_init_progress")
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
    exclude_source_id: Option<String>,
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
        .enhance(
            query,
            &preferences,
            &profile,
            strategy,
            exclude_source_id.as_deref(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Enhancement failed: {}", e))?;

    Ok(result)
}
