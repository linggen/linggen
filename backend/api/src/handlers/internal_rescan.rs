//! Internal index rescan handler

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use std::sync::Arc;
use tracing::info;

use super::index::AppState;

#[derive(Serialize)]
pub struct RescanResponse {
    pub files_indexed: usize,
    pub files_failed: usize,
}

/// Rescan and reindex all internal files (memories/prompts) for a source
/// This is useful for out-of-band edits or initial population
pub async fn rescan_internal_index(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
) -> Result<Json<RescanResponse>, (StatusCode, String)> {
    info!("Rescanning internal index for source: {}", source_id);
    
    // Get source
    let source = state
        .metadata_store
        .get_source(&source_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("Source not found: {}", source_id),
        ))?;

    // Rescan
    let (files_indexed, files_failed) = crate::internal_indexer::rescan_internal_files(
        &state.internal_index_store,
        &state.embedding_model,
        &state.chunker,
        &source_id,
        &source.path,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to rescan internal files: {}", e),
        )
    })?;

    info!(
        "Rescan complete: {} files indexed, {} failed",
        files_indexed, files_failed
    );

    Ok(Json(RescanResponse {
        files_indexed,
        files_failed,
    }))
}
