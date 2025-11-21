use axum::{extract::State, http::StatusCode, Json};
use rememberme_core::IndexingJob;
use serde::Serialize;
use std::sync::Arc;

use super::index::AppState;

#[derive(Serialize)]
pub struct ListJobsResponse {
    pub jobs: Vec<IndexingJob>,
}

pub async fn list_jobs(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ListJobsResponse>, (StatusCode, String)> {
    let jobs = state
        .metadata_store
        .get_jobs(Some(50)) // Last 50 jobs
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(ListJobsResponse { jobs }))
}
