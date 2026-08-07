//! The write door to this machine's activity log.
//!
//! Skills, app pages and scripts have no way into the engine's memory, so this
//! is how they record that the world changed: apple-shifu after a backup, CFO
//! after an import, DJ after a delete. The engine names no app — it takes the
//! app's own word for what it is, exactly as `/api/topic/publish` does.
//!
//! Gated like every other route: loopback (local skills) passes, LAN callers
//! need their device token. Perception is local by law
//! (`doc/perception-spec.md` §8) — nothing written here leaves the machine.

use axum::{extract::Query, http::HeaderMap, response::IntoResponse, Json};
use serde::Deserialize;

use crate::perception::activity;

#[derive(Deserialize)]
pub(crate) struct RecordBody {
    /// `user`, `yinyue`, `ling`, `system` — the actor, not the device.
    #[serde(default)]
    by: Option<String>,
    /// `dj`, `photos`, `cfo`, `shifu`, `system`.
    app: String,
    /// `delete`, `add`, `edit`, `sync`, `backup`, `clean`, `import`, `pair`, …
    verb: String,
    /// What it happened to, in the user's terms: a song title, not a path.
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    detail: Option<serde_json::Value>,
}

/// POST /api/activity — record one change to this machine's world.
///
/// A change, never a glance (§3). The caller decides what a verb means; the
/// engine cannot know, and a door that argues with its callers just grows a
/// second vocabulary.
///
/// Two things the caller does *not* decide, because both end up inside the
/// resident's prompt: **who** — `by` is a closed set, since it is rendered as
/// the subject of a sentence — and **where** — the device is read from the
/// caller's own token, so a phone's record says the phone rather than this Mac.
/// Length and line breaks are the writer's door's business (`clean`), so every
/// writer gets the same ceiling and none of them can end a bullet early.
pub(crate) async fn record(headers: HeaderMap, Json(body): Json<RecordBody>) -> impl IntoResponse {
    if body.app.trim().is_empty() || body.verb.trim().is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "app and verb are required" })),
        );
    }
    let by = body.by.as_deref().unwrap_or("user").trim().to_string();
    if !activity::is_actor(&by) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("by must be one of {:?}", activity::ACTORS),
            })),
        );
    }
    // A LAN caller carries a device token; loopback does not, and loopback is
    // this Mac.
    let device = crate::server::api::pair::caller_device(&headers)
        .map(|id| crate::server::api::pair::device_name(&id).unwrap_or(id));
    activity::record_from(
        device,
        &by,
        body.app.trim(),
        body.verb.trim(),
        body.object,
        body.detail,
    );
    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({ "ok": true })),
    )
}

#[derive(Deserialize)]
pub(crate) struct RecentQuery {
    #[serde(default)]
    limit: Option<usize>,
}

/// GET /api/activity?limit= — the same lines the agent's `recent_activity`
/// tool returns, for a page that wants to show them.
///
/// One reader and one writer for one log: a page rendering its own version of
/// "what happened" from some other source is how two accounts of one day start
/// to disagree.
pub(crate) async fn recent(Query(q): Query<RecentQuery>) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    Json(serde_json::json!({
        "lines": activity::log().lines(limit),
    }))
}
