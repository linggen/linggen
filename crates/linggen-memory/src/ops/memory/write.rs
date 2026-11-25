//! The add path: one row (`add`) or an import (`add_batch`) — the
//! person check on `replace_ids`, the user-voice guard, cross-tier dedup,
//! and retiring the rows a write replaces.

use crate::error::MemError;
use crate::handle::Memory;
use crate::memory::Memory as Row;
use crate::memory::{AccountScope, InsertOutcome, MemoryType, Tier};
use crate::ops::memory::dto::{fact_public, AddBatchRequest, AddRequest};
use crate::ops::memory::place::{
    clean_summary, lock_tier, resolve_scope, resolve_tier, tier_rank, written_scope,
};
use crate::ops::memory::{pick_store, stores_for_read};
use serde_json::{json, Value};
use std::sync::Arc;

pub(in crate::ops) async fn add(state: &Memory, req: AddRequest) -> Result<Value, MemError> {
    if req.content.trim().is_empty() {
        return Err(MemError::bad_request("content must not be empty"));
    }
    let account_id = req.account_id.clone().filter(|a| !a.trim().is_empty());
    // The rows this one replaces: their tier and scope carry over. A
    // person may name only their own rows; that is settled before the
    // user-voice guard reads any row, and before anything is written.
    let losers = locate_rows(state, &req.replace_ids, account_id.as_deref()).await?;
    guard_user_voice(state, &req.replace_ids, req.user_directed).await?;

    let skip_dedup = req.skip_dedup;
    let replace_ids = req.replace_ids.clone();
    let asked = resolve_tier(req.tier, req.episodic, &losers, req.indexed);
    let ty = req.r#type.unwrap_or(MemoryType::Fact);
    let from = req.from.unwrap_or_default();
    // A dream promotion writes words already staged in episodic.
    let promoting = asked != Tier::Episodic
        && state
            .episodic
            .find_exact_content_public(
                &req.content,
                ty,
                &replace_ids,
                &AccountScope::for_dedup(account_id.as_deref()),
            )
            .await?
            .is_some();
    let (tier, moved) = lock_tier(asked, ty, from, &req.content, promoting)?;
    let scope = resolve_scope(&req, &losers, tier);
    let mut fact = Row::new(req.content, ty, from);
    fact.tier = tier;
    fact.outcome = req.outcome;
    fact.scope = scope;
    fact.summary = clean_summary(req.summary);
    fact.indexed = req.indexed && tier != Tier::Core;
    fact.occurred_at = req.occurred_at;
    fact.source_session = req.source_session;
    fact.host = req.host;
    fact.account_id = account_id;
    fact.account_name = req.account_name.filter(|a| !a.trim().is_empty());

    // Embed the content so the row is immediately searchable. Serialized +
    // off the async workers so concurrent adds can't stack forward passes.
    let vector = state
        .embedder()
        .await?
        .embed_passage(fact.content.clone())
        .await
        .map_err(MemError::internal)?;
    fact.vector = Some(vector);

    let mut data = write_row(state, fact, &replace_ids, skip_dedup).await?;
    if moved {
        data["placed"] = json!("episodic: progress, status and built/fixed/tried rows are staged there");
    }
    Ok(data)
}

