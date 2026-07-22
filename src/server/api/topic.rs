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
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::server::{ServerEvent, ServerState};

/// Publish one control message to the user's devices.
pub(crate) fn publish_topic(
    state: &Arc<ServerState>,
    topic: &str,
    op: &str,
    payload: serde_json::Value,
) {
    let _ = state.events_tx.send(ServerEvent::DeviceTopic {
        topic: topic.to_string(),
        op: op.to_string(),
        payload,
        from_device: None,
    });
}

/// Watch a directory and announce changes on a topic, so devices stop polling
/// for them. Debounced, because one logical change (an album copied in, a scan
/// rewriting its state file) fires a burst of filesystem events. `filter` picks
/// which paths matter; `None` means every change counts.
pub(crate) fn watch_dir(
    state: Arc<ServerState>,
    dir: PathBuf,
    topic: &'static str,
    op: &'static str,
    debounce: Duration,
    filter: Option<fn(&std::path::Path) -> bool>,
) {
    if !dir.is_dir() {
        return;
    }
    tokio::spawn(async move {
        use notify::Watcher;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut watcher = match notify::recommended_watcher(
            move |res: notify::Result<notify::Event>| {
                let Ok(ev) = res else { return };
                if !(ev.kind.is_create() || ev.kind.is_remove() || ev.kind.is_modify()) {
                    return;
                }
                let relevant = match filter {
                    Some(f) => ev.paths.iter().any(|p| f(p)),
                    None => true,
                };
                if relevant {
                    let _ = tx.send(());
                }
            },
        ) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("[topic] watcher for {topic}/{op} unavailable: {e}");
                return;
            }
        };
        if let Err(e) = watcher.watch(&dir, notify::RecursiveMode::Recursive) {
            tracing::warn!("[topic] watching {} failed: {e}", dir.display());
            return;
        }
        tracing::info!("[topic] watching {} → {topic}/{op}", dir.display());
        while rx.recv().await.is_some() {
            // Drain the burst before announcing once.
            while tokio::time::timeout(debounce, rx.recv())
                .await
                .is_ok_and(|v| v.is_some())
            {}
            publish_topic(&state, topic, op, serde_json::Value::Null);
            tracing::info!("[topic] {topic}/{op} — devices notified");
        }
    });
}

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
