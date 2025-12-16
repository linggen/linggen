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

    Ok(Json(MemoryCreateResponse {
        id: mem.meta.id,
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

    Ok(Json(to_read(mem)))
}

pub async fn delete_memory(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MemoryDeleteRequest>,
) -> Result<Json<()>, (StatusCode, String)> {
    state
        .memory_store
        .delete(&req.id)
        .map_err(internal_err)?;
    Ok(Json(()))
}

fn to_summary(mem: MemoryEntry) -> MemorySummary {
    let snippet = mem.body.lines().take(3).collect::<Vec<_>>().join("\n");
    MemorySummary {
        id: mem.meta.id,
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
    MemoryReadResponse {
        id: mem.meta.id,
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