/// Write an embedded row: as given (`skip_dedup`), or merged into an exact
/// duplicate of the same person (the other table first, then its own);
/// then retire `replace_ids`. Returns the add verb's `data`.
async fn write_row(
    state: &Memory,
    fact: Row,
    replace_ids: &[String],
    skip_dedup: bool,
) -> Result<Value, MemError> {
    let episodic = fact.tier == Tier::Episodic;
    let store = pick_store(state, episodic);
    if skip_dedup {
        store.insert(std::slice::from_ref(&fact)).await?;
        let body = json!({
            "action": "added",
            "fact": fact_public(&fact),
        });
        return Ok(apply_replace_ids(state, replace_ids, &fact.id, body).await);
    }

    // Cross-tier dedup. The single-table `insert_with_dedup` below only
    // catches duplicates inside the chosen store; without this step, a
    // semantic copy and an episodic copy of byte-identical (content, type)
    // would coexist. Tier rank: Core > Semantic > Episodic. The higher-
    // tier row wins; on equal rank (e.g. both semantic) the in-table
    // dedup handles it. See linggen `agents/ling-mem.md` tier-discipline.
    // A person's write meets only that person's rows, here and in-table.
    let whose = AccountScope::for_dedup(fact.account_id.as_deref());
    let other = if episodic {
        Arc::clone(&state.store)
    } else {
        Arc::clone(&state.episodic)
    };
    let lower_copy = other
        .find_exact_content_public(&fact.content, fact.r#type, replace_ids, &whose)
        .await?;
    if let Some(existing) = &lower_copy {
        let new_rank = tier_rank(fact.tier);
        let existing_rank = tier_rank(existing.tier);
        if existing_rank >= new_rank {
            // Existing row is at the same or higher tier — keep it. Fill
            // what it lacks from the new write, then return as if dedup'd.
            let merged = merge_with_existing(existing, &fact);
            other.update_full_public(&existing.id, &merged).await?;
            let body = json!({
                "action": "merged",
                "similarity": 1.0,
                "previous_id": existing.id,
                "fact": fact_public(&merged),
            });
            return Ok(apply_replace_ids(state, replace_ids, &existing.id, body).await);
        }
        // New row is at a higher tier — promote: write it below, and only
        // then delete the lower-tier copy (a failure between leaves two
        // copies, never none).
    }

    // Never into a row this call replaces: that row is about to be retired.
    let outcome = store.insert_with_dedup_except(fact, replace_ids).await?;
    if let Some(lower) = lower_copy {
        let _ = other.delete(&lower.id).await?;
    }
    let survivor = match &outcome {
        crate::memory::InsertOutcome::Added(f) => f.id.clone(),
        crate::memory::InsertOutcome::Merged { fact, .. } => fact.id.clone(),
    };
    Ok(apply_replace_ids(state, replace_ids, &survivor, outcome_public(&outcome)).await)
}

/// The rows `replace_ids` names, wherever they live. The owner's write
/// (`person` absent) skips missing ids. A person's write must name only
/// their own rows: any other id — missing, or another person's — refuses
/// the whole call, with the same answer either way.
async fn locate_rows(
    state: &Memory,
    ids: &[String],
    person: Option<&str>,
) -> Result<Vec<Row>, MemError> {
    let mut found = Vec::new();
    for id in ids {
        let row = locate_row(state, id).await?;
        match (row, person) {
            (Some(row), None) => found.push(row),
            (None, None) => {}
            (Some(row), Some(p)) if AccountScope::Id(p.to_string()).admits(&row) => found.push(row),
            (_, Some(_)) => return Err(MemError::not_found(format!("no fact with id {id}"))),
        }
    }
    Ok(found)
}

/// One row by id, semantic table first.
async fn locate_row(state: &Memory, id: &str) -> Result<Option<Row>, MemError> {
    for store in stores_for_read(state, None) {
        if let Some(row) = store.get(id).await? {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

/// Bulk insert N rows in one call: build every row, batch-embed all
/// contents (one gate acquisition), then commit each table's rows with a
/// single `MemoryStore::insert` (one LanceDB version per table, not per
/// row). Empty-content rows are skipped rather than failing the batch.
pub(in crate::ops) async fn add_batch(state: &Memory, req: AddBatchRequest) -> Result<Value, MemError> {
    // Partition by target table up front so each table commits exactly once.
    let mut semantic: Vec<Row> = Vec::new();
    let mut episodic: Vec<Row> = Vec::new();
    // Blanks are skipped; one doesn't abort the whole import.
    let mut rows = Vec::new();
    for r in req.facts {
        rows.extend(batch_row(r)?);
    }
    for fact in rows {
        if fact.tier == Tier::Episodic {
            episodic.push(fact);
        } else {
            semantic.push(fact);
        }
    }

    let mut added = Vec::new();
    for (mut rows, is_episodic) in [(semantic, false), (episodic, true)] {
        if rows.is_empty() {
            continue;
        }
        // One serialized batch embed for the whole group (chunked to the
        // embedder's MAX_EMBED_BATCH internally), then one commit.
        let texts: Vec<String> = rows.iter().map(|f| f.content.clone()).collect();
        let vectors = state
            .embedder()
            .await?
            .embed_passages(texts)
            .await
            .map_err(MemError::internal)?;
        for (f, v) in rows.iter_mut().zip(vectors) {
            f.vector = Some(v);
        }
        pick_store(state, is_episodic).insert(&rows).await?;
        added.extend(rows.iter().map(fact_public));
    }

    Ok(json!({
        "action": "added",
        "count": added.len(),
        "facts": added,
    }))
}

/// One batch-imported row, stamped with the person it was written for;
/// `None` for blank content.
fn batch_row(r: AddRequest) -> Result<Option<Row>, MemError> {
    if r.content.trim().is_empty() {
        return Ok(None);
    }
    let ty = r.r#type.unwrap_or(MemoryType::Fact);
    let from = r.from.unwrap_or_default();
    let asked = resolve_tier(r.tier, r.episodic, &[], r.indexed);
    // An import has no staged copy to promote from.
    let (tier, _) = lock_tier(asked, ty, from, &r.content, false)?;
    let mut fact = Row::new(r.content, ty, from);
    fact.tier = tier;
    fact.outcome = r.outcome;
    fact.scope = written_scope(r.scope.or(r.cwd), r.global || tier == Tier::Core);
    fact.summary = clean_summary(r.summary);
    fact.indexed = r.indexed && tier != Tier::Core;
    fact.occurred_at = r.occurred_at;
    fact.source_session = r.source_session;
    fact.host = r.host;
    fact.account_id = r.account_id.filter(|a| !a.trim().is_empty());
    fact.account_name = r.account_name.filter(|a| !a.trim().is_empty());
    Ok(Some(fact))
}

/// The merge law's floor, enforced at the store: a write that replaces
/// (`add` + `replace_ids`) or rewrites (`update` + `content`) rows in the
/// USER'S VOICE (`from=user`) is refused unless the caller asserts
/// `user_directed: true` — the user's current message states the change
/// as settled, or the host resolved the conflict with the user via an
/// ask. Hosts justify the flag; the daemon guarantees no silent
/// user-voice rewrite reaches the store regardless of which frontend
/// wrote it (the Linggen engine has its own pre-flight guard and
/// forwards the flag; CLI/MCP hosts pass it directly). Derived rows
/// pass free. A missing id is not the guard's problem — the real call
/// reports it.
pub(super) async fn guard_user_voice(
    state: &Memory,
    target_ids: &[String],
    user_directed: bool,
) -> Result<(), MemError> {
    if user_directed || target_ids.is_empty() {
        return Ok(());
    }
    let mut offending = Vec::new();
    for id in target_ids {
        for store in [Arc::clone(&state.store), Arc::clone(&state.episodic)] {
            if let Some(fact) = store.get(id).await? {
                if fact.origin == crate::memory::Origin::User {
                    let gist: String = fact.content.chars().take(60).collect();
                    offending.push(format!("{id} \"{gist}\""));
                }
                break;
            }
        }
    }
    if offending.is_empty() {
        return Ok(());
    }
    Err(MemError::bad_request(format!(
        "BLOCKED by the user-voice merge guard: this write replaces/rewrites rows in the USER'S VOICE (from=user): {}. The user's voice changes only with the user (merge law). Recovery — pick ONE: (1) if the user's CURRENT message states this change as SETTLED — a command (\"update X to Y\", \"forget X\"), a declaration (\"my X is now Y\"), or a commitment (\"from now on, X\") — retry the exact same call with \"user_directed\": true; a hedged reflection (\"X feels about right to me\") does NOT qualify; (2) otherwise ask the user to choose (the host's ask-user primitive, or a plain question in chat), then retry with \"user_directed\": true after their answer. Do NOT work around it by writing a new row without replace_ids — that creates the drift this guard exists to prevent.",
        offending.join(", ")
    )))
}

/// Apply the caller's `replace_ids` deletion sweep across both tables and
/// fold the outcome into `body`.
///
/// `replace_ids` are the losers the user picked against via AskUser; they
/// are deleted regardless of which tier they live in (the caller doesn't
/// track tier). Failures are surfaced (`replaced_failed`) but never abort
/// the add — the insert is the load-bearing operation.
///
/// This runs on **every** add return path (added / merged / promoted). It
/// used to be inline before only the final return, so an AskUser-resolved
/// conflict whose winner happened to dedup-merge into an existing row would
/// silently keep all the losers. Hoisting it into a shared helper closes
/// that gap.
/// Retire the losers of a merge. Semantic-table losers are EXPIRED, not
/// deleted (2026-08-17, Hanli's call): stamped `expired_at` +
/// `superseded_by = successor`, they leave every default read but stay on
/// disk — a merge or digest is reversible via the unpack query
/// (`superseded_by` filter). Episodic losers stay hard-deleted: staging
/// is disposable by design, and an immortal staged row would outlive the
/// sweep that exists to kill it.
async fn apply_replace_ids(
    state: &Memory,
    replace_ids: &[String],
    successor: &str,
    mut body: serde_json::Value,
) -> serde_json::Value {
    if replace_ids.is_empty() {
        return body;
    }
    let mut replaced = Vec::new();
    let mut replaced_failed = Vec::new();
    for id in replace_ids {
        // The row this write landed in is never its own loser.
        if id == successor {
            continue;
        }
        let retired = state.store.expire(id, successor).await.unwrap_or(false)
            || state.episodic.delete(id).await.unwrap_or(false);
        if retired {
            replaced.push(id.clone());
        } else {
            replaced_failed.push(id.clone());
        }
    }
    if let Some(obj) = body.as_object_mut() {
        obj.insert("replaced".to_string(), json!(replaced));
        if !replaced_failed.is_empty() {
            obj.insert("replaced_failed".to_string(), json!(replaced_failed));
        }
    }
    body
}

/// Same merge logic as `store::merge_fact` but at the HTTP layer so we
/// can compose it with a cross-store update. Takes the longer content and
/// fills missing optional fields from the candidate. The surviving row's
/// `scope` and summary stay unless empty: a fact re-said from elsewhere
/// must not leave the directory it is about.
fn merge_with_existing(
    existing: &crate::memory::Memory,
    candidate: &crate::memory::Memory,
) -> crate::memory::Memory {
    let mut merged = existing.clone();
    if candidate.content.len() > existing.content.len() {
        merged.content = candidate.content.clone();
        merged.vector = candidate.vector.clone();
    }
    if candidate.outcome.is_some() {
        merged.outcome = candidate.outcome;
    }
    if merged.scope.is_none() && merged.tier != crate::memory::Tier::Core {
        merged.scope = candidate.scope.clone();
    }
    if merged.summary.is_none() {
        merged.summary = candidate.summary.clone();
    }
    merged.indexed |= candidate.indexed;
    if candidate.occurred_at.is_some() {
        merged.occurred_at = candidate.occurred_at;
    }
    if candidate.source_session.is_some() {
        merged.source_session = candidate.source_session.clone();
    }
    if candidate.host.is_some() {
        merged.host = candidate.host.clone();
    }
    merged
}

/// Wrap an [`InsertOutcome`] as the JSON payload returned by the add
/// endpoint. Always includes `action` and `fact`; on a merge also
/// includes `similarity` and `previous_id`.
fn outcome_public(outcome: &InsertOutcome) -> Value {
    match outcome {
        InsertOutcome::Added(f) => json!({
            "action": "added",
            "fact": fact_public(f),
        }),
        InsertOutcome::Merged {
            fact,
            similarity,
            previous_id,
        } => json!({
            "action": "merged",
            "similarity": similarity,
            "previous_id": previous_id,
            "fact": fact_public(fact),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Origin;

    async fn scratch_state(dir: &std::path::Path) -> std::sync::Arc<Memory> {
        crate::handle::scratch_state(dir, std::net::SocketAddr::from(([127, 0, 0, 1], 0))).await
    }

    fn row_of(content: &str, account: Option<&str>, tier: Tier) -> Row {
        let mut m = Row::new(content, MemoryType::Fact, Origin::User);
        m.account_id = account.map(str::to_string);
        m.tier = tier;
        m
    }

    /// The 2026-10-05 loss: `memory_add` merged into the row it was told to
    /// replace, then retired that same row — nothing live was left. The row
    /// a write lands in is never one of its losers; the others still retire.
    #[tokio::test]
    async fn a_replacement_never_retires_the_row_it_wrote_into() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = scratch_state(dir.path()).await;
        let kept = Row::new("kept", MemoryType::Fact, Origin::Derived);
        let other = Row::new("other", MemoryType::Fact, Origin::Derived);
        state
            .store
            .insert(&[kept.clone(), other.clone()])
            .await
            .unwrap();

        let ids = vec![kept.id.clone(), other.id.clone()];
        let body = apply_replace_ids(&state, &ids, &kept.id, json!({})).await;
        assert_eq!(body["replaced"], json!([other.id]));
        assert!(body.get("replaced_failed").is_none(), "{body}");

        let live = state.store.get(&kept.id).await.unwrap().unwrap();
        assert!(live.expired_at.is_none(), "the survivor stays live");
        let gone = state.store.get(&other.id).await.unwrap().unwrap();
        assert!(gone.expired_at.is_some());
        assert_eq!(gone.superseded_by.as_deref(), Some(kept.id.as_str()));
    }

    /// A person naming their own row gets past the check (and on to the
    /// embedder, which this test state does not have).
    #[tokio::test]
    async fn a_person_may_replace_their_own_row() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = scratch_state(dir.path()).await;
        let alex = Row::new("Alex prefers tea", MemoryType::Fact, Origin::Derived);
        let alex = Row {
            account_id: Some("u-alex".into()),
            ..alex
        };
        state.store.insert(&[alex.clone()]).await.unwrap();
        let losers = locate_rows(&state, &[alex.id.clone()], Some("u-alex"))
            .await
            .unwrap();
        assert_eq!(losers.len(), 1);
        // The owner's write: a missing id is skipped, any row is found.
        let losers = locate_rows(&state, &[alex.id.clone(), "nope".into()], None)
            .await
            .unwrap();
        assert_eq!(losers.len(), 1);
    }

    /// Exact duplicates of another person's row, in the same table and
    /// across tables, become rows of their own; the other row is not
    /// merged into, promoted over or deleted.
    #[tokio::test]
    async fn a_duplicate_of_another_persons_row_is_a_row_of_its_own() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = scratch_state(dir.path()).await;
        let same_tier = row_of("same words", Some("u-alex"), Tier::Semantic);
        let staged = row_of("staged words", Some("u-alex"), Tier::Episodic);
        state.store.insert(&[same_tier.clone()]).await.unwrap();
        state.episodic.insert(&[staged.clone()]).await.unwrap();

        let cases = [
            ("same words", Tier::Semantic),
            ("same words", Tier::Episodic),
            ("staged words", Tier::Episodic),
            ("staged words", Tier::Semantic),
        ];
        // A writer per case: each one's first copy is a row of its own.
        for (i, (content, tier)) in cases.into_iter().enumerate() {
            let mine = row_of(content, Some(&format!("u-other-{i}")), tier);
            let data = write_row(&state, mine.clone(), &[], false).await.unwrap();
            assert_eq!(data["action"], "added", "{content} {tier:?}: {data}");
            assert_eq!(data["fact"]["id"], mine.id.as_str());
        }

        let a = state.store.get(&same_tier.id).await.unwrap().unwrap();
        assert_eq!(
            (a.content.as_str(), a.account_id.as_deref()),
            ("same words", Some("u-alex"))
        );
        assert!(a.expired_at.is_none());
        let b = state.episodic.get(&staged.id).await.unwrap().unwrap();
        assert_eq!(b.account_id.as_deref(), Some("u-alex"));
        assert_eq!(state.store.count().await.unwrap(), 3);
        assert_eq!(state.episodic.count().await.unwrap(), 3);
    }

    /// The owner's write deduplicates and replaces as it always has.
    #[tokio::test]
    async fn the_owners_write_still_merges_and_replaces() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = scratch_state(dir.path()).await;
        let mut kept = row_of("owner words", None, Tier::Semantic);
        kept.origin = Origin::Derived;
        let mut staged = row_of("staged words", None, Tier::Episodic);
        staged.origin = Origin::Derived;
        let mut loser = row_of("old words", None, Tier::Semantic);
        loser.origin = Origin::Derived;
        state
            .store
            .insert(&[kept.clone(), loser.clone()])
            .await
            .unwrap();
        state.episodic.insert(&[staged.clone()]).await.unwrap();

        let again = row_of("owner words", None, Tier::Semantic);
        let data = write_row(&state, again, &[loser.id.clone()], false)
            .await
            .unwrap();
        assert_eq!(data["action"], "merged", "{data}");
        assert_eq!(data["previous_id"], kept.id.as_str());
        assert_eq!(data["replaced"], json!([loser.id]));

        // Cross tier: a long-term copy promotes over the staged one.
        let promoted = row_of("staged words", None, Tier::Semantic);
        let data = write_row(&state, promoted, &[], false).await.unwrap();
        assert_eq!(data["action"], "added", "{data}");
        assert!(state.episodic.get(&staged.id).await.unwrap().is_none());
    }

    /// A promotion writes the long-term copy before it deletes the staged
    /// one: when that write fails, the staged row is still there.
    #[tokio::test]
    async fn a_failed_promotion_keeps_the_staged_copy() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = scratch_state(dir.path()).await;
        let mut staged = row_of("staged words", None, Tier::Episodic);
        staged.origin = Origin::Derived;
        state.episodic.insert(&[staged.clone()]).await.unwrap();

        let mut promoted = row_of("staged words", None, Tier::Semantic);
        promoted.origin = Origin::Derived;
        // Not the table's vector width: the long-term write fails.
        promoted.vector = Some(vec![0.5; 3]);
        assert!(write_row(&state, promoted, &[], false).await.is_err());
        assert!(state.episodic.get(&staged.id).await.unwrap().is_some());
    }

    /// A batch row carries the person it was written for — a person's
    /// import is never filed as the owner's — and a batch never merges.
    #[test]
    fn a_batch_row_is_the_persons_it_was_written_for() {
        let req: AddBatchRequest = serde_json::from_value(json!({"facts": [
            {"content": "Alex prefers tea", "account_id": "u-other", "account_name": "Sam"},
            {"content": "  "},
            {"content": "owner row", "tier": "core"},
        ]}))
        .unwrap();
        let rows: Vec<Row> = req
            .facts
            .into_iter()
            .filter_map(|r| batch_row(r).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].account_id.as_deref(), Some("u-other"));
        assert_eq!(rows[0].account_name.as_deref(), Some("Sam"));
        assert_eq!(rows[1].account_id, None);
    }

    async fn call_add(state: &Arc<Memory>, verb: &str, args: Value) -> Result<Value, MemError> {
        state.call(&crate::Who::Owner, verb, args).await?.into_result()
    }

    /// The tier-lock: built / fixed / tried never land in semantic through
    /// `add` or `add_batch` (MCP, REST and CLI all call these verbs).
    #[tokio::test]
    async fn built_fixed_tried_never_land_in_semantic() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::hosted::open_quiet_fake(dir.path(), String::new()).await.unwrap();
        for ty in ["built", "fixed", "tried"] {
            let r = call_add(&state, "add", json!({"content": format!("{ty} one"), "type": ty, "tier": "semantic", "from": "agent"}))
                .await
                .unwrap();
            assert!(r["placed"].is_string(), "{ty}: {r}");
            assert_eq!(r["fact"]["tier"], "episodic", "{ty}");
        }
        let r = call_add(&state, "add_batch", json!({"facts": [
            {"content": "batch built", "type": "built", "tier": "semantic"},
            {"content": "batch core fixed", "type": "fixed", "tier": "core"},
        ]})).await.unwrap();
        assert_eq!(r["count"], 2);
        assert!(r["facts"].as_array().unwrap().iter().all(|f| f["tier"] == "episodic"));
        assert_eq!(state.store.count().await.unwrap(), 0);
        // A replacement of a semantic status row also stays episodic.
        let old = call_add(&state, "add", json!({"content": "old status", "tier": "semantic"})).await.unwrap();
        let id = old["fact"]["id"].as_str().unwrap().to_string();
        let r = call_add(&state, "add", json!({"content": "new status", "type": "built", "replace_ids": [id]})).await.unwrap();
        assert_eq!(r["fact"]["tier"], "episodic");
        // The user's words keep their tier.
        let r = call_add(&state, "add", json!({"content": "I shipped it", "type": "built", "tier": "semantic", "from": "user"})).await.unwrap();
        assert_eq!(r["fact"]["tier"], "semantic");
    }

    /// Semantic and core refuse content over 600 characters (not the user's).
    #[tokio::test]
    async fn semantic_and_core_are_capped_at_600_chars() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::hosted::open_quiet_fake(dir.path(), String::new()).await.unwrap();
        let long = "x".repeat(601);
        for tier in ["semantic", "core"] {
            let e = call_add(&state, "add", json!({"content": long, "tier": tier})).await.unwrap_err();
            assert!(e.message.contains("too long for semantic"), "{e:?}");
        }
        let e = call_add(&state, "add_batch", json!({"facts": [{"content": long, "tier": "semantic"}]})).await;
        assert!(e.is_err());
        assert!(call_add(&state, "add", json!({"content": "x".repeat(600), "tier": "semantic"})).await.is_ok());
        assert!(call_add(&state, "add", json!({"content": long, "tier": "episodic"})).await.is_ok());
        assert!(call_add(&state, "add", json!({"content": long, "tier": "semantic", "from": "user"})).await.is_ok());
    }

    /// A dream promotion (the same words already staged) may keep a
    /// built row in semantic, but still obeys the cap.
    #[tokio::test]
    async fn a_promotion_may_reach_semantic_within_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::hosted::open_quiet_fake(dir.path(), String::new()).await.unwrap();
        call_add(&state, "add", json!({"content": "staged build", "type": "built"})).await.unwrap();
        let r = call_add(&state, "add", json!({"content": "staged build", "type": "built", "tier": "semantic"})).await.unwrap();
        assert_eq!(r["fact"]["tier"], "semantic", "{r}");
        let long = "y".repeat(601);
        call_add(&state, "add", json!({"content": long, "type": "built"})).await.unwrap();
        assert!(call_add(&state, "add", json!({"content": long, "type": "built", "tier": "semantic"})).await.is_err());
    }
}
