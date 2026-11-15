//! What a skill's fetched files stand at: `GET /api/assets/{skill}` answers
//! `{packs: [{pack, files, bytes, held_files, held_bytes, failing}], files,
//! bytes, held_files, held_bytes}` — a skill without a manifest is a 404.
//! A page may read it; nothing requires it to.

use crate::server::ServerState;
use axum::{extract::Path, extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;
use std::sync::Arc;

pub(crate) async fn get_assets(
    State(state): State<Arc<ServerState>>,
    Path(skill): Path<String>,
) -> impl IntoResponse {
    match state.assets.report(&skill) {
        Some(report) => (StatusCode::OK, Json(json!(report))),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no asset manifest" })),
        ),
    }
}
