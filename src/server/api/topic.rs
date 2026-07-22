//! Device topics over HTTP — the door Mac-side components use to push to the
//! user's phones. The daemon is the hub: whatever is published here lands on
//! every connected surface's control channel as a `device_topic` event, the
//! same messages phones publish over the data channel.
//!
//! Skills, scripts, and app shells have no WebRTC peer of their own, so this
//! is how mac-shifu announces new media verdicts, CFO announces an import, and
//! so on. Gated like every other route: loopback (local skills) passes, LAN
//! callers need their device token.

use axum::{extract::State, response::IntoResponse, Json};
use serde::Deserialize;
use std::sync::Arc;

use crate::server::{ServerEvent, ServerState};

#[derive(Deserialize)]
pub(crate) struct PublishBody {
    topic: String,
    op: String,
    #[serde(default)]
    payload: serde_json::Value,
    /// Optional publisher id — a device echoes nothing back to itself.
    #[serde(default)]
    from_device: Option<String>,
}

/// POST /api/topic/publish — relay one control message to the user's devices.
pub(crate) async fn publish(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<PublishBody>,
) -> impl IntoResponse {
    if body.topic.is_empty() || body.op.is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "topic and op are required" })),
        );
    }
    tracing::info!("[topic] {}/{} published over HTTP", body.topic, body.op);
    let _ = state.events_tx.send(ServerEvent::DeviceTopic {
        topic: body.topic,
        op: body.op,
        payload: body.payload,
        from_device: body.from_device,
    });
    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({ "ok": true })),
    )
}
