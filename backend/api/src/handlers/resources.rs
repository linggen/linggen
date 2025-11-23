use axum::{extract::State, http::StatusCode, Json};
use rememberme_core::{IndexingJob, SourceConfig, SourceType};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use super::index::AppState;

#[derive(Deserialize)]
pub struct AddResourceRequest {
    pub name: String,
    pub resource_type: ResourceType,
    pub path: String, // URL for git/web, file path for local
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceType {
    Git,
    Local,
    Web,
}

impl From<ResourceType> for SourceType {
    fn from(rt: ResourceType) -> Self {
        match rt {
            ResourceType::Git => SourceType::Git,
            ResourceType::Local => SourceType::Local,
            ResourceType::Web => SourceType::Web,
        }
    }
}

impl From<SourceType> for ResourceType {
    fn from(st: SourceType) -> Self {
        match st {
            SourceType::Git => ResourceType::Git,
            SourceType::Local => ResourceType::Local,
            SourceType::Web => ResourceType::Web,
        }
    }
}

#[derive(Serialize)]
pub struct AddResourceResponse {
    pub id: String,
    pub name: String,
    pub resource_type: ResourceType,
    pub path: String,
    pub enabled: bool,
}

#[derive(Serialize)]
pub struct ListResourcesResponse {
    pub resources: Vec<ResourceInfo>,
}

#[derive(Serialize)]
pub struct ResourceInfo {
    pub id: String,
    pub name: String,
    pub resource_type: ResourceType,
    pub path: String,
    pub enabled: bool,
    pub latest_job: Option<IndexingJob>,
}

#[derive(Deserialize)]
pub struct RemoveResourceRequest {
    pub id: String,
}

#[derive(Serialize)]
pub struct RemoveResourceResponse {
    pub success: bool,
    pub id: String,
}

pub async fn add_resource(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AddResourceRequest>,
) -> Result<Json<AddResourceResponse>, (StatusCode, String)> {
    let source = SourceConfig {
        id: Uuid::new_v4().to_string(),
        name: req.name.clone(),
        source_type: req.resource_type.into(),
        path: req.path.clone(),
        enabled: true,
    };

    state
        .metadata_store
        .add_source(&source)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(AddResourceResponse {
        id: source.id,
        name: source.name,
        resource_type: source.source_type.into(),
        path: source.path,
        enabled: source.enabled,
    }))
}

pub async fn list_resources(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ListResourcesResponse>, (StatusCode, String)> {
    let sources = state
        .metadata_store
        .get_sources()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut resources = Vec::new();

    for s in sources {
        let latest_job = state
            .metadata_store
            .get_latest_job_for_source(&s.id)
            .unwrap_or(None); // Log error in real app

        resources.push(ResourceInfo {
            id: s.id,
            name: s.name,
            resource_type: s.source_type.into(),
            path: s.path,
            enabled: s.enabled,
            latest_job,
        });
    }

    Ok(Json(ListResourcesResponse { resources }))
}

pub async fn remove_resource(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RemoveResourceRequest>,
) -> Result<Json<RemoveResourceResponse>, (StatusCode, String)> {
    // Delete from metadata store (redb)
    state
        .metadata_store
        .remove_source(&req.id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Delete all chunks from vector store (LanceDB) - hard delete
    state
        .vector_store
        .delete_by_source(&req.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(RemoveResourceResponse {
        success: true,
        id: req.id,
    }))
}
