use axum::{extract::State, http::StatusCode, Json};
use linggen_core::IndexingJob;
use serde::{Deserialize, Serialize};
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
        .get_jobs(Some(200)) // Last 200 jobs
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(ListJobsResponse { jobs }))
}

#[derive(Deserialize)]
pub struct CancelJobRequest {
    pub job_id: String,
}

#[derive(Serialize)]
pub struct CancelJobResponse {
    pub success: bool,
    pub job_id: String,
}

pub async fn cancel_job(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CancelJobRequest>,
) -> Result<Json<CancelJobResponse>, (StatusCode, String)> {
    tracing::info!("📛 Cancel request received for job: {}", req.job_id);

    // Set the cancellation flag
    state.cancellation_flags.insert(req.job_id.clone(), true);
    tracing::info!("✓ Cancellation flag set for job: {}", req.job_id);

    Ok(Json(CancelJobResponse {
        success: true,
        job_id: req.job_id,
    }))
}
