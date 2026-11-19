//! The review-queue sidecar, `<data_dir>/memory/.issues.json`: what the
//! dream audit queued and how each item was closed. The `issue*` verbs live
//! here; `http::issues` adapts them.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Scope, index and summary fixes are not queued: the dream applies them
/// itself (they are not the row's words) and logs what it changed.
pub(crate) const KINDS: &[&str] = &[
    "chain",
    "stale-status",
    "contradiction",
    "subject",
    crate::replica::forward::CONFLICT_KIND,
];
pub(crate) const OUTCOMES: &[&str] = &["resolved", "dismissed"];

/// One queued review item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueRecord {
    pub id: String,
    /// What the audit saw: `chain` (uncertain merge candidate),
    /// `stale-status` (a status claim older than the world),
    /// `contradiction` (conflicting rows needing the user's pick), or
    /// `subject` (digest cluster of doubtful subject coherence). Items of the
    /// retired `index`/`scope` kinds still load and resolve.
    pub kind: String,
    /// The memory row ids this item is about.
    #[serde(default)]
    pub row_ids: Vec<String>,
    /// What the finder saw / proposes — the fact a solver starts from.
    pub note: String,
    pub created_at: DateTime<Utc>,
    /// `open` | `resolved` | `dismissed`.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub resolved_at: Option<DateTime<Utc>>,
    /// Solver's one-line record of what was done.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub resolution: Option<String>,
}

/// The `.issues.json` sidecar — `{"issues": [record, …]}` in queue order.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IssuesState {
    #[serde(default)]
    pub issues: Vec<IssueRecord>,
}

pub(crate) fn issues_path(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join(".issues.json")
}

/// Read-or-default, mirroring `days::load` — a missing/corrupt file never
/// blocks the store.
pub(crate) async fn load(data_dir: &Path) -> IssuesState {
    let Ok(bytes) = tokio::fs::read(issues_path(data_dir)).await else {
        return IssuesState::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// The one lock around every change to `.issues.json`. Its writers:
/// `issue_add` and `issue_resolve`, each a read-change-save through
/// [`modify`]; and a replica's pull, which replaces the file with the
/// primary's copy under [`lock`].
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) async fn lock() -> tokio::sync::MutexGuard<'static, ()> {
    LOCK.lock().await
}

/// Read, change and save `.issues.json` under [`lock`]; `f` answers
/// whether to save (`Ok(false)` leaves the file as it is).
async fn modify<R>(
    data_dir: &Path,
    f: impl FnOnce(&mut IssuesState) -> Result<(R, bool), crate::error::MemError>,
) -> Result<R, crate::error::MemError> {
    let _guard = lock().await;
    let mut state = load(data_dir).await;
    let (out, changed) = f(&mut state)?;
    if changed {
        save(data_dir, &state).await.map_err(|e| {
            crate::error::MemError::internal(anyhow::anyhow!("persisting .issues.json: {e}"))
        })?;
    }
    Ok(out)
}

async fn save(data_dir: &Path, state: &IssuesState) -> Result<(), std::io::Error> {
    let path = issues_path(data_dir);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(state).expect("IssuesState serializes");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(tmp, path).await?;
    Ok(())
}

/// Open-item count for the stats/days rollups.
pub(crate) async fn open_count(data_dir: &Path) -> usize {
    load(data_dir)
        .await
        .issues
        .iter()
        .filter(|i| i.status == "open")
        .count()
}

/// Every row id referenced by any issue, whatever its status. The marker
/// scan excludes these from candidate selection: an open item is already
/// queued, a closed one was already ruled on — re-surfacing either burns
/// a capped slot without advancing the cursor (a lane that only queues
/// cannot advance its own cursor).
pub(crate) async fn issued_row_ids(data_dir: &Path) -> std::collections::HashSet<String> {
    load(data_dir)
        .await
        .issues
        .into_iter()
        .flat_map(|i| i.row_ids)
        .collect()
}

/// Row ids named by `subject`-kind issues, any status. A rejected digest
/// cluster is recorded as a (dismissed) subject issue on its seed; the
/// subject scan excludes those seeds so a ruled-out cluster stops
/// re-serving — the same cursor discipline the marker lane uses.
/// Accepted digests need no record: the merge consumes the members.
pub(crate) async fn subject_ruled_ids(data_dir: &Path) -> std::collections::HashSet<String> {
    load(data_dir)
        .await
        .issues
        .into_iter()
        .filter(|i| i.kind == "subject")
        .flat_map(|i| i.row_ids)
        .collect()
}

