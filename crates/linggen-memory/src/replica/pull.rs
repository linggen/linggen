//! Pulling the primary's changes and applying them to this replica's copy.
//!
//! Idempotent by row id: a row is written exactly as the primary has it (in
//! the table it lives in there, and removed from the other — an episodic row
//! promoted to long-term moves), a tombstone removes the id from both. The
//! day records and the review queue are mirrored whole; the primary wins.
//! A primary whose embedding model differs is not followed at all: vectors
//! from two models in one table would make search quietly wrong.

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;

use crate::http::state::SharedState;
use crate::memory::Memory;

const PAGE: usize = 500;

#[derive(Deserialize)]
struct Page {
    model: String,
    next: u64,
    #[serde(default)]
    more: bool,
    #[serde(default)]
    rows: Vec<SyncedRow>,
    #[serde(default)]
    tombstones: Vec<String>,
    #[serde(default)]
    days: Value,
    #[serde(default)]
    issues: Value,
}

#[derive(Deserialize)]
struct SyncedRow {
    table: String,
    row: Memory,
}

/// Pull until caught up. Records online/offline and why it stopped.
pub async fn pull_all(state: &SharedState) {
    let Some((base, token)) = state.replica.upstream() else {
        return;
    };
    loop {
        let cursor = state.replica.snapshot().cursor;
        let page = match fetch(&base, &token, cursor).await {
            Ok(p) => p,
            Err(e) => return note(state, e),
        };
        if page.model != crate::http::sync::model_id() {
            let why = format!(
                "embedding model differs: primary {}, here {}",
                page.model,
                crate::http::sync::model_id()
            );
            return state.replica.update(|f| {
                f.online = true;
                f.error = Some(why);
            });
        }
        if let Err(e) = apply(state, &page).await {
            return note(state, e);
        }
        state.replica.update(|f| {
            f.cursor = page.next;
            f.last_sync = Some(Utc::now());
            f.online = true;
            f.error = None;
        });
        if !page.more {
            return;
        }
    }
}

/// Record a failed pull: offline when the primary did not answer.
fn note(state: &SharedState, e: UpstreamError) {
    state.replica.update(|f| {
        f.online = !e.offline;
        f.error = Some(e.message);
    });
}

pub(crate) struct UpstreamError {
    pub offline: bool,
    pub message: String,
}

impl From<anyhow::Error> for UpstreamError {
    fn from(e: anyhow::Error) -> Self {
        Self {
            offline: false,
            message: format!("{e:#}"),
        }
    }
}

async fn fetch(base: &str, token: &str, cursor: u64) -> Result<Page, UpstreamError> {
    let url = format!("{base}/api/sync/changes?since={cursor}&limit={PAGE}");
    let resp = super::pull_http()
        .get(&url)
        .header(super::DEVICE_HEADER, token)
        .send()
        .await
        .map_err(|e| {
            tracing::info!("replica: pull from {base} failed: {e}");
            UpstreamError {
                offline: true,
                message: "primary offline".into(),
            }
        })?;
    if matches!(resp.status().as_u16(), 401 | 403) {
        return Err(anyhow!("the primary no longer accepts this machine").into());
    }
    let body: Value = resp.json().await.context("reading the primary's changes")?;
    let data = body
        .get("data")
        .cloned()
        .ok_or_else(|| anyhow!("no data in the reply"))?;
    Ok(serde_json::from_value(data).context("parsing the primary's changes")?)
}

async fn apply(state: &SharedState, page: &Page) -> Result<(), UpstreamError> {
    apply_rows(state, &page.rows, &page.tombstones).await?;
    mirror(&crate::http::days::days_path(&state.data_dir), &page.days)?;
    mirror(
        &crate::http::issues::issues_path(&state.data_dir),
        &page.issues,
    )?;
    Ok(())
}

async fn apply_rows(
    state: &SharedState,
    rows: &[SyncedRow],
    tombstones: &[String],
) -> Result<(), UpstreamError> {
    for r in rows {
        put(state, r).await?;
    }
    for id in tombstones {
        state.store.delete(id).await?;
        state.episodic.delete(id).await?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct RowsOnly {
    #[serde(default)]
    rows: Vec<SyncedRow>,
    #[serde(default)]
    tombstones: Vec<String>,
}

/// Apply a `/api/sync/rows` answer: these rows as the primary has them.
pub(crate) async fn apply_value(state: &SharedState, data: &Value) -> Result<(), UpstreamError> {
    let r: RowsOnly = serde_json::from_value(data.clone()).map_err(anyhow::Error::from)?;
    apply_rows(state, &r.rows, &r.tombstones).await
}

/// The row where the primary keeps it, and nowhere else.
async fn put(state: &SharedState, r: &SyncedRow) -> Result<()> {
    let (target, other) = match r.table.as_str() {
        "episodic" => (&state.episodic, &state.store),
        _ => (&state.store, &state.episodic),
    };
    target.put_synced(&r.row).await?;
    other.delete(&r.row.id).await?;
    Ok(())
}

/// Write a sidecar the primary sent (absent on the primary = leave ours).
fn mirror(path: &std::path::Path, value: &Value) -> Result<()> {
    if value.is_null() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.sync-tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}
