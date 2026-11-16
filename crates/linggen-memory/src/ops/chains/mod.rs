//! Chain scan — the condense stage's mechanical detector (see
//! `../../linggen/doc/memory-spec.md` §2 Condense).
//!
//! Semantic is append-mostly, so project truths accumulate **chains**:
//! same-subject rows where the newest completes or obsoletes the rest.
//! This endpoint computes them so the condense mission's agent (which is
//! Memory-tools-only and cannot script) never has to page the whole
//! store through its context. Read-only, zero-LLM — judgment and the
//! actual `replace_ids` merges belong to the caller.
//!
//! Three scan kinds, selected by `kind`:
//!
//! - **`cited`** (tier-1) — rows whose content cites another row's id
//!   verbatim. Citations are provable edges; connected components over
//!   them are auto-accept-quality chains.
//! - **`marker`** (tier-2) — rows carrying provisional-state language
//!   ("OPEN:", "uncommitted", "pending", …), each with its nearest
//!   stored-vector neighbors. Candidates only: the caller must confirm
//!   a real supersession before merging. Rows already named by a review
//!   issue (any status) are excluded, so each capped page surfaces fresh
//!   candidates instead of re-serving the queue.
//! - **`subject`** (v2) — star clusters over stored vectors: a seed row
//!   plus every row within `SUBJECT_FLOOR` cosine of it, minimum
//!   [`SUBJECT_MIN_ROWS`] members. Feeds the digest pass — N same-subject
//!   rows condensed into one focused per-subject row (never one mega
//!   state row: clusters are seed-anchored and size-capped, not
//!   transitive-closure blobs).
//!
//! Every cluster carries `derived_only` — true when all member rows are
//! the agent's own notes (`from=derived`, `tier=semantic`). The merge
//! law gates on it: derived-only chains may be merged unattended;
//! anything touching the user's voice may not.

use crate::error::MemError;
use crate::handle::Memory;
use crate::memory::Memory as Row;
use crate::memory::{Filters, Origin, SortOrder, Tier};
use serde::Deserialize;
use serde_json::{json, Value};

mod cited;
mod marker;
mod subject;

use cited::cited_chains;
use marker::marker_candidates;
use subject::subject_clusters;

const DEFAULT_LIMIT: usize = 10;
/// The scan needs every semantic row in memory at once; the table is
/// low-volume by design (hundreds), so a flat fetch is fine.
const SCAN_FETCH_LIMIT: usize = 100_000;

#[derive(Debug, Deserialize)]
pub struct ChainsRequest {
    /// `cited` (default), `marker`, or `subject`.
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
    /// Only clusters mergeable unattended (every row an agent note).
    /// The condense mission passes true so user-voice clusters never
    /// pollute its worklist pages across runs.
    #[serde(default)]
    derived_only: bool,
}

pub(in crate::ops) async fn chains(state: &Memory, req: ChainsRequest) -> Result<Value, MemError> {
    let kind = req.kind.as_deref().unwrap_or("cited");
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT).max(1);
    let offset = req.offset.unwrap_or(0);

    let rows = state
        .store
        .list(&Filters::default(), SortOrder::Oldest, SCAN_FETCH_LIMIT, 0)
        .await
        .map_err(|e| MemError::internal(e.context("listing semantic rows")))?;

    match kind {
        "cited" => Ok(cited_chains(&rows, limit, offset, req.derived_only)),
        "marker" => {
            let issued = crate::ops::issues::issued_row_ids(&state.data_dir).await;
            Ok(marker_candidates(
                &rows,
                limit,
                offset,
                req.derived_only,
                &issued,
            ))
        }
        "subject" => {
            let ruled = crate::ops::issues::subject_ruled_ids(&state.data_dir).await;
            Ok(subject_clusters(
                &rows,
                limit,
                offset,
                req.derived_only,
                &ruled,
                chrono::Utc::now(),
            ))
        }
        other => Err(MemError::bad_request(format!(
            "invalid kind {other:?}: expected \"cited\", \"marker\", or \"subject\""
        ))),
    }
}

// ── Shared helpers ──────────────────────────────────────────────────────────

/// The merge law's unattended-merge bar: the agent's own note, not the
/// user's voice, and never a core row.
fn is_derived_note(row: &Row) -> bool {
    row.origin == Origin::Derived && row.tier == Tier::Semantic
}

/// Row as the caller sees it — no vector (1024 floats of context noise).
fn public_row(row: &Row) -> Value {
    json!({
        "id": row.id,
        "content": row.content,
        "type": row.r#type.as_str(),
        "from": row.origin.as_str(),
        "tier": row.tier.as_str(),
        "scope": row.scope,
        "summary": row.summary,
        "created_at": row.created_at,
        "occurred_at": row.occurred_at,
    })
}

/// A row fixture for the scans' unit tests.
#[cfg(test)]
fn row(id: &str, content: &str, origin: Origin) -> Row {
    let mut m = Row::new(content, crate::memory::MemoryType::Built, origin);
    m.id = id.to_string();
    m
}