#[derive(Debug, Deserialize)]
pub struct IssuesRequest {
    /// `open` (default) | `resolved` | `dismissed` | `all`.
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

pub(in crate::ops) async fn issues(
    state: &crate::handle::Memory,
    req: IssuesRequest,
) -> Result<serde_json::Value, crate::error::MemError> {
    use crate::error::MemError;
    use serde_json::json;
    let want = req.status.as_deref().unwrap_or("open");
    if !matches!(want, "open" | "resolved" | "dismissed" | "all") {
        return Err(MemError::bad_request(format!(
            "invalid status {want:?}: expected open | resolved | dismissed | all"
        )));
    }
    let all = load(&state.data_dir).await.issues;
    let open = all.iter().filter(|i| i.status == "open").count();
    let limit = req.limit.unwrap_or(50);
    let items: Vec<&IssueRecord> = all
        .iter()
        .filter(|i| want == "all" || i.status == want)
        .take(limit)
        .collect();
    Ok(json!({ "open_count": open, "issues": items }))
}

#[derive(Debug, Deserialize)]
pub struct IssueAddRequest {
    kind: String,
    #[serde(default)]
    row_ids: Vec<String>,
    note: String,
}

pub(in crate::ops) async fn issue_add(
    state: &crate::handle::Memory,
    req: IssueAddRequest,
) -> Result<serde_json::Value, crate::error::MemError> {
    use crate::error::MemError;
    use crate::memory::types::short_id;
    use serde_json::json;
    let kind = req.kind.trim();
    if !KINDS.contains(&kind) {
        return Err(MemError::bad_request(format!(
            "invalid kind {kind:?}: expected one of {KINDS:?}"
        )));
    }
    if req.note.trim().is_empty() {
        return Err(MemError::bad_request("note is required".to_string()));
    }
    let mut rows = req.row_ids.clone();
    rows.sort();
    modify(&state.data_dir, |all| {
        if let Some(existing) = all.issues.iter().find(|i| {
            i.status == "open" && i.kind == kind && {
                let mut theirs = i.row_ids.clone();
                theirs.sort();
                theirs == rows
            }
        }) {
            return Ok((json!({ "issue": existing, "deduped": true }), false));
        }
        let record = IssueRecord {
            id: short_id(),
            kind: kind.to_string(),
            row_ids: req.row_ids,
            note: req.note.trim().to_string(),
            created_at: Utc::now(),
            status: "open".to_string(),
            resolved_at: None,
            resolution: None,
        };
        all.issues.push(record.clone());
        Ok((json!({ "issue": record, "deduped": false }), true))
    })
    .await
}

#[derive(Debug, Deserialize)]
pub struct IssueResolveRequest {
    id: String,
    /// `resolved` | `dismissed`.
    outcome: String,
    /// Solver's one-line record of what was done.
    #[serde(default)]
    note: Option<String>,
}

pub(in crate::ops) async fn issue_resolve(
    state: &crate::handle::Memory,
    req: IssueResolveRequest,
) -> Result<serde_json::Value, crate::error::MemError> {
    use crate::error::MemError;
    use serde_json::json;
    let outcome = req.outcome.trim();
    if !OUTCOMES.contains(&outcome) {
        return Err(MemError::bad_request(format!(
            "invalid outcome {outcome:?}: expected resolved | dismissed"
        )));
    }
    modify(&state.data_dir, |all| {
        let Some(item) = all.issues.iter_mut().find(|i| i.id == req.id) else {
            return Err(MemError::bad_request(format!(
                "no issue with id {:?}",
                req.id
            )));
        };
        if item.status != "open" {
            return Ok((json!({ "issue": item, "already_closed": true }), false));
        }
        item.status = outcome.to_string();
        item.resolved_at = Some(Utc::now());
        if let Some(note) = req.note.filter(|n| !n.trim().is_empty()) {
            item.resolution = Some(note.trim().to_string());
        }
        Ok((json!({ "issue": item, "already_closed": false }), true))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::short_id;

    fn record(kind: &str, rows: &[&str], status: &str) -> IssueRecord {
        IssueRecord {
            id: short_id(),
            kind: kind.to_string(),
            row_ids: rows.iter().map(|s| s.to_string()).collect(),
            note: "n".to_string(),
            created_at: Utc::now(),
            status: status.to_string(),
            resolved_at: None,
            resolution: None,
        }
    }

    #[tokio::test]
    async fn sidecar_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = IssuesState::default();
        state.issues.push(record("chain", &["a", "b"], "open"));
        save(dir.path(), &state).await.unwrap();
        let loaded = load(dir.path()).await;
        assert_eq!(loaded.issues.len(), 1);
        assert_eq!(loaded.issues[0].kind, "chain");
        assert_eq!(open_count(dir.path()).await, 1);
    }

    /// Concurrent adds all land: no save carries a snapshot that misses
    /// another's item.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_adds_keep_every_item() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::handle::scratch_state(
            dir.path(),
            std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        )
        .await;
        let calls: Vec<_> = (0..40)
            .map(|i| {
                let state = std::sync::Arc::clone(&state);
                tokio::spawn(async move {
                    let req: IssueAddRequest = serde_json::from_value(serde_json::json!({
                        "kind": "chain", "row_ids": [format!("r{i}")], "note": "n"
                    }))
                    .unwrap();
                    issue_add(&state, req).await.map(drop)
                })
            })
            .collect();
        for call in calls {
            call.await.unwrap().unwrap();
        }
        assert_eq!(load(dir.path()).await.issues.len(), 40);
    }

    #[tokio::test]
    async fn missing_sidecar_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).await.issues.is_empty());
        assert_eq!(open_count(dir.path()).await, 0);
    }

    #[tokio::test]
    async fn open_count_ignores_closed() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = IssuesState::default();
        state.issues.push(record("chain", &["a"], "open"));
        state
            .issues
            .push(record("stale-status", &["b"], "resolved"));
        state
            .issues
            .push(record("contradiction", &["c"], "dismissed"));
        save(dir.path(), &state).await.unwrap();
        assert_eq!(open_count(dir.path()).await, 1);
    }
}
