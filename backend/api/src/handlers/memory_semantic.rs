use anyhow::Result;
use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;

use super::AppState;

#[derive(Deserialize)]
pub struct MemorySemanticSearchRequest {
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
    pub source_id: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct MemorySemanticSearchResult {
    pub source_id: String,
    pub file_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub snippet: String,
}

#[derive(Serialize)]
pub struct MemorySemanticSearchResponse {
    pub results: Vec<MemorySemanticSearchResult>,
    pub count: usize,
}

/// Core implementation for semantic memory search
/// This is called by both REST API and MCP tool for consistency
pub async fn search_memories_semantic(
    app_state: &Arc<AppState>,
    query: &str,
    limit: usize,
    source_id: Option<&str>,
) -> Result<Vec<MemorySemanticSearchResult>> {
    info!("🔍 [MemSemantic] Starting search - query: {:?}, limit: {}, source_id: {:?}", query, limit, source_id);
    
    // 1. Embed query
    info!("📊 [MemSemantic] Acquiring embedding model lock...");
    let model_guard = app_state.embedding_model.read().await;
    let model = model_guard
        .as_ref()
        .ok_or_else(|| {
            let err = anyhow::anyhow!("Embedding model is initializing");
            info!("❌ [MemSemantic] Embedding model not ready: {}", err);
            err
        })?;

    info!("✅ [MemSemantic] Embedding model acquired, generating embedding for query...");
    let embedding = model.embed(query)
        .map_err(|e| {
            info!("❌ [MemSemantic] Failed to generate embedding: {}", e);
            e
        })?;
    info!("✅ [MemSemantic] Embedding generated (dim: {})", embedding.len());
    drop(model_guard);

    // 2. Search internal index (get more results, then filter)
    info!("🔎 [MemSemantic] Searching internal index (limit: {})...", limit * 2);
    let chunks = app_state
        .internal_index_store
        .search(embedding, Some(query), limit * 2)
        .await
        .map_err(|e| {
            info!("❌ [MemSemantic] Internal index search failed: {}", e);
            e
        })?;
    
    info!("✅ [MemSemantic] Internal index returned {} chunks", chunks.len());

    // 3. Filter to memories only and optionally by source_id
    info!("🔧 [MemSemantic] Filtering chunks (kind='memory', source_id={:?})...", source_id);
    
    let mut filtered_count = 0;
    let mut memory_count = 0;
    
    let results: Vec<MemorySemanticSearchResult> = chunks
        .into_iter()
        .filter(|chunk| {
            filtered_count += 1;
            
            // Filter to memories only
            if let Some(kind) = chunk.metadata.get("kind").and_then(|v| v.as_str()) {
                if kind != "memory" {
                    info!("🔧 [MemSemantic] Chunk {} filtered out: kind={}", filtered_count, kind);
                    return false;
                }
                memory_count += 1;
            } else {
                info!("🔧 [MemSemantic] Chunk {} filtered out: no kind metadata", filtered_count);
                return false;
            }

            // Filter by source_id if provided
            if let Some(filter_source) = source_id {
                if chunk.source_id != filter_source {
                    info!("🔧 [MemSemantic] Chunk {} filtered out: source_id mismatch (got {}, want {})", 
                        filtered_count, chunk.source_id, filter_source);
                    return false;
                }
            }

            true
        })
        .take(limit)
        .map(|chunk| {
            let file_path = chunk
                .metadata
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = chunk
                .metadata
                .get("title")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            info!("✅ [MemSemantic] Including result: source={}, file={}, title={:?}", 
                chunk.source_id, file_path, title);

            MemorySemanticSearchResult {
                source_id: chunk.source_id.clone(),
                file_path,
                title,
                snippet: chunk.content.chars().take(300).collect::<String>(),
            }
        })
        .collect();

    info!("🎯 [MemSemantic] Final results: {} out of {} chunks ({} were memories)", 
        results.len(), filtered_count, memory_count);

    Ok(results)
}

/// REST API handler for semantic memory search
pub async fn search_semantic(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MemorySemanticSearchRequest>,
) -> std::result::Result<Json<MemorySemanticSearchResponse>, (StatusCode, String)> {
    // Clamp limit
    let limit = req.limit.unwrap_or(10).min(50).max(1);

    info!(
        "REST API: Memory semantic search: query={:?}, limit={}, source_id={:?}",
        req.query, limit, req.source_id
    );

    // Call shared implementation
    let results = search_memories_semantic(&state, &req.query, limit, req.source_id.as_deref())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let count = results.len();

    Ok(Json(MemorySemanticSearchResponse { results, count }))
}
