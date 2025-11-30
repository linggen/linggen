//! MCP Tool Definitions and Execution
//!
//! Defines the available MCP tools and their execution logic.

use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use tracing::info;

use crate::api_client::LinggenApiClient;

const DEFAULT_LIMIT: usize = 5;

/// Get MCP tool definitions for tools/list response
pub fn get_tool_definitions() -> Vec<serde_json::Value> {
    vec![
        json!({
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
        json!({
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
        json!({
            "name": "list_sources",
            "description": "List all indexed sources/projects in Linggen. Shows what codebases are available for searching, including their stats (file count, chunk count, size).",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        json!({
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

/// Execute a tool by name with given arguments
pub async fn execute_tool(
    name: &str,
    args: serde_json::Value,
    api_client: &LinggenApiClient,
) -> Result<String> {
    match name {
        "search_codebase" => execute_search_codebase(args, api_client).await,
        "enhance_prompt" => execute_enhance_prompt(args, api_client).await,
        "list_sources" => execute_list_sources(api_client).await,
        "get_status" => execute_get_status(api_client).await,
        _ => anyhow::bail!("Unknown tool: {}", name),
    }
}

// ============================================================================
// Tool Parameter Types
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

// ============================================================================
// Tool Implementations
// ============================================================================

async fn execute_search_codebase(
    args: serde_json::Value,
    api_client: &LinggenApiClient,
) -> Result<String> {
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
        "=== TOOL CALL: search_codebase ===\n  Query: {:?}\n  Limit: {}\n  Strategy: {:?}\n  Source ID: {:?}",
        params.query, limit, params.strategy, params.source_id
    );

    let response = api_client
        .enhance(&params.query, params.strategy, params.source_id)
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

    Ok(output)
}

async fn execute_enhance_prompt(
    args: serde_json::Value,
    api_client: &LinggenApiClient,
) -> Result<String> {
    let params: EnhanceParams = serde_json::from_value(args)?;

    info!(
        "=== TOOL CALL: enhance_prompt ===\n  Query: {:?}\n  Strategy: {:?}\n  Source ID: {:?}",
        params.query, params.strategy, params.source_id
    );

    let response = api_client
        .enhance(&params.query, params.strategy, params.source_id)
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

    Ok(output)
}

async fn execute_list_sources(api_client: &LinggenApiClient) -> Result<String> {
    info!("=== TOOL CALL: list_sources ===");

    let response = api_client.list_resources().await?;

    let mut output = String::new();

    if response.resources.is_empty() {
        output.push_str("No sources indexed in Linggen.\n\n");
        output.push_str("To add a source, use the Linggen web UI.");
    } else {
        output.push_str(&format!(
            "## Indexed Sources ({} total)\n\n",
            response.resources.len()
        ));

        for resource in &response.resources {
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
        response.resources.len(),
        output.len()
    );

    Ok(output)
}

async fn execute_get_status(api_client: &LinggenApiClient) -> Result<String> {
    info!("=== TOOL CALL: get_status ===");

    let response = api_client.get_status().await?;

    let mut output = String::new();
    output.push_str("## Linggen Status\n\n");
    output.push_str(&format!("**Status:** {}\n", response.status));

    if let Some(msg) = &response.message {
        output.push_str(&format!("**Message:** {}\n", msg));
    }

    if let Some(progress) = &response.progress {
        output.push_str(&format!("**Progress:** {}\n", progress));
    }

    info!(
        "=== TOOL RESPONSE: get_status ===\n  Status: {}\n  Output length: {} chars",
        response.status,
        output.len()
    );

    Ok(output)
}
