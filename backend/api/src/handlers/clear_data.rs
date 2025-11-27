use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;
use std::sync::Arc;

use super::index::AppState;

#[derive(Serialize)]
pub struct ClearDataResponse {
    pub success: bool,
    pub message: String,
}

pub async fn clear_all_data(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ClearDataResponse>, (StatusCode, String)> {
    let mut errors = Vec::new();

    // Clear all chunks from LanceDB
    if let Err(e) = state.vector_store.clear_all().await {
        errors.push(format!("Failed to clear LanceDB: {}", e));
    }

    // Clear all metadata from Redb
    if let Err(e) = state.metadata_store.clear_all() {
        errors.push(format!("Failed to clear metadata: {}", e));
    }

    if !errors.is_empty() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Errors during cleanup: {}", errors.join("; ")),
        ));
    }

    Ok(Json(ClearDataResponse {
        success: true,
        message: "All data cleared successfully. Refresh the page to see the changes.".to_string(),
    }))
}
