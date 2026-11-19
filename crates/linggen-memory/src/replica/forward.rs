//! Writes on a replica: forwarded to the primary when it answers, queued in
//! the outbox (and applied locally) when it does not.
//!
//! `Memory::call` decides (`call::route`) and comes here for a write, so
//! every caller — the routes (and through them the CLI, `/mcp`, the
//! engine) and an in-process call — takes the same path. Reads are served
//! from the local copy.
//!
//! - Online: the write goes to the primary with this replica's token, the
//!   primary's own handler (and its guards) decide, and its answer is
//!   returned as-is. The replica then pulls, so the caller reads its write.
//! - Offline (or not yet approved): applied locally, so it is searchable at
//!   once, and queued. Earlier queued writes always go before a new one.
//! - Dreaming is the primary's (`remember_day`, `sweep`, the dream lease,
//!   the schema step): a replica refuses those and names the primary.

use serde_json::{json, Value};

use crate::call::Reply;
use crate::error::MemError;
use crate::handle::Memory;
use crate::http::state::SharedState;
use crate::ops::verbs;
use crate::replica::outbox;

/// The review-queue kind for a queued write the primary did not take.
pub const CONFLICT_KIND: &str = "replica-conflict";

/// Writes a replica forwards (or queues) — with the pull and the connect
/// routes, the whole of what a replica asks of its primary. Read off the
/// verb table.
pub const WRITES: &[&str] = &verbs::WRITE_PATHS;

/// The primary's alone: dreaming, the forget sweep, the schema step.
pub const PRIMARY_ONLY: &[&str] = &verbs::PRIMARY_ONLY_PATHS;

/// Forward when nothing is queued ahead (replaying the queue first).
/// `None` = the primary did not answer; the caller queues.
pub(crate) async fn forward_in_order(
    state: &SharedState,
    path: &str,
    args: &Value,
) -> Option<Reply> {
    if outbox::len(&state.data_dir) > 0 {
        if let Some(refusal) = super::sync_and_report_warmup(state).await {
            if matches!(path, "/api/memory/add" | "/api/memory/add_batch" | "/api/memory/update") {
                return Some(refusal);
            }
        }
        if outbox::len(&state.data_dir) > 0 {
            return None;
        }
    }
    let answer = forward(state, path, args).await?;
    if matches!(answer, Reply::Primary { status, .. } if (200..300).contains(&status)) {
        super::sync_once(state).await;
    }
    Some(answer)
}

/// One write to the primary. `None` when it did not answer.
async fn forward(state: &SharedState, path: &str, args: &Value) -> Option<Reply> {
    let (base, token) = state.replica.upstream()?;
    let sent = super::write_http()
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
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    Some(Reply::Primary { status, body })
}

/// The local apply happened and answered `data`; queue the write for the
/// primary. A write that could not be queued is an error its caller sees.
pub(crate) fn queue(state: &SharedState, path: &str, args: Value, data: &Value) -> anyhow::Result<()> {
    let reply = json!({ "data": data });
    outbox::push(state, path, args, created_ids(&reply)).map_err(|e| {
        tracing::warn!("replica: could not queue {path}: {e:#}");
        e
    })
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

/// A replica's refusal of a primary-only verb, naming the primary.
pub(crate) fn primary_only(state: &Memory) -> MemError {
    let host = state.replica.snapshot().primary.unwrap_or_default();
    MemError::replica(format!("Memory is dreamed on the primary ({host})"))
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
