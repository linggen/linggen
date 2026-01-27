use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::info;

use super::AppState;

/// Gracefully shutdown the server
pub async fn shutdown_server(State(_state): State<Arc<AppState>>) -> (StatusCode, Json<Value>) {
    info!("Received shutdown request via API");

    // Spawn a task to shutdown after a brief delay to allow the response to be sent
    tokio::spawn(async {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        info!("Shutting down server...");
        std::process::exit(0);
    });

    (
        StatusCode::OK,
        Json(json!({
            "message": "Server is shutting down"
        })),
    )
}
