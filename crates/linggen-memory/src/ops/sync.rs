//! What a replica checks against its primary before it mixes rows in.

use crate::error::MemError;
use crate::handle::Memory;
use crate::memory::changes::Entry;
use serde::Deserialize;
use serde_json::{json, Value};

const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 1000;

#[derive(Deserialize)]
pub struct RowsQuery {
    /// Comma-separated row ids.
    ids: String,
}

pub async fn rows(state: &Memory, q: RowsQuery) -> Result<Value, MemError> {
    let entries: Vec<Entry> = q
        .ids
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .take(MAX_LIMIT)
        .map(|id| Entry {
            seq: 0,
            id: id.to_string(),
            table: String::new(),
            op: crate::memory::changes::Op::Upsert,
        })
        .collect();
    let (rows, tombstones) = bodies(state, &entries).await?;
    Ok(json!({ "rows": rows, "tombstones": tombstones }))
}

#[derive(Deserialize)]
pub struct ChangesQuery {
    #[serde(default)]
    since: u64,
    limit: Option<usize>,
}

pub async fn changes(state: &Memory, q: ChangesQuery) -> Result<Value, MemError> {
    let log = state
        .store
        .changes()
        .ok_or_else(|| MemError::internal(anyhow::anyhow!("no change log on this daemon")))?;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let (entries, next, more) = log.since(q.since, limit);
    let (rows, tombstones) = bodies(state, &entries).await?;
    Ok(json!({
        "model": crate::ops::sync::model_id(),
        "head": log.head(),
        "next": next,
        "more": more,
        "rows": rows,
        "tombstones": tombstones,
        "days": read_sidecar(&crate::ops::days::days_path(&state.data_dir)),
        "issues": read_sidecar(&crate::ops::issues::issues_path(&state.data_dir)),
    }))
}

/// Each id's current row and table, or a tombstone when it is gone.
async fn bodies(state: &Memory, entries: &[Entry]) -> Result<(Vec<Value>, Vec<String>), MemError> {
    let mut rows = Vec::new();
    let mut tombstones = Vec::new();
    for e in entries {
        match current(state, &e.id).await? {
            Some((table, row)) => rows.push(json!({ "table": table, "row": row })),
            None => tombstones.push(e.id.clone()),
        }
    }
    Ok((rows, tombstones))
}

async fn current(state: &Memory, id: &str) -> Result<Option<(&'static str, Value)>, MemError> {
    let err = |e: anyhow::Error| MemError::internal(e);
    if let Some(row) = state.store.get(id).await.map_err(err)? {
        return Ok(Some((
            "semantic",
            serde_json::to_value(row).unwrap_or(Value::Null),
        )));
    }
    if let Some(row) = state.episodic.get(id).await.map_err(err)? {
        return Ok(Some((
            "episodic",
            serde_json::to_value(row).unwrap_or(Value::Null),
        )));
    }
    Ok(None)
}

fn read_sidecar(path: &std::path::Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

/// The embedding model this store's vectors come from.
pub fn model_id() -> &'static str {
    crate::embed::MODEL_REPO
}
