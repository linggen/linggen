//! `update`: patch one row by id — set/clear twins, `~` expansion, core
//! forcing no scope and no index, and the move across tables when the tier
//! patch crosses the semantic/episodic line.

use crate::error::MemError;
use crate::handle::Memory;
use crate::memory::Memory as Row;
use crate::memory::{MemoryPatch, MemoryStore, Tier};
use crate::ops::memory::dto::{fact_public, UpdateRequest};
use crate::ops::memory::place::{check_cap, clean_summary, scope_patch};
use crate::ops::memory::stores_for_read;
use crate::ops::memory::write::guard_user_voice;
use chrono::{SubsecRound, Utc};
use serde_json::Value;
use std::sync::Arc;

pub(in crate::ops) async fn update(state: &Memory, req: UpdateRequest) -> Result<Value, MemError> {
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
    // New words get a new vector in this same update, by the same warm-up
    // rule as a write; the same words again (or no content) embed nothing.
    if let Some(content) = patch.content.as_ref().filter(|c| **c != existing.content) {
        let vector = state
            .embedder()
            .await?
            .embed_passage(content.clone())
            .await
            .map_err(MemError::internal)?;
        patch.vector = Some(Some(vector));
    }

    let target_tier = patch.tier.unwrap_or(existing.tier);
    // Core is who the person is: no scope, no index — ever.
    if target_tier == Tier::Core {
        patch.scope = Some(None);
        patch.indexed = Some(false);
    }
    // A promotion by update may reach semantic, but within the cap.
    // Metadata-only patches leave an old long row alone.
    if patch.content.is_some() || target_tier != existing.tier {
        check_cap(
            target_tier,
            patch.origin.unwrap_or(existing.origin),
            patch.content.as_deref().unwrap_or(&existing.content),
        )?;
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

    // Cross-table move: apply patch in memory, insert into the target, then
    // delete from the source — a failure between leaves two copies, never
    // none. Preserves the id, and the vector unless the content changed.
    let target_store = if target_is_episodic {
        Arc::clone(&state.episodic)
    } else {
        Arc::clone(&state.store)
    };
    let mut moved = existing;
    patch.apply(&mut moved);
    moved.tier = target_tier;
    moved.updated_at = Some(Utc::now().trunc_subsecs(6));
    target_store.insert(std::slice::from_ref(&moved)).await?;
    current_store.delete(&req.id).await?;
    Ok(fact_public(&moved))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MemoryType, Origin};
    use std::path::Path;

    #[cfg(unix)]
    fn set_mode(dir: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        for entry in walkdir(dir) {
            std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(mode)).unwrap();
        }
    }

    /// Every directory under `dir`, `dir` itself last.
    #[cfg(unix)]
    fn walkdir(dir: &Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            if entry.file_type().unwrap().is_dir() {
                out.extend(walkdir(&entry.path()));
            }
        }
        out.push(dir.to_path_buf());
        out
    }

    /// A move between tables writes the target before it deletes the
    /// source: when the target cannot be written, the row stays where it was.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_move_whose_target_write_fails_keeps_the_source_row() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = crate::handle::scratch_state(
            dir.path(),
            std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        )
        .await;
        let mut staged = Row::new("Alex keeps bees", MemoryType::Fact, Origin::Derived);
        staged.tier = Tier::Episodic;
        state.episodic.insert(&[staged.clone()]).await.unwrap();

        let target = crate::memory::maintenance::db_dir(dir.path()).join("semantic.lance");
        set_mode(&target, 0o555);
        let req: UpdateRequest =
            serde_json::from_value(serde_json::json!({"id": staged.id, "tier": "semantic"}))
                .unwrap();
        let moved = update(&state, req).await;
        set_mode(&target, 0o755);

        assert!(moved.is_err(), "the target write was refused: {moved:?}");
        assert!(state.episodic.get(&staged.id).await.unwrap().is_some());
    }

    async fn call(state: &Arc<Memory>, verb: &str, args: serde_json::Value) -> Result<Value, MemError> {
        state.call(&crate::Who::Owner, verb, args).await?.into_result()
    }

    async fn patch(state: &Memory, args: serde_json::Value) -> Result<Value, MemError> {
        update(state, serde_json::from_value(args).unwrap()).await
    }

    /// A content change re-embeds the row in the same update — in its own
    /// table and when it moves — so search ranks it by what it says now.
    #[tokio::test]
    async fn a_content_change_re_embeds_the_row() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::hosted::open_quiet_fake(dir.path(), String::new())
            .await
            .unwrap();
        let embedder = state.embedder().await.unwrap();
        for (tier, to) in [("semantic", "semantic"), ("episodic", "semantic")] {
            let added = call(&state, "add", serde_json::json!({"content": "Alex keeps bees", "tier": tier}))
                .await
                .unwrap();
            let id = added["fact"]["id"].as_str().unwrap().to_string();
            let new = format!("Alex races sailboats ({tier} to {to})");
            call(&state, "update", serde_json::json!({"id": id, "content": new, "tier": to}))
                .await
                .unwrap();
            let row = state.store.get(&id).await.unwrap().unwrap();
            let want = Arc::clone(&embedder).embed_passage(new.clone()).await.unwrap();
            assert!(row.vector == Some(want), "{tier} → {to}: the old vector was kept");
        }
    }

    /// Only a content change embeds: with the model still warming, the
    /// verb answers a metadata patch (or the same words again) without the
    /// model, and a new content waits by the same warm-up rule a write
    /// does. (The verb itself, below the call's own warm-up gate.)
    #[tokio::test]
    async fn an_update_that_changes_no_content_does_not_embed() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::hosted::open_quiet_fake(dir.path(), String::new())
            .await
            .unwrap();
        let mut row = Row::new("Alex keeps bees", MemoryType::Fact, Origin::Derived);
        row.tier = Tier::Semantic;
        state.store.insert(&[row.clone()]).await.unwrap();
        let (release, warmup) = crate::hosted::hold_embedding_warmup(
            &state,
            std::time::Duration::from_millis(50),
        )
        .await;

        let meta = serde_json::json!({"id": row.id, "summary": "bees", "content": "Alex keeps bees"});
        assert!(patch(&state, meta).await.is_ok());
        let rewrite = serde_json::json!({"id": row.id, "content": "Alex races sailboats"});
        assert!(patch(&state, rewrite).await.is_err());
        assert_eq!(state.store.get(&row.id).await.unwrap().unwrap().content, "Alex keeps bees");

        release.send(()).unwrap();
        warmup.await.unwrap();
    }
}
