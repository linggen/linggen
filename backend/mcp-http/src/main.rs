//! Linggen MCP Server over HTTP/SSE
//!
//! This server exposes MCP tools via HTTP endpoints so Cursor users can connect
//! without installing a local binary. It uses:
//! - `GET /mcp/sse` for Server-Sent Events (server -> client streaming)
//! - `POST /mcp/message` for client -> server MCP messages
//!
//! The server acts as a gateway to the Linggen backend API.

use anyhow::Result;
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use dashmap::DashMap;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, info, warn};

mod api_client;
mod tools;

use api_client::LinggenApiClient;

// ============================================================================
// Configuration
// ============================================================================

const DEFAULT_PORT: u16 = 3001;
const REQUEST_TIMEOUT_SECS: u64 = 30;
const SSE_CHANNEL_SIZE: usize = 100;
const SSE_KEEPALIVE_SECS: u64 = 15;

// ============================================================================
// MCP Protocol Types
// ============================================================================

/// MCP JSON-RPC request envelope
#[derive(Debug, Deserialize)]
struct McpRequest {
    jsonrpc: String,
    id: Option<serde_json::Value>,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

/// MCP JSON-RPC response envelope
#[derive(Debug, Serialize)]
struct McpResponse {
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
#[allow(dead_code)]
const MCP_PARSE_ERROR: i32 = -32700;
const MCP_INVALID_REQUEST: i32 = -32600;
const MCP_METHOD_NOT_FOUND: i32 = -32601;
const MCP_INTERNAL_ERROR: i32 = -32603;

// ============================================================================
// Server State
// ============================================================================

/// Shared application state
struct AppState {
    /// Linggen API client
    api_client: LinggenApiClient,
    /// Active SSE client connections: session_id -> sender
    clients: DashMap<String, mpsc::Sender<Result<Event, Infallible>>>,
    /// Optional access token for basic auth (if set, requests must include it)
    access_token: Option<String>,
    /// Connection counter for metrics
    connection_count: std::sync::atomic::AtomicU64,
    /// Request counter for metrics
    request_count: std::sync::atomic::AtomicU64,
}

impl AppState {
    fn new(api_url: String, access_token: Option<String>) -> Self {
        Self {
            api_client: LinggenApiClient::new(api_url, REQUEST_TIMEOUT_SECS),
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

    fn get_stats(&self) -> (u64, u64) {
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

// ============================================================================
// SSE Query Parameters
// ============================================================================

#[derive(Debug, Deserialize)]
struct SseQuery {
    /// Optional session ID; if not provided, server generates one
    session_id: Option<String>,
}

// ============================================================================
// Handlers
// ============================================================================

/// GET /mcp/sse - Establish SSE connection
async fn sse_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<SseQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, &'static str)> {
    // Validate access token if configured
    if !state.validate_token(&headers) {
        warn!("SSE connection rejected: invalid or missing access token");
        return Err((StatusCode::UNAUTHORIZED, "Invalid or missing access token"));
    }

    // Generate or use provided session ID
    let session_id = query
        .session_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let conn_num = state.increment_connections();
    info!(
        "SSE connection established: session_id={}, active_connections={}",
        session_id, conn_num
    );

    // Create channel for this client
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(SSE_CHANNEL_SIZE);

    // Store sender in state
    state.clients.insert(session_id.clone(), tx.clone());

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
    let state_clone = state.clone();
    let session_id_for_cleanup = session_id.clone();
    tokio::spawn(async move {
        // Wait for the channel to close (receiver dropped)
        tokio::time::sleep(Duration::from_secs(3600)).await;
        state_clone.clients.remove(&session_id_for_cleanup);
        let conn_num = state_clone.decrement_connections();
        info!(
            "SSE connection cleaned up: session_id={}, active_connections={}",
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
async fn message_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<McpRequest>,
) -> impl IntoResponse {
    // Validate access token if configured
    if !state.validate_token(&headers) {
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

    let req_num = state.increment_requests();
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
        "initialize" => handle_initialize(request.id.clone(), &request.params).await,
        "initialized" => handle_initialized(request.id.clone()).await,
        "tools/list" => handle_tools_list(request.id.clone()).await,
        "tools/call" => {
            handle_tools_call(request.id.clone(), &request.params, &state.api_client).await
        }
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
async fn handle_initialize(
    id: Option<serde_json::Value>,
    _params: &serde_json::Value,
) -> McpResponse {
    info!("Handling initialize request");

    let result = serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "linggen-mcp-http",
            "version": env!("CARGO_PKG_VERSION")
        }
    });

    McpResponse::success(id, result)
}

/// Handle initialized notification
async fn handle_initialized(id: Option<serde_json::Value>) -> McpResponse {
    info!("Client initialized");
    McpResponse::success(id, serde_json::json!({}))
}

/// Handle tools/list request
async fn handle_tools_list(id: Option<serde_json::Value>) -> McpResponse {
    info!("Handling tools/list request");

    let tools = tools::get_tool_definitions();
    let result = serde_json::json!({
        "tools": tools
    });

    McpResponse::success(id, result)
}

/// Handle tools/call request
async fn handle_tools_call(
    id: Option<serde_json::Value>,
    params: &serde_json::Value,
    api_client: &LinggenApiClient,
) -> McpResponse {
    let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let tool_args = params.get("arguments").cloned().unwrap_or_default();

    info!("Handling tools/call: name={}", tool_name);

    match tools::execute_tool(tool_name, tool_args, api_client).await {
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
            error!("Tool execution failed: {}", e);
            McpResponse::error(id, MCP_INTERNAL_ERROR, e.to_string())
        }
    }
}

/// Handle ping request
async fn handle_ping(id: Option<serde_json::Value>) -> McpResponse {
    McpResponse::success(id, serde_json::json!({}))
}

/// Health check endpoint
async fn health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let (connections, requests) = state.get_stats();
    let response = serde_json::json!({
        "status": "ok",
        "active_connections": connections,
        "total_requests": requests
    });
    (StatusCode::OK, Json(response))
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(true)
        .with_line_number(true)
        .with_level(true)
        .compact()
        .init();

    info!("Starting Linggen MCP HTTP/SSE Server...");

    // Get configuration from environment
    let api_url =
        std::env::var("LINGGEN_API_URL").unwrap_or_else(|_| "http://localhost:3000".to_string());
    let port: u16 = std::env::var("MCP_HTTP_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let access_token = std::env::var("LINGGEN_ACCESS_TOKEN").ok();

    info!("Linggen API URL: {}", api_url);
    info!("MCP HTTP port: {}", port);
    if access_token.is_some() {
        info!("Access token: configured (requests will require authentication)");
    } else {
        info!("Access token: not configured (all requests allowed)");
    }

    // Create shared state
    let state = Arc::new(AppState::new(api_url, access_token));

    // Configure CORS for browser/Cursor access
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers(tower_http::cors::Any);

    // Build router
    let app = Router::new()
        .route("/mcp/sse", get(sse_handler))
        .route("/mcp/message", post(message_handler))
        .route("/health", get(health_handler))
        .with_state(state)
        .layer(cors);

    // Run server
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
