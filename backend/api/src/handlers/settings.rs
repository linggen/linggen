use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::handlers::index::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettingsDto {
    pub intent_detection_enabled: bool,
}

impl From<storage::metadata::AppSettings> for AppSettingsDto {
    fn from(s: storage::metadata::AppSettings) -> Self {
        Self {
            intent_detection_enabled: s.intent_detection_enabled,
        }
    }
}

impl From<AppSettingsDto> for storage::metadata::AppSettings {
    fn from(dto: AppSettingsDto) -> Self {
        Self {
            intent_detection_enabled: dto.intent_detection_enabled,
        }
    }
}

/// Get application-wide settings
pub async fn get_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<AppSettingsDto>, (StatusCode, String)> {
    let settings = state
        .metadata_store
        .get_app_settings()
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to load settings: {}", e),
            )
        })?;

    Ok(Json(settings.into()))
}

/// Update application-wide settings
pub async fn update_settings(
    State(state): State<Arc<AppState>>,
    Json(dto): Json<AppSettingsDto>,
) -> Result<StatusCode, (StatusCode, String)> {
    let settings: storage::metadata::AppSettings = dto.into();

    state
        .metadata_store
        .update_app_settings(&settings)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to update settings: {}", e),
            )
        })?;

    Ok(StatusCode::OK)
}


