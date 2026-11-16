//! `update`: patch one row by id — set/clear twins, `~` expansion, core
//! forcing no scope and no index, and the move across tables when the tier
//! patch crosses the semantic/episodic line.

use crate::error::MemError;
use crate::handle::Memory;
use crate::memory::Memory as Row;
use crate::memory::{MemoryPatch, MemoryStore, Tier};
use crate::ops::memory::dto::{fact_public, UpdateRequest};
use crate::ops::memory::place::{clean_summary, scope_patch};
use crate::ops::memory::stores_for_read;
use crate::ops::memory::write::guard_user_voice;
use chrono::{SubsecRound, Utc};
use serde_json::Value;
use std::sync::Arc;

pub async fn update(state: &Memory, req: UpdateRequest) -> Result<Value, MemError> {
    let rewrites = req.content.is_some();
    let outcome_patch = match (req.outcome, req.clear_outcome) {
        (Some(o), _) => Some(Some(o)),
        (None, true) => Some(None),
        (None, false) => None,
    };
    // `~/…` expands; a relative path is no directory and is refused.
    let scope_dir = match req.scope_dir.filter(|d| !d.trim().is_empty()) {
        Some(d) => Some(
            crate::memory::scope::expand(&d, &crate::memory::scope::home())
                .ok_or_else(|| {
                    MemError::bad_request(format!(
                        "scope {d:?} is not an absolute directory (or ~/…)"
                    ))
                })?
                .to_string_lossy()
                .to_string(),
        ),
        None => None,
    };
    let scope_patch = scope_patch(scope_dir, req.clear_scope, req.global);
    let host_patch = match (req.host, req.clear_host) {
        (Some(v), _) => Some(Some(v)),
        (None, true) => Some(None),
        (None, false) => None,
    };
    let occurred_patch = match (req.occurred_at, req.clear_occurred_at) {
        (Some(v), _) => Some(Some(v)),
        (None, true) => Some(None),
        (None, false) => None,
    };

    let summary_patch = match (clean_summary(req.summary), req.clear_summary) {
        (Some(h), _) => Some(Some(h)),
        (None, true) => Some(None),
        (None, false) => None,
    };
    let mut patch = MemoryPatch {
        content: req.content,
        summary: summary_patch,
        indexed: req.indexed,
        r#type: req.r#type,
        tier: req.tier,
        origin: req.from,
        outcome: outcome_patch,
        scope: scope_patch,
        host: host_patch,
        occurred_at: occurred_patch,
        ..Default::default()
    };

    // Locate which table currently holds the row, so a tier patch that
    // crosses the semantic/episodic boundary can be honored by moving the
    // row across tables instead of leaving a tier=episodic row stranded
    // in the semantic table (or vice-versa).
    let mut located: Option<(Arc<MemoryStore>, Row)> = None;
    for store in stores_for_read(state, req.episodic) {
        if let Some(fact) = store.get(&req.id).await? {
            located = Some((store, fact));
            break;
        }
    }
    let Some((current_store, existing)) = located else {
        return Err(MemError::not_found(format!("no fact with id {}", req.id)));
    };
    if !req.scope.scope().admits(&existing) {
        return Err(MemError::not_found(format!("no fact with id {}", req.id)));
    }
    // A content rewrite of an existing row is the same violation class
    // as an add-with-replace_ids; metadata-only patches stay unguarded.
    // Checked after the account, so the guard never quotes another
    // person's row.
    if rewrites {
        guard_user_voice(state, std::slice::from_ref(&req.id), req.user_directed).await?;
    }

    let target_tier = patch.tier.unwrap_or(existing.tier);
    // Core is who the person is: no scope, no index — ever.
    if target_tier == Tier::Core {
        patch.scope = Some(None);
        patch.indexed = Some(false);
    }
    let target_is_episodic = matches!(target_tier, Tier::Episodic);
    let current_is_episodic = Arc::ptr_eq(&current_store, &state.episodic);

    if current_is_episodic == target_is_episodic {
        // Same-table update — delete+reinsert inside one store.
        let updated = current_store
            .update(&req.id, &patch)
            .await?
            .ok_or_else(|| MemError::not_found(format!("no fact with id {}", req.id)))?;
        return Ok(fact_public(&updated));
    }

    // Cross-table move: apply patch in memory, delete from source, insert
    // into target. Preserves the id and the existing embedding vector.
    let target_store = if target_is_episodic {
        Arc::clone(&state.episodic)
    } else {
        Arc::clone(&state.store)
    };
    let mut moved = existing;
    patch.apply(&mut moved);
    moved.tier = target_tier;
    moved.updated_at = Some(Utc::now().trunc_subsecs(6));
    current_store.delete(&req.id).await?;
    target_store.insert(std::slice::from_ref(&moved)).await?;
    Ok(fact_public(&moved))
}
