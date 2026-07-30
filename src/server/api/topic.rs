//! Device topics over HTTP — the door Mac-side components use to push to the
//! user's phones. The daemon is the hub: whatever is published here lands on
//! every connected surface's control channel as a `device_topic` event, the
//! same messages phones publish over the data channel.
//!
//! Skills, scripts, and app shells have no WebRTC peer of their own, so this
//! is how apple-shifu announces new media verdicts, CFO announces an import, and
//! so on. Gated like every other route: loopback (local skills) passes, LAN
//! callers need their device token.
//!
//! # Retained topics
//!
//! A plain publish is fire-and-forget: it reaches whoever is connected at that
//! instant and is then gone. That works for "something changed, go look", and
//! not at all for "here is what I am" — a surface that connects a second later
//! never learns it, and the surfaces with no peer never learn it at all.
//!
//! So a publisher may mark a message `retain`. The daemon keeps **the last one
//! per topic and op** — a single value, overwritten each time, never a log —
//! and serves it from `GET /api/topic/latest`. That is the difference between
//! a retained topic and a queue: a queue holds every item because each one is
//! work to be done exactly once, while a retained topic holds only the newest
//! because a fresher reading makes the older one worthless.
//!
//! The payload stays opaque here, as it is for a live publish. What is in it,
//! and how stale is too stale, are contracts between the surfaces.

use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Json,
};
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
    topic: String,
    op: String,
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
            publish_topic(&state, &topic, &op, serde_json::Value::Null);
            tracing::info!("[topic] {topic}/{op} — devices notified");
        }
    });
}

/// Where a retained `topic`/`op` is kept. `None` when either name could escape
/// the topics directory — these arrive from devices, so they are not trusted.
fn retained_path(topic: &str, op: &str) -> Option<PathBuf> {
    fn safe(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 64
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }
    if !safe(topic) || !safe(op) {
        return None;
    }
    Some(crate::paths::topics_dir().join(topic).join(format!("{op}.json")))
}

/// Keep the newest payload for a topic, so a surface that was not listening —
/// or has no peer to listen with — can still read it.
pub(crate) fn retain(topic: &str, op: &str, payload: &serde_json::Value) {
    let Some(path) = retained_path(topic, op) else {
        tracing::warn!("[topic] refusing to retain {topic}/{op}: unsafe name");
        return;
    };
    let Some(dir) = path.parent() else { return };
    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::warn!("[topic] retain {topic}/{op} failed: {e}");
        return;
    }
    // Write-then-rename: a reader polling this file never catches a half-write.
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec(payload).unwrap_or_else(|_| b"null".to_vec());
    if let Err(e) = std::fs::write(&tmp, &body).and_then(|()| std::fs::rename(&tmp, &path)) {
        tracing::warn!("[topic] retain {topic}/{op} failed: {e}");
    }
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
    /// Keep this payload as the topic's current value, for surfaces that are
    /// not connected when it is published.
    #[serde(default)]
    retain: bool,
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
    if body.retain {
        retain(&body.topic, &body.op, &body.payload);
    }
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

#[derive(Deserialize)]
pub(crate) struct LatestQuery {
    topic: String,
    op: String,
}

/// GET /api/topic/latest?topic=&op= — the topic's retained value.
///
/// 404 when nothing has been retained, which is a real answer and not an
/// error: it means nobody has published this yet. Callers must show that as
/// "no reading" rather than inventing one.
pub(crate) async fn latest(Query(q): Query<LatestQuery>) -> impl IntoResponse {
    let missing = (
        axum::http::StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "nothing retained on this topic" })),
    );
    let Some(path) = retained_path(&q.topic, &q.op) else {
        return missing;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return missing;
    };
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&text) else {
        return missing;
    };
    let retained_at = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .map(|t| {
            chrono::DateTime::<chrono::Utc>::from(t)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
        .unwrap_or_default();
    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({
            "topic": q.topic,
            "op": q.op,
            "retained_at": retained_at,
            "payload": payload,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_names_that_would_escape_the_topics_dir() {
        assert!(retained_path("shifu", "readout").is_some());
        assert!(retained_path("../etc", "passwd").is_none());
        assert!(retained_path("shifu", "../../x").is_none());
        assert!(retained_path("shifu/nested", "readout").is_none());
        assert!(retained_path("", "readout").is_none());
        assert!(retained_path("shifu", "").is_none());
        assert!(retained_path(&"x".repeat(65), "readout").is_none());
    }

    #[test]
    fn retained_path_is_under_the_topics_dir() {
        let p = retained_path("shifu", "readout").unwrap();
        assert!(p.starts_with(crate::paths::topics_dir()));
        assert!(p.ends_with("shifu/readout.json"));
    }
}
