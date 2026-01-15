use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::handlers::AppState;
use crate::memory::{MemoryCitation, MemoryEntry, MemoryScope};

#[derive(Deserialize)]
pub struct MemorySearchRequest {
    pub query: Option<String>,
    pub tags: Option<Vec<String>>,
    pub source_id: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Serialize)]
pub struct MemorySearchResponse {
    pub results: Vec<MemorySummary>,
}

#[derive(Serialize)]
pub struct MemorySummary {
    pub id: String,
    pub title: String,
    pub tags: Vec<String>,
    pub scope: MemoryScope,
    pub confidence: Option<f64>,
    pub updated_at: String,
    pub created_at: String,
    pub snippet: String,
}

#[derive(Serialize)]
pub struct MemoryReadResponse {
    pub id: String,
    pub title: String,
    pub tags: Vec<String>,
    pub scope: MemoryScope,
    pub confidence: Option<f64>,
    pub citations: Vec<MemoryCitation>,
    pub created_at: String,
    pub updated_at: String,
    pub body: String,
}

#[derive(Deserialize)]
pub struct MemoryCreateRequest {
    pub title: String,
    pub body: String,
    pub tags: Option<Vec<String>>,
    pub scope: Option<MemoryScope>,
    pub citations: Option<Vec<MemoryCitation>>,
    pub confidence: Option<f64>,
}

#[derive(Serialize)]
pub struct MemoryCreateResponse {
    pub id: String,
    pub path: String,
}

#[derive(Deserialize)]
pub struct MemoryUpdateRequest {
    pub id: String,
    pub title: Option<String>,
    pub body: Option<String>,
    pub tags: Option<Vec<String>>,
    pub scope: Option<MemoryScope>,
    pub citations: Option<Vec<MemoryCitation>>,
    pub confidence: Option<f64>,
}

#[derive(Deserialize)]
pub struct MemoryDeleteRequest {
    pub id: String,
}

pub async fn list_memories(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MemorySearchRequest>,
) -> Result<Json<MemorySearchResponse>, (StatusCode, String)> {
    let results = state
        .memory_store
        .search(
            req.query.as_deref(),
            req.tags.as_ref().map(|v| v.as_slice()),
            req.source_id.as_deref(),
            req.limit,
        )
        .map_err(internal_err)?;

    Ok(Json(MemorySearchResponse {
        results: results.into_iter().map(to_summary).collect(),
    }))
}

pub async fn read_memory(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<MemoryDeleteRequest>,
) -> Result<Json<MemoryReadResponse>, (StatusCode, String)> {
    let mem = state
        .memory_store
        .read(&params.id)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(Json(to_read(mem)))
}

pub async fn create_memory(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MemoryCreateRequest>,
) -> Result<Json<MemoryCreateResponse>, (StatusCode, String)> {
    let mem = state
        .memory_store
        .create(
            &req.title,
            &req.body,
            req.tags.unwrap_or_default(),
            req.scope.unwrap_or_default(),
            req.citations.unwrap_or_default(),
            req.confidence,
        )
        .map_err(internal_err)?;

    // Index in background
    let internal_index_store = state.internal_index_store.clone();
    let embedding_model = state.embedding_model.clone();
    let chunker = state.chunker.clone();
    // For global memories, we can use a "global" source_id or empty
    let source_id = "global".to_string();
    let file_path = mem.path.clone();
    let relative_path = format!("memory/{}", mem.path.file_name().unwrap().to_string_lossy());

    tokio::spawn(async move {
        if let Err(e) = crate::internal_indexer::index_internal_file(
            &internal_index_store,
            &embedding_model,
            &chunker,
            &source_id,
            &file_path,
            &relative_path,
        )
        .await
        {
            tracing::warn!("Failed to index global memory: {}", e);
        }
    });

    Ok(Json(MemoryCreateResponse {
        id: mem
            .meta
            .id
            .clone()
            .unwrap_or_else(|| mem.path.file_stem().unwrap().to_string_lossy().to_string()),
        path: mem.path.display().to_string(),
    }))
}

pub async fn update_memory(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MemoryUpdateRequest>,
) -> Result<Json<MemoryReadResponse>, (StatusCode, String)> {
    let mem = state
        .memory_store
        .update(
            &req.id,
            req.title.as_deref(),
            req.body.as_deref(),
            req.tags.clone(),
            req.scope.clone(),
            req.citations.clone(),
            Some(req.confidence),
        )
        .map_err(internal_err)?;

    // Index in background
    let internal_index_store = state.internal_index_store.clone();
    let embedding_model = state.embedding_model.clone();
    let chunker = state.chunker.clone();
    let source_id = "global".to_string();
    let file_path = mem.path.clone();
    let relative_path = format!("memory/{}", mem.path.file_name().unwrap().to_string_lossy());

    tokio::spawn(async move {
        if let Err(e) = crate::internal_indexer::index_internal_file(
            &internal_index_store,
            &embedding_model,
            &chunker,
            &source_id,
            &file_path,
            &relative_path,
        )
        .await
        {
            tracing::warn!("Failed to index global memory: {}", e);
        }
    });

    Ok(Json(to_read(mem)))
}

pub async fn delete_memory(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MemoryDeleteRequest>,
) -> Result<Json<()>, (StatusCode, String)> {
    // Get info first to know relative path for unindexing
    let relative_path = if let Ok(mem) = state.memory_store.read(&req.id) {
        Some(format!(
            "memory/{}",
            mem.path.file_name().unwrap().to_string_lossy()
        ))
    } else {
        None
    };

    state.memory_store.delete(&req.id).map_err(internal_err)?;

    if let Some(rel_path) = relative_path {
        let internal_index_store = state.internal_index_store.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::internal_indexer::remove_internal_file(
                &internal_index_store,
                "global",
                "memory",
                &rel_path,
            )
            .await
            {
                tracing::warn!("Failed to unindex global memory: {}", e);
            }
        });
    }

    Ok(Json(()))
}

fn to_summary(mem: MemoryEntry) -> MemorySummary {
    let snippet = mem.body.lines().take(3).collect::<Vec<_>>().join("\n");
    let id = mem
        .meta
        .id
        .clone()
        .unwrap_or_else(|| mem.path.file_stem().unwrap().to_string_lossy().to_string());
    MemorySummary {
        id,
        title: mem.meta.title,
        tags: mem.meta.tags,
        scope: mem.meta.scope,
        confidence: mem.meta.confidence,
        created_at: mem.meta.created_at.to_rfc3339(),
        updated_at: mem.meta.updated_at.to_rfc3339(),
        snippet,
    }
}

fn to_read(mem: MemoryEntry) -> MemoryReadResponse {
    let id = mem
        .meta
        .id
        .clone()
        .unwrap_or_else(|| mem.path.file_stem().unwrap().to_string_lossy().to_string());
    MemoryReadResponse {
        id,
        title: mem.meta.title,
        tags: mem.meta.tags,
        scope: mem.meta.scope,
        confidence: mem.meta.confidence,
        citations: mem.meta.citations,
        created_at: mem.meta.created_at.to_rfc3339(),
        updated_at: mem.meta.updated_at.to_rfc3339(),
        body: mem.body,
    }
}

fn internal_err<E: std::fmt::Display>(err: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
