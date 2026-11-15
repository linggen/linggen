//! Writes a replica made while its primary was offline.
//!
//! Each one is applied to the local copy at once (so it is searchable) and
//! queued here — `<data_dir>/memory/.outbox.json`, in order — as `{seq, verb,
//! args, replica_id, created_at, local_ids}`. When the primary answers again
//! the queue is replayed in order, then the replica pulls.
//!
//! The primary wins. A queued write whose target rows the primary changed in
//! the meantime is NOT applied; nor is one the primary refuses (its own
//! guards — the user-voice rule among them — run on every forwarded or
//! replayed write). Either way a review item is filed on the primary and the
//! touched rows are restored here from the primary's copy. Rows the local
//! apply created are always dropped after replay; the pull brings the
//! primary's own.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::http::state::SharedState;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Queued {
    pub seq: u64,
    /// The route, e.g. `/api/memory/add`.
    pub verb: String,
    pub args: Value,
    pub replica_id: String,
    pub created_at: DateTime<Utc>,
    /// Rows the local apply created — dropped once the primary has the write.
    #[serde(default)]
    pub local_ids: Vec<String>,
}

fn path(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join(".outbox.json")
}

pub fn load(data_dir: &Path) -> Vec<Queued> {
    std::fs::read_to_string(path(data_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(data_dir: &Path, q: &[Queued]) -> std::io::Result<()> {
    let p = path(data_dir);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(q).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(tmp, p)
}

pub fn len(data_dir: &Path) -> usize {
    load(data_dir).len()
}

/// Queue one write, after its local apply.
pub fn push(state: &SharedState, verb: &str, args: Value, local_ids: Vec<String>) -> Result<()> {
    let mut q = load(&state.data_dir);
    let seq = q.last().map_or(1, |e| e.seq + 1);
    let replica_id = state.replica.snapshot().device_id.unwrap_or_default();
    q.push(Queued {
        seq,
        verb: verb.to_string(),
        args,
        replica_id,
        created_at: Utc::now(),
        local_ids,
    });
    save(&state.data_dir, &q)?;
    Ok(())
}

/// The primary did not answer — stop and keep the rest queued.
#[derive(Debug)]
pub struct Offline;

impl std::fmt::Display for Offline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("primary offline")
    }
}

impl std::error::Error for Offline {}

/// Replay the queue in order. Stops at the first unanswered call.
pub async fn replay(state: &SharedState) -> Result<()> {
    let Some((base, token)) = state.replica.upstream() else {
        return Ok(());
    };
    loop {
        let q = load(&state.data_dir);
        let Some(first) = q.first().cloned() else {
            return Ok(());
        };
        replay_one(state, &base, &token, &first).await?;
        let rest: Vec<Queued> = load(&state.data_dir)
            .into_iter()
            .filter(|e| e.seq != first.seq)
            .collect();
        save(&state.data_dir, &rest)?;
    }
}

async fn replay_one(state: &SharedState, base: &str, token: &str, e: &Queued) -> Result<()> {
    let targets = target_ids(&e.verb, &e.args);
    if changed_since(base, token, &targets, e.created_at).await? {
        let note =
            "changed on the primary while this machine was offline; the primary's version stands";
        return settle_refused(state, base, token, e, &targets, note).await;
    }
    let resp = super::http()
        .post(format!("{base}{}", e.verb))
        .header(super::DEVICE_HEADER, token)
        .json(&e.args)
        .send()
        .await
        .map_err(|_| Offline)?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        drop_local(state, &e.local_ids).await;
        return Ok(());
    }
    let why = body["error"].as_str().unwrap_or("refused").to_string();
    let note = format!("the primary refused a write queued offline: {why}");
    settle_refused(state, base, token, e, &targets, &note).await
}

/// Primary wins: file a review item there, and put the touched rows back the
/// way the primary has them.
async fn settle_refused(
    state: &SharedState,
    base: &str,
    token: &str,
    e: &Queued,
    targets: &[String],
    note: &str,
) -> Result<()> {
    let issue = json!({
        "kind": super::forward::CONFLICT_KIND,
        "row_ids": targets,
        "note": format!("{note} ({} from replica {})", e.verb, e.replica_id),
    });
    super::http()
        .post(format!("{base}/api/memory/issue_add"))
        .header(super::DEVICE_HEADER, token)
        .json(&issue)
        .send()
        .await
        .map_err(|_| Offline)?;
    drop_local(state, &e.local_ids).await;
    restore(state, base, token, targets).await
}

/// Rows a write would change on the primary: what it replaces or edits.
pub fn target_ids(verb: &str, args: &Value) -> Vec<String> {
    let mut ids: Vec<String> = args["replace_ids"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let by_id = matches!(verb, "/api/memory/update" | "/api/memory/delete");
    if let (true, Some(id)) = (by_id, args["id"].as_str()) {
        ids.push(id.to_string());
    }
    ids
}

/// Did the primary touch any of these rows after `since` (or remove them)?
async fn changed_since(
    base: &str,
    token: &str,
    ids: &[String],
    since: DateTime<Utc>,
) -> Result<bool> {
    if ids.is_empty() {
        return Ok(false);
    }
    let data = fetch_rows(base, token, ids).await?;
    let gone = data["tombstones"].as_array().is_some_and(|t| !t.is_empty());
    let touched = data["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|r| row_time(&r["row"]).is_some_and(|t| t > since));
    Ok(gone || touched)
}

/// The latest of a row's own clocks.
fn row_time(row: &Value) -> Option<DateTime<Utc>> {
    ["updated_at", "expired_at", "created_at"]
        .iter()
        .filter_map(|k| row[*k].as_str())
        .filter_map(|s| s.parse::<DateTime<Utc>>().ok())
        .max()
}

async fn fetch_rows(base: &str, token: &str, ids: &[String]) -> Result<Value> {
    let resp = super::http()
        .get(format!("{base}/api/sync/rows"))
        .query(&[("ids", ids.join(","))])
        .header(super::DEVICE_HEADER, token)
        .send()
        .await
        .map_err(|_| Offline)?;
    let body: Value = resp.json().await?;
    body.get("data")
        .cloned()
        .ok_or_else(|| anyhow!("no data from the primary"))
}

/// Put these rows back exactly as the primary has them.
async fn restore(state: &SharedState, base: &str, token: &str, ids: &[String]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let data = fetch_rows(base, token, ids).await?;
    super::pull::apply_value(state, &data)
        .await
        .map_err(|e| anyhow!(e.message))
}

async fn drop_local(state: &SharedState, ids: &[String]) {
    for id in ids {
        let _ = state.store.delete(id).await;
        let _ = state.episodic.delete(id).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_are_what_a_write_replaces_or_edits() {
        let add = json!({ "content": "x", "replace_ids": ["a", "b"] });
        assert_eq!(target_ids("/api/memory/add", &add), vec!["a", "b"]);
        let upd = json!({ "id": "c", "content": "y" });
        assert_eq!(target_ids("/api/memory/update", &upd), vec!["c"]);
        assert!(target_ids("/api/memory/add", &json!({ "content": "z" })).is_empty());
        assert!(target_ids("/api/memory/issue_add", &json!({ "id": "q" })).is_empty());
    }

    #[test]
    fn a_rows_clock_is_its_latest_stamp() {
        let row = json!({
            "created_at": "2026-10-01T00:00:00Z",
            "updated_at": "2026-10-03T00:00:00Z",
        });
        assert_eq!(
            row_time(&row).unwrap(),
            "2026-10-03T00:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }
}
