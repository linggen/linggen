//! Writes on a replica: forwarded to the primary when it answers, queued in
//! the outbox (and applied locally) when it does not.
//!
//! A middleware inside the gate, so every caller — the CLI, `/mcp` (which
//! loops back through these routes), the engine — takes the same path.
//! Reads are untouched: they are served from the local copy.
//!
//! - Online: the write goes to the primary with this replica's token, the
//!   primary's own handler (and its guards) decide, and its answer is
//!   returned as-is. The replica then pulls, so the caller reads its write.
//! - Offline (or not yet approved): applied locally, so it is searchable at
//!   once, and queued. Earlier queued writes always go before a new one.
//! - Dreaming is the primary's (`remember_day`, `sweep`, the dream lease,
//!   the schema step): a replica refuses those and names the primary.

use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use crate::http::state::SharedState;
use crate::replica::{outbox, Role};

/// The review-queue kind for a queued write the primary did not take.
pub const CONFLICT_KIND: &str = "replica-conflict";

/// Writes a replica forwards (or queues).
const WRITES: &[&str] = &[
    "/api/memory/add",
    "/api/memory/add_batch",
    "/api/memory/update",
    "/api/memory/delete",
    "/api/memory/forget",
    "/api/memory/restamp",
    "/api/memory/issue_add",
    "/api/memory/issue_resolve",
    "/api/memory/harvest_day",
];

/// The primary's alone: dreaming, the forget sweep, the schema step.
const PRIMARY_ONLY: &[&str] = &[
    "/api/memory/remember_day",
    "/api/memory/sweep",
    "/api/memory/dream_lease",
    "/api/schema/apply",
];

const MAX_BODY: usize = 16 * 1024 * 1024;

pub async fn replica_writes(
    State(state): State<SharedState>,
    req: Request,
    next: Next,
) -> Response {
    if req.method() != Method::POST || state.replica.role() != Role::Replica {
        return next.run(req).await;
    }
    let path = req.uri().path().to_string();
    if PRIMARY_ONLY.contains(&path.as_str()) {
        return primary_only(&state);
    }
    if !WRITES.contains(&path.as_str()) {
        return next.run(req).await;
    }
    let (parts, body) = req.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_BODY).await else {
        return refusal(StatusCode::BAD_REQUEST, "request body too large");
    };
    let args: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if let Some(answer) = forward_in_order(&state, &path, &args).await {
        return answer;
    }
    let local = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    queue_after_local(&state, &path, args, local).await
}

/// Forward when nothing is queued ahead (replaying the queue first).
/// `None` = the primary did not answer; the caller queues.
async fn forward_in_order(state: &SharedState, path: &str, args: &Value) -> Option<Response> {
    if outbox::len(&state.data_dir) > 0 {
        super::sync_once(state).await;
        if outbox::len(&state.data_dir) > 0 {
            return None;
        }
    }
    let answer = forward(state, path, args).await?;
    if answer.status().is_success() {
        super::sync_once(state).await;
    }
    Some(answer)
}

/// One write to the primary. `None` when it did not answer.
async fn forward(state: &SharedState, path: &str, args: &Value) -> Option<Response> {
    let (base, token) = state.replica.upstream()?;
    let sent = super::http()
        .post(format!("{base}{path}"))
        .header(super::DEVICE_HEADER, token)
        .json(args)
        .send()
        .await;
    let Ok(resp) = sent else {
        state.replica.update(|f| {
            f.online = false;
            f.error = Some("primary offline".into());
        });
        return None;
    };
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    Some((status, Json(body)).into_response())
}

/// The local apply happened; queue the write for the primary, marked so a
/// caller can tell.
async fn queue_after_local(
    state: &SharedState,
    path: &str,
    args: Value,
    local: Response,
) -> Response {
    let (mut parts, body) = local.into_parts();
    let bytes = to_bytes(body, MAX_BODY).await.unwrap_or_default();
    if parts.status.is_success() {
        let reply: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if let Err(e) = outbox::push(state, path, args, created_ids(&reply)) {
            tracing::warn!("replica: could not queue {path}: {e:#}");
        }
        parts
            .headers
            .insert("x-ling-mem-queued", HeaderValue::from_static("1"));
    }
    Response::from_parts(parts, Body::from(bytes))
}

/// Rows the local apply CREATED (not merged into): dropped after replay,
/// when the primary's own copy arrives by pull.
pub fn created_ids(reply: &Value) -> Vec<String> {
    let data = &reply["data"];
    if data["action"] != "added" {
        return Vec::new();
    }
    let one = data["fact"]["id"].as_str().map(str::to_string);
    let many = data["facts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| f["id"].as_str().map(str::to_string));
    one.into_iter().chain(many).collect()
}

fn primary_only(state: &SharedState) -> Response {
    let host = state.replica.snapshot().primary.unwrap_or_default();
    refusal(
        StatusCode::CONFLICT,
        &format!("Memory is dreamed on the primary ({host})"),
    )
}

fn refusal(status: StatusCode, msg: &str) -> Response {
    (
        status,
        Json(json!({ "ok": false, "error": msg, "code": "REPLICA" })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_created_rows_are_dropped_after_replay() {
        let added = json!({ "ok": true, "data": { "action": "added", "fact": { "id": "a" } } });
        assert_eq!(created_ids(&added), vec!["a"]);
        let batch =
            json!({ "data": { "action": "added", "facts": [{ "id": "b" }, { "id": "c" }] } });
        assert_eq!(created_ids(&batch), vec!["b", "c"]);
        let merged = json!({ "data": { "action": "merged", "fact": { "id": "d" } } });
        assert!(created_ids(&merged).is_empty());
    }

    #[test]
    fn dreaming_and_writes_are_told_apart() {
        assert!(PRIMARY_ONLY.contains(&"/api/memory/remember_day"));
        assert!(WRITES.contains(&"/api/memory/add"));
        assert!(!WRITES.contains(&"/api/memory/search"));
        assert!(!WRITES.iter().any(|w| PRIMARY_ONLY.contains(w)));
    }
}
