use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use storage::UserPreferences;

use super::index::AppState;

#[derive(Serialize)]
pub struct GetPreferencesResponse {
    pub preferences: UserPreferences,
}

#[derive(Deserialize)]
pub struct UpdatePreferencesRequest {
    pub preferences: UserPreferences,
}

/// Get user preferences
pub async fn get_preferences(
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<GetPreferencesResponse>, (StatusCode, String)> {
    let prefs = state.metadata_store.get_preferences().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get preferences: {}", e),
        )
    })?;

    Ok(Json(GetPreferencesResponse { preferences: prefs }))
}

/// Update user preferences
pub async fn update_preferences(
    State(state): State<std::sync::Arc<AppState>>,
    Json(req): Json<UpdatePreferencesRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .metadata_store
        .update_preferences(&req.preferences)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to update preferences: {}", e),
            )
        })?;

    Ok(StatusCode::OK)
}

