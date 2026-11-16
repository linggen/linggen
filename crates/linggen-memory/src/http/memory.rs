//! `/api/memory/<method>` — RPC-style endpoints, 1:1 with the `Memory.*`
//! tools in Linggen. Each endpoint POSTs JSON args and returns
//! `{ok, data}` or `{ok:false, error, code}` via `envelope::ApiError`.
//!
//! Semantics mirror the CLI handlers in `crate::cli` — this module is
//! the network-facing path to the same `MemoryStore` operations. Once
//! Phase 4 lands in Linggen, the CLI data-ops wrappers are removed and
//! HTTP becomes the only dispatch path.

use super::envelope::{ok, ApiError};
use super::state::SharedState;
use crate::memory::{
    AccountScope, Filters, Hit, InsertOutcome, Memory, MemoryPatch, MemoryStore, MemoryType,
    Tier,
};
pub(crate) use crate::ops::memory::dto::AddRequest;
use crate::ops::memory::dto::{
    fact_public, facts_public, scored_facts_public, AddBatchRequest, CountRequest,
    DeleteRequest, FilterDTO, ForgetRequest, GetRequest, ListRequest, RestampRequest,
    SearchRequest, SearchTable, UpdateRequest,
};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use chrono::{SubsecRound, Utc};
use serde_json::{json, Value};
use std::sync::Arc;

/// Memory subrouter. Mounted at `/api/memory/` by the parent router.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/api/memory/add", post(add))
        .route("/api/memory/add_batch", post(add_batch))
        .route("/api/memory/get", post(get))
        .route("/api/memory/search", post(search))
        .route("/api/memory/list", post(list))
        .route("/api/memory/count", post(count))
        .route("/api/memory/update", post(update))
        .route("/api/memory/delete", post(delete))
        .route("/api/memory/forget", post(forget))
        .route("/api/memory/restamp", post(restamp))
        .route("/api/memory/accounts", post(accounts))
}

impl FilterDTO {
    /// Fold the `day` sugar into `since`/`until`. Called by every
    /// handler before `into_filters` — a bad day string is a 400, not a
    /// silently-ignored filter.
    fn resolve_day(&mut self) -> Result<(), ApiError> {
        let Some(day) = self.day.take() else {
            return Ok(());
        };
        let date = chrono::NaiveDate::parse_from_str(day.trim(), "%Y-%m-%d").map_err(|_| {
            ApiError::bad_request(format!("invalid day {day:?}: expected YYYY-MM-DD"))
        })?;
        let (start, end) = crate::ops::days::local_day_bounds(date);
        if self.since.is_none() {
            self.since = Some(start);
        }
        if self.until.is_none() {
            self.until = Some(end);
        }
        Ok(())
    }

    fn into_filters(mut self) -> Result<Filters, ApiError> {
        self.resolve_day()?;
        let mut types = self.types;
        if let Some(t) = self.r#type {
            if !types.contains(&t) {
                types.push(t);
            }
        }
        let mut apps = self.apps;
        for a in self.contexts_any {
            if !apps.contains(&a) {
                apps.push(a);
            }
        }
        Ok(Filters {
            apps,
            indexed: self.indexed,
            scope_in: Vec::new(),
            no_skills: false,
            legacy_contexts: false,
            legacy_cwd: false,
            account: self.scope.scope(),
            types,
            exclude_types: self.exclude_types,
            origin: self.from,
            outcome: self.outcome,
            since: self.since,
            until: self.until,
            created_since: None,
            tier: self.tier,
            source_session: self.source_session,
            scope_root: self.scope_root,
            include_expired: self.include_expired,
            superseded_by: self.superseded_by,
        })
    }
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// Pick the semantic or episodic store based on the request's `episodic`
/// flag. Centralized so every CRUD endpoint routes consistently.
fn pick_store(state: &SharedState, episodic: bool) -> Arc<MemoryStore> {
    if episodic {
        Arc::clone(&state.episodic)
    } else {
        Arc::clone(&state.store)
    }
}

/// Resolve which store(s) a read/edit/delete should hit.
///   - `Some(true)`  → episodic only
///   - `Some(false)` → semantic only
///   - `None`        → both (caller decides how to merge)
///
/// Returned in the order semantic-first so locate-first-hit semantics
/// preserve the higher-tier preference when an id collision exists
/// across tables.
fn stores_for_read(state: &SharedState, episodic: Option<bool>) -> Vec<Arc<MemoryStore>> {
    match episodic {
        Some(true) => vec![Arc::clone(&state.episodic)],
        Some(false) => vec![Arc::clone(&state.store)],
        None => vec![Arc::clone(&state.store), Arc::clone(&state.episodic)],
    }
}

async fn add(
    State(state): State<SharedState>,
    Json(req): Json<AddRequest>,
) -> Result<Response, ApiError> {
    if req.content.trim().is_empty() {
        return Err(ApiError::bad_request("content must not be empty"));
    }
    let account_id = req.account_id.clone().filter(|a| !a.trim().is_empty());
    // The rows this one replaces: their tier and scope carry over. A
    // person may name only their own rows; that is settled before the
    // user-voice guard reads any row, and before anything is written.
    let losers = locate_rows(&state, &req.replace_ids, account_id.as_deref()).await?;
    guard_user_voice(&state, &req.replace_ids, req.user_directed).await?;

    let skip_dedup = req.skip_dedup;
    let replace_ids = req.replace_ids.clone();
    let tier = resolve_tier(req.tier, req.episodic, &losers, req.indexed);
    let scope = resolve_scope(&req, &losers, tier);
    let mut fact = Memory::new(
        req.content,
        req.r#type.unwrap_or(MemoryType::Fact),
        req.from.unwrap_or_default(),
    );
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
        .map_err(ApiError::internal)?;
    fact.vector = Some(vector);

    Ok(ok(write_row(&state, fact, &replace_ids, skip_dedup).await?))
}

/// Write an embedded row: as given (`skip_dedup`), or merged into an exact
/// duplicate of the same person (the other table first, then its own);
/// then retire `replace_ids`. Returns the add verb's `data`.
async fn write_row(
    state: &SharedState,
    fact: Memory,
    replace_ids: &[String],
    skip_dedup: bool,
) -> Result<Value, ApiError> {
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
    if let Some(existing) = other
        .find_exact_content_public(&fact.content, fact.r#type, replace_ids, &whose)
        .await?
    {
        let new_rank = tier_rank(fact.tier);
        let existing_rank = tier_rank(existing.tier);
        if existing_rank >= new_rank {
            // Existing row is at the same or higher tier — keep it. Fill
            // what it lacks from the new write, then return as if dedup'd.
            let merged = merge_with_existing(&existing, &fact);
            other.update_full_public(&existing.id, &merged).await?;
            let body = json!({
                "action": "merged",
                "similarity": 1.0,
                "previous_id": existing.id,
                "fact": fact_public(&merged),
            });
            return Ok(apply_replace_ids(state, replace_ids, &existing.id, body).await);
        }
        // New row is at a higher tier — promote: delete the lower-tier
        // copy from the other table, then proceed with single-table insert.
        let _ = other.delete(&existing.id).await?;
    }

    // Never into a row this call replaces: that row is about to be retired.
    let outcome = store.insert_with_dedup_except(fact, replace_ids).await?;
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
    state: &SharedState,
    ids: &[String],
    person: Option<&str>,
) -> Result<Vec<Memory>, ApiError> {
    let mut found = Vec::new();
    for id in ids {
        let row = locate_row(state, id).await?;
        match (row, person) {
            (Some(row), None) => found.push(row),
            (None, None) => {}
            (Some(row), Some(p)) if AccountScope::Id(p.to_string()).admits(&row) => found.push(row),
            (_, Some(_)) => return Err(ApiError::not_found(format!("no fact with id {id}"))),
        }
    }
    Ok(found)
}

/// One row by id, semantic table first.
async fn locate_row(state: &SharedState, id: &str) -> Result<Option<Memory>, ApiError> {
    for store in stores_for_read(state, None) {
        if let Some(row) = store.get(id).await? {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

/// The tier a write lands in. Asked-for wins (`episodic: true` or `tier`);
/// a replacement keeps its losers' highest tier; an indexed row is
/// long-term by nature; anything else is episodic — the protocol's default
/// per-turn capture, which the nightly dream promotes from.
fn resolve_tier(asked: Option<Tier>, episodic: bool, losers: &[Memory], indexed: bool) -> Tier {
    if episodic {
        return Tier::Episodic;
    }
    if let Some(t) = asked {
        return t;
    }
    if let Some(t) = losers.iter().map(|l| l.tier).max_by_key(|t| tier_rank(*t)) {
        return t;
    }
    if indexed {
        return Tier::Semantic;
    }
    Tier::Episodic
}

/// The scope a new row is stored with. Core rows and `global` rows have
/// none. A `scope` the model named wins when it resolves to a valid dir for
/// this session; a replacement with no named scope takes its losers' common
/// scope; otherwise the host's stamped session cwd, if it can be a scope.
fn resolve_scope(req: &AddRequest, losers: &[Memory], tier: Tier) -> Option<String> {
    if req.global || tier == Tier::Core {
        return None;
    }
    let home = crate::memory::scope::home();
    let session = req
        .cwd
        .as_deref()
        .and_then(|c| crate::memory::scope::expand(c, &home))
        .filter(|c| crate::memory::scope::is_scope_dir(c, &home));
    let root = req
        .root
        .as_deref()
        .and_then(|r| crate::memory::scope::expand(r, &home))
        .or_else(|| {
            session
                .as_deref()
                .map(|s| crate::memory::scope::find_root(s, &home))
        });
    if let Some(named) = req.scope.as_deref().filter(|s| !s.trim().is_empty()) {
        let chosen = match root.as_deref() {
            Some(r) => crate::memory::scope::resolve(named, Some(r), &home)
                .filter(|d| crate::memory::scope::is_valid_for(d, r, &home)),
            // No host stamp (Codex cannot rewrite tool input): nothing to
            // check the choice against, and falling back would store the row
            // with no scope — "about the person", recalled everywhere — the
            // opposite of what naming a directory says. An absolute (or ~/)
            // existing directory that can hold rows is taken as named.
            None => crate::memory::scope::resolve(named, None, &home)
                .filter(|d| d.is_dir() && crate::memory::scope::is_scope_dir(d, &home)),
        };
        if let Some(dir) = chosen {
            return Some(dir.to_string_lossy().to_string());
        }
    } else if !losers.is_empty() {
        let scopes: Vec<Option<String>> = losers.iter().map(|l| l.scope.clone()).collect();
        return crate::memory::scope::common(&scopes, &home);
    }
    session.map(|s| s.to_string_lossy().to_string())
}

/// The longest a summary may be, in characters.
const SUMMARY_MAX_CHARS: usize = 80;

/// One line, trimmed, at most [`SUMMARY_MAX_CHARS`] — cut with `…` beyond.
fn clean_summary(raw: Option<String>) -> Option<String> {
    let line = raw?.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    if line.chars().count() <= SUMMARY_MAX_CHARS {
        return Some(line);
    }
    let cut: String = line.chars().take(SUMMARY_MAX_CHARS - 1).collect();
    Some(format!("{}…", cut.trim_end()))
}

/// The `scope` a batch-imported row is stored with: `global` wins, an empty
/// string is no scope.
fn written_scope(scope: Option<String>, global: bool) -> Option<String> {
    if global {
        None
    } else {
        scope.filter(|c| !c.trim().is_empty())
    }
}

/// An update's `scope` change. `global` clears it and wins over a new value;
/// otherwise set wins over clear, and nothing given leaves it alone.
fn scope_patch(scope: Option<String>, clear: bool, global: bool) -> Option<Option<String>> {
    match (scope, clear) {
        _ if global => Some(None),
        (Some(v), _) => Some(Some(v)),
        (None, true) => Some(None),
        (None, false) => None,
    }
}

/// Bulk insert N rows in one call: build every row, batch-embed all
/// contents (one gate acquisition), then commit each table's rows with a
/// single `MemoryStore::insert` (one LanceDB version per table, not per
/// row). Empty-content rows are skipped rather than failing the batch.
async fn add_batch(
    State(state): State<SharedState>,
    Json(req): Json<AddBatchRequest>,
) -> Result<Response, ApiError> {
    // Partition by target table up front so each table commits exactly once.
    let mut semantic: Vec<Memory> = Vec::new();
    let mut episodic: Vec<Memory> = Vec::new();
    // Blanks are skipped; one doesn't abort the whole import.
    for fact in req.facts.into_iter().filter_map(batch_row) {
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
            .map_err(ApiError::internal)?;
        for (f, v) in rows.iter_mut().zip(vectors) {
            f.vector = Some(v);
        }
        pick_store(&state, is_episodic).insert(&rows).await?;
        added.extend(rows.iter().map(fact_public));
    }

    Ok(ok(json!({
        "action": "added",
        "count": added.len(),
        "facts": added,
    })))
}

/// One batch-imported row, stamped with the person it was written for;
/// `None` for blank content.
fn batch_row(r: AddRequest) -> Option<Memory> {
    if r.content.trim().is_empty() {
        return None;
    }
    let tier = resolve_tier(r.tier, r.episodic, &[], r.indexed);
    let mut fact = Memory::new(
        r.content,
        r.r#type.unwrap_or(MemoryType::Fact),
        r.from.unwrap_or_default(),
    );
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
    Some(fact)
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
async fn guard_user_voice(
    state: &SharedState,
    target_ids: &[String],
    user_directed: bool,
) -> Result<(), ApiError> {
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
    Err(ApiError::bad_request(format!(
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
    state: &SharedState,
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

/// Tier rank for the "higher wins on cross-tier dedup" rule.
/// Higher number = higher tier. Core (2) > Semantic (1) > Episodic (0).
fn tier_rank(tier: crate::memory::Tier) -> u8 {
    use crate::memory::Tier::*;
    match tier {
        Core => 2,
        Semantic => 1,
        Episodic => 0,
    }
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

async fn get(
    State(state): State<SharedState>,
    Json(req): Json<GetRequest>,
) -> Result<Response, ApiError> {
    let scope = req.scope.scope();
    for store in stores_for_read(&state, req.episodic) {
        if let Some(fact) = store.get(&req.id).await? {
            if !scope.admits(&fact) {
                break;
            }
            return Ok(ok(fact_public(&fact)));
        }
    }
    Err(ApiError::not_found(format!("no fact with id {}", req.id)))
}

async fn search(
    State(state): State<SharedState>,
    Json(req): Json<SearchRequest>,
) -> Result<Response, ApiError> {
    if req.query.trim().is_empty() {
        return Err(ApiError::bad_request("query must not be empty"));
    }

    let vector = state
        .embedder()
        .await?
        .embed_query_serialized(req.query.clone())
        .await
        .map_err(ApiError::internal)?;
    let filters = req.filters.into_filters()?;

    // Store-wide recall floor: when the client omits `min_score`, apply the
    // daemon's configured `recall_min_score` so every host shares one recall
    // selectivity. An explicit per-call `min_score` overrides. (Same
    // server-owns-it pattern as `episodic_ttl_days`.)
    let min_score = match req.min_score {
        Some(s) => Some(s),
        None => Some(
            crate::ops::config::load(&state.data_dir)
                .await
                .recall_min_score,
        ),
    };

    // Hybrid retrieval: each row scored as cosine + an IDF-weighted keyword
    // boost, so exact-keyword queries rank correctly without inventing
    // relevance for unrelated rows. `min_score` gates the hybrid score,
    // applied inside the fuse step (see crate::memory::hybrid).
    let table = match req.episodic {
        Some(true) => SearchTable::Episodic,
        Some(false) => SearchTable::Semantic,
        None => req.table,
    };
    let results = scored(
        &state, &table, &vector, &req.query, &filters, req.limit, min_score,
    )
    .await?;
    Ok(ok(scored_facts_public(&results)))
}

async fn scored(
    state: &SharedState,
    table: &SearchTable,
    vector: &[f32],
    query: &str,
    filters: &Filters,
    limit: usize,
    min_score: Option<f32>,
) -> Result<Vec<Hit>, ApiError> {
    Ok(match table {
        SearchTable::Both => {
            state
                .recall
                .query(vector, query, filters, limit, min_score)
                .await?
        }
        SearchTable::Semantic => {
            state
                .store
                .hybrid_scored(vector, query, filters, limit, min_score)
                .await?
        }
        SearchTable::Episodic => {
            state
                .episodic
                .hybrid_scored(vector, query, filters, limit, min_score)
                .await?
        }
    })
}

async fn list(
    State(state): State<SharedState>,
    Json(mut req): Json<ListRequest>,
) -> Result<Response, ApiError> {
    // Resolve `past_ttl: true` into a concrete `until` cutoff. The mission
    // body sends `past_ttl: true` so it doesn't have to hardcode the TTL
    // number; the daemon owns the live value and applies it here.
    // An explicit `until`/`older_than` from the caller wins (caller knows
    // best); we only fill in when nothing is set.
    if req.filters.past_ttl && req.filters.until.is_none() {
        let cfg = crate::ops::config::load(&state.data_dir).await;
        let cutoff = chrono::Utc::now() - chrono::Duration::days(cfg.episodic_ttl_days as i64);
        req.filters.until = Some(cutoff);
    }
    // `past_ttl` is an episodic-TTL concept and the tool schema promises
    // "implies tier=episodic" — scope the read accordingly unless the
    // caller explicitly asked for a table, so old semantic rows don't
    // masquerade as evictable staging.
    if req.filters.past_ttl && req.episodic.is_none() {
        req.episodic = Some(true);
    }
    let created_since = unjudged_since(&state, &req).await?;
    let mut filters = req.filters.into_filters()?;
    filters.created_since = created_since;
    let sort = req.sort.into();
    let mut combined = Vec::new();
    // For each in-scope store, pull `limit + offset` rows so the post-
    // merge sort+page still has enough material; bounded by the per-
    // store list cap.
    let take = req.limit.saturating_add(req.offset);
    for store in stores_for_read(&state, req.episodic) {
        let rows = store.list(&filters, sort, take, 0).await?;
        combined.extend(rows);
    }
    sort_combined(&mut combined, sort);
    let paged: Vec<_> = combined
        .into_iter()
        .skip(req.offset)
        .take(req.limit)
        .collect();
    Ok(ok(facts_public(&paged)))
}

/// `unjudged` resolved to its cutoff: the listed day's `remembered_at`.
/// The flag names a day's worklist, so it needs `day`.
async fn unjudged_since(
    state: &SharedState,
    req: &ListRequest,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, ApiError> {
    if !req.unjudged {
        return Ok(None);
    }
    let Some(day) = req.filters.day.as_deref() else {
        return Err(ApiError::bad_request("unjudged needs a day"));
    };
    let days = crate::ops::days::load(&state.data_dir).await;
    Ok(days.days.get(day.trim()).and_then(|r| r.remembered_at))
}

fn sort_combined(rows: &mut [crate::memory::Memory], sort: crate::memory::SortOrder) {
    use crate::memory::SortOrder::*;
    // Match the store-layer order (and the UI's age badge): sort by
    // activity_timestamp (updated_at ?? created_at), NOT effective_timestamp.
    // effective_timestamp prefers the consolidator's back-dated occurred_at,
    // which buries freshly-written rows below their save date.
    rows.sort_by(|a, b| match sort {
        Newest => b.activity_timestamp().cmp(&a.activity_timestamp()),
        Oldest => a.activity_timestamp().cmp(&b.activity_timestamp()),
    });
}

async fn count(
    State(state): State<SharedState>,
    Json(req): Json<CountRequest>,
) -> Result<Response, ApiError> {
    let filters = req.filters.into_filters()?;
    let stores = stores_for_read(&state, req.episodic);

    // Sum row counts across in-scope stores (metadata-only LanceDB call).
    let mut total: usize = 0;
    for store in &stores {
        total = total.saturating_add(store.count_filtered(&filters).await?);
    }

    // "Latest" = max activity_timestamp (updated_at ?? created_at) across
    // in-scope stores — the same key the list order and UI badge use.
    // `list(Newest, 1)` returns each store's true newest row (list sorts the
    // full filtered set before slicing), so the top row's activity timestamp
    // is the answer. Skipped when count is 0 to avoid an empty fetch.
    let latest = if total == 0 {
        None
    } else {
        let mut newest: Option<chrono::DateTime<chrono::Utc>> = None;
        for store in &stores {
            let rows = store
                .list(&filters, crate::memory::SortOrder::Newest, 1, 0)
                .await?;
            if let Some(ts) = rows.first().map(|r| r.activity_timestamp()) {
                newest = Some(newest.map_or(ts, |cur| cur.max(ts)));
            }
        }
        newest
    };

    Ok(ok(json!({
        // Key kept for the dashboard's existing contract; value is now the
        // activity timestamp (updated_at ?? created_at), so an edited row
        // counts as "latest".
        "count": total,
        "latest_created_at": latest,
    })))
}

async fn update(
    State(state): State<SharedState>,
    Json(req): Json<UpdateRequest>,
) -> Result<Response, ApiError> {
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
                    ApiError::bad_request(format!(
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
    let mut located: Option<(Arc<MemoryStore>, Memory)> = None;
    for store in stores_for_read(&state, req.episodic) {
        if let Some(fact) = store.get(&req.id).await? {
            located = Some((store, fact));
            break;
        }
    }
    let Some((current_store, existing)) = located else {
        return Err(ApiError::not_found(format!("no fact with id {}", req.id)));
    };
    if !req.scope.scope().admits(&existing) {
        return Err(ApiError::not_found(format!("no fact with id {}", req.id)));
    }
    // A content rewrite of an existing row is the same violation class
    // as an add-with-replace_ids; metadata-only patches stay unguarded.
    // Checked after the account, so the guard never quotes another
    // person's row.
    if rewrites {
        guard_user_voice(&state, std::slice::from_ref(&req.id), req.user_directed).await?;
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
            .ok_or_else(|| ApiError::not_found(format!("no fact with id {}", req.id)))?;
        return Ok(ok(fact_public(&updated)));
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
    Ok(ok(fact_public(&moved)))
}

async fn delete(
    State(state): State<SharedState>,
    Json(req): Json<DeleteRequest>,
) -> Result<Response, ApiError> {
    let scope = req.scope.scope();
    for store in stores_for_read(&state, req.episodic) {
        // Fetch first: a row outside the caller's account is simply not
        // theirs to remove, and it answers exactly as a missing one does.
        let Some(fact) = store.get(&req.id).await? else {
            continue;
        };
        if !scope.admits(&fact) {
            break;
        }
        if store.delete(&req.id).await? {
            return Ok(ok(json!({"id": req.id, "removed": true})));
        }
    }
    Ok(ok(json!({"id": req.id, "removed": false})))
}

/// `POST /api/memory/restamp` — re-own every row stamped `from` (both
/// tables, archived rows included, so provenance follows the person).
async fn restamp(
    State(state): State<SharedState>,
    Json(req): Json<RestampRequest>,
) -> Result<Response, ApiError> {
    let from = req.from.trim().to_string();
    if from.is_empty() {
        return Err(ApiError::bad_request("from must not be empty"));
    }
    let to = req
        .account_id
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty());
    let name = req.account_name.filter(|n| !n.trim().is_empty());
    let mut moved = 0usize;
    for store in stores_for_read(&state, None) {
        moved += store
            .restamp_account(&from, to.as_deref(), name.as_deref())
            .await?;
    }
    Ok(ok(json!({"from": from, "account_id": to, "moved": moved})))
}

/// `POST /api/memory/accounts` — every person the store holds rows for
/// besides the owner: `{account_id, account_name, rows}`. What a
/// per-account maintenance pass iterates.
async fn accounts(State(state): State<SharedState>) -> Result<Response, ApiError> {
    let mut seen: std::collections::BTreeMap<String, (Option<String>, usize)> =
        std::collections::BTreeMap::new();
    for store in stores_for_read(&state, None) {
        for (id, name, n) in store.distinct_accounts().await? {
            let e = seen.entry(id).or_insert((None, 0));
            if e.0.is_none() {
                e.0 = name;
            }
            e.1 += n;
        }
    }
    let list: Vec<Value> = seen
        .into_iter()
        .map(|(id, (name, rows))| json!({"account_id": id, "account_name": name, "rows": rows}))
        .collect();
    Ok(ok(json!(list)))
}

async fn forget(
    State(state): State<SharedState>,
    Json(req): Json<ForgetRequest>,
) -> Result<Response, ApiError> {
    let filters = req.filters.into_filters()?;
    // Refuse empty filters — bulk delete must be intentional. Matches the
    // CLI's refusal when no filter flags are passed.
    if filters.apps.is_empty()
        && filters.types.is_empty()
        && filters.origin.is_none()
        && filters.outcome.is_none()
        && filters.tier.is_none()
        && filters.since.is_none()
        && filters.until.is_none()
        && filters.source_session.is_none()
    {
        return Err(ApiError::bad_request(
            "forget refuses an empty filter — supply at least one of \
             apps, type, tier, from, outcome, since, until, source_session",
        ));
    }
    let store = pick_store(&state, req.episodic);
    let removed = store.forget(&filters).await?;
    Ok(ok(json!({"removed": removed})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Origin;

    #[test]
    fn global_wins_over_a_stamped_scope() {
        assert_eq!(written_scope(Some("/u/w/p".into()), true), None);
        assert_eq!(
            written_scope(Some("/u/w/p".into()), false).as_deref(),
            Some("/u/w/p")
        );
        // An explicit empty cwd is no project, not a project named "".
        assert_eq!(written_scope(Some("".into()), false), None);
    }

    fn row(tier: Tier, cwd: Option<&str>) -> Memory {
        let mut m = Memory::new("x", MemoryType::Fact, Origin::Derived);
        m.tier = tier;
        m.scope = cwd.map(str::to_string);
        m
    }

    #[test]
    fn an_omitted_tier_is_episodic_unless_something_says_otherwise() {
        assert_eq!(resolve_tier(None, false, &[], false), Tier::Episodic);
        assert_eq!(
            resolve_tier(Some(Tier::Core), false, &[], false),
            Tier::Core
        );
        assert_eq!(
            resolve_tier(Some(Tier::Core), true, &[], false),
            Tier::Episodic
        );
        // A replacement keeps its losers' highest tier.
        let losers = [row(Tier::Semantic, None), row(Tier::Core, None)];
        assert_eq!(resolve_tier(None, false, &losers, false), Tier::Core);
        // An indexed row is long-term.
        assert_eq!(resolve_tier(None, false, &[], true), Tier::Semantic);
    }

    fn add_req(v: Value) -> AddRequest {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn scope_resolution_on_add() {
        let home = crate::memory::scope::home();
        // A real dir under home: this crate's checkout is one.
        let here = std::env::current_dir().unwrap();
        if !here.starts_with(&home) {
            return; // CI outside home: the rules are covered in memory::scope
        }
        let root = here.to_string_lossy().to_string();
        let src = here.join("src").to_string_lossy().to_string();
        // The model's scope wins when it is a dir inside root.
        let r = add_req(json!({"content": "x", "cwd": root, "root": root, "scope": src}));
        assert_eq!(
            resolve_scope(&r, &[], Tier::Semantic).as_deref(),
            Some(src.as_str())
        );
        // A scope outside root falls back to the session cwd.
        let r = add_req(json!({"content": "x", "cwd": root, "root": root, "scope": "/etc"}));
        assert_eq!(
            resolve_scope(&r, &[], Tier::Semantic).as_deref(),
            Some(root.as_str())
        );
        // A parent of root is fine.
        let parent = here.parent().unwrap().to_string_lossy().to_string();
        let r = add_req(json!({"content": "x", "cwd": root, "root": root, "scope": parent}));
        if here.parent().unwrap() != home {
            assert_eq!(
                resolve_scope(&r, &[], Tier::Semantic).as_deref(),
                Some(parent.as_str())
            );
        }
        // Home itself is never a scope.
        let h = home.to_string_lossy().to_string();
        let r = add_req(json!({"content": "x", "cwd": h}));
        assert_eq!(resolve_scope(&r, &[], Tier::Episodic), None);
        // Core and global rows have none.
        let r = add_req(json!({"content": "x", "cwd": root}));
        assert_eq!(resolve_scope(&r, &[], Tier::Core), None);
        let r = add_req(json!({"content": "x", "cwd": root, "global": true}));
        assert_eq!(resolve_scope(&r, &[], Tier::Semantic), None);
        // A merge takes its losers' common scope, not the session's.
        let r = add_req(json!({"content": "x", "cwd": root}));
        let a = home.join("w/a/x").to_string_lossy().to_string();
        let b = home.join("w/a/y").to_string_lossy().to_string();
        let losers = [row(Tier::Semantic, Some(&a)), row(Tier::Semantic, Some(&b))];
        assert_eq!(
            resolve_scope(&r, &losers, Tier::Semantic),
            Some(home.join("w/a").to_string_lossy().to_string())
        );
        let losers = [row(Tier::Semantic, Some(&a)), row(Tier::Semantic, None)];
        assert_eq!(resolve_scope(&r, &losers, Tier::Semantic), None);
    }

    /// The 2026-10-05 session: started in a non-git workspace holding two
    /// repos, shell cd'd into one of them. With the session's root stamped,
    /// a candidate as the line shows it, an absolute dir under root, no scope
    /// and a dir outside root each land where the spec says.
    #[test]
    fn scope_from_a_nested_repo_resolves_against_the_session_root() {
        use std::path::{Path, PathBuf};
        let home = crate::memory::scope::home();
        let ws = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("scope-nested-{}", std::process::id()))
            .join("linggen");
        if !ws.starts_with(&home) {
            return; // CI outside home: the rules are covered in memory::scope
        }
        let engine = ws.join("linggen");
        let cfo = ws.join("skills").join("cfo");
        std::fs::create_dir_all(engine.join(".git")).unwrap();
        std::fs::create_dir_all(ws.join("skills").join(".git")).unwrap();
        std::fs::create_dir_all(&cfo).unwrap();
        let s = |p: &Path| p.to_string_lossy().to_string();
        let at = |cwd: &Path, scope: Option<String>| {
            let mut v = json!({"content": "x", "cwd": s(cwd), "root": s(&ws)});
            if let Some(sc) = scope {
                v["scope"] = json!(sc);
            }
            resolve_scope(&add_req(v), &[], Tier::Episodic)
        };
        // Relative candidate (shown relative to root's parent), from skills/cfo.
        assert_eq!(at(&cfo, Some("linggen/linggen".into())), Some(s(&engine)));
        // Absolute dir under root, from the other repo.
        assert_eq!(at(&engine, Some(s(&cfo))), Some(s(&cfo)));
        // No scope: the session cwd.
        assert_eq!(at(&cfo, None), Some(s(&cfo)));
        // Outside root: falls back to the session cwd.
        assert_eq!(at(&engine, Some("/etc".into())), Some(s(&engine)));
        // The pre-fix stamp (root = the shell cwd's git root) dropped the
        // candidate: the hook, not the daemon, must send the session's root.
        let v = json!({"content": "x", "cwd": s(&cfo), "root": s(&ws.join("skills")), "scope": "linggen/linggen"});
        assert_eq!(
            resolve_scope(&add_req(v), &[], Tier::Episodic),
            Some(s(&cfo))
        );
        let _ = std::fs::remove_dir_all(ws.parent().unwrap());
    }

    /// An unstamped add (Codex) that names an existing directory keeps it;
    /// one naming nothing real stays unscoped.
    #[test]
    fn an_unstamped_add_keeps_a_named_directory() {
        let dir = std::env::current_dir().unwrap();
        let named = dir.to_string_lossy().to_string();
        let r = add_req(json!({"content": "x", "scope": named}));
        assert_eq!(resolve_scope(&r, &[], Tier::Semantic), Some(named));
        let r = add_req(json!({"content": "x", "scope": "/no/such/dir/anywhere"}));
        assert_eq!(resolve_scope(&r, &[], Tier::Semantic), None);
        let r = add_req(json!({"content": "x", "scope": "relative/dir"}));
        assert_eq!(resolve_scope(&r, &[], Tier::Semantic), None);
    }

    #[test]
    fn hooks_are_one_short_line() {
        assert_eq!(
            clean_summary(Some("  a\n b  ".into())).as_deref(),
            Some("a b")
        );
        assert_eq!(clean_summary(Some("   ".into())), None);
        let long = clean_summary(Some("x".repeat(200))).unwrap();
        assert_eq!(long.chars().count(), 80);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn an_old_phone_naming_contexts_any_gets_its_apps() {
        let req: ListRequest =
            serde_json::from_value(json!({"contexts_any": ["dj", "cfo"], "contexts": ["x"]}))
                .unwrap();
        let f = req.filters.into_filters().unwrap();
        assert_eq!(f.apps, ["dj", "cfo"]);
    }

    #[test]
    fn global_on_update_clears_the_scope() {
        assert_eq!(scope_patch(None, false, true), Some(None));
        assert_eq!(scope_patch(Some("/x".into()), false, true), Some(None));
        assert_eq!(
            scope_patch(Some("/x".into()), true, false),
            Some(Some("/x".into()))
        );
        assert_eq!(scope_patch(None, true, false), Some(None));
        assert_eq!(scope_patch(None, false, false), None);
    }

    #[test]
    fn list_takes_types_and_exclude_types() {
        let req: ListRequest = serde_json::from_value(json!({
            "types": ["preference", "decision"],
            "type": "fact",
            "exclude_types": ["built"],
            "cwd_scope": "/u/w/p"
        }))
        .unwrap();
        let f = req.filters.into_filters().unwrap();
        assert_eq!(
            f.types,
            [
                MemoryType::Preference,
                MemoryType::Decision,
                MemoryType::Fact
            ]
        );
        assert_eq!(f.exclude_types, [MemoryType::Built]);
        // `cwd_scope` is the pre-v2 name, still accepted.
        assert_eq!(f.scope_root.as_deref(), Some("/u/w/p"));
        let req: ListRequest = serde_json::from_value(json!({"scope_root": "/u/w/q"})).unwrap();
        let f = req.filters.into_filters().unwrap();
        assert_eq!(f.scope_root.as_deref(), Some("/u/w/q"));
    }

    #[test]
    fn search_takes_exclude_types() {
        let req: SearchRequest = serde_json::from_value(json!({
            "query": "q",
            "exclude_types": ["preference"]
        }))
        .unwrap();
        let f = req.filters.into_filters().unwrap();
        assert_eq!(f.exclude_types, [MemoryType::Preference]);
        let sql = f.to_sql_for_test();
        assert!(sql.contains("type NOT IN ('preference')"), "{sql}");
    }

    /// A state over fresh stores, with an embedder that is never loaded.
    async fn scratch_state(dir: &std::path::Path) -> SharedState {
        let semantic = Arc::new(MemoryStore::open_semantic(dir).await.unwrap());
        let episodic = Arc::new(MemoryStore::open_episodic(dir).await.unwrap());
        let recall = Arc::new(crate::memory::Recall::new(
            Arc::clone(&semantic),
            Arc::clone(&episodic),
        ));
        fn no_model() -> anyhow::Result<crate::embed::Embedder> {
            anyhow::bail!("no model in unit tests")
        }
        Arc::new(crate::handle::Memory::new(
            semantic,
            episodic,
            recall,
            Arc::new(crate::embed::LoadOnce::new(no_model)),
            dir.to_path_buf(),
            std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        ))
    }

    /// The 2026-10-05 loss: `memory_add` merged into the row it was told to
    /// replace, then retired that same row — nothing live was left. The row
    /// a write lands in is never one of its losers; the others still retire.
    #[tokio::test]
    async fn a_replacement_never_retires_the_row_it_wrote_into() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = scratch_state(dir.path()).await;
        let kept = Memory::new("kept", MemoryType::Fact, Origin::Derived);
        let other = Memory::new("other", MemoryType::Fact, Origin::Derived);
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

    /// Reading rows never waits on the embedding model: with a model that
    /// cannot load, `session_start` still answers with core.
    #[tokio::test]
    async fn session_start_needs_no_embedder() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = scratch_state(dir.path()).await;
        let mut core = Memory::new("Alex lives in Lisbon", MemoryType::Fact, Origin::User);
        core.tier = Tier::Core;
        state.store.insert(&[core]).await.unwrap();

        let app = super::super::session::router().with_state(Arc::clone(&state));
        let res = tower::ServiceExt::oneshot(
            app,
            axum::http::Request::post("/api/memory/session_start")
                .header("content-type", "application/json")
                .body(axum::body::Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(res.status(), 200);
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["data"]["core"].as_array().map(Vec::len), Some(1), "{v}");
        assert!(!state.embedder.is_ready(), "nothing asked for the model");
        // A call that embeds reports the load failure instead of hanging.
        assert!(state.embedder().await.is_err());
    }

    // ── one person's write never reaches another person's rows ──────────

    fn row_of(content: &str, account: Option<&str>, tier: Tier) -> Memory {
        let mut m = Memory::new(content, MemoryType::Fact, Origin::User);
        m.account_id = account.map(str::to_string);
        m.tier = tier;
        m
    }

    async fn post(state: &SharedState, path: &str, body: Value) -> (u16, Value) {
        let app = router().with_state(Arc::clone(state));
        let res = tower::ServiceExt::oneshot(
            app,
            axum::http::Request::post(path)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
        let status = res.status().as_u16();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// Alex's row, in the user's voice. Another person naming it in
    /// `replace_ids` — or rewriting it — is refused before anything is
    /// written (the test state has no model, so nothing could be embedded
    /// either), with the same answer as an id that does not exist, and the
    /// user-voice guard never gets to quote it.
    #[tokio::test]
    async fn another_persons_rows_cannot_be_replaced_or_rewritten() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = scratch_state(dir.path()).await;
        let alex = row_of("Alex prefers tea", Some("u-alex"), Tier::Semantic);
        state.store.insert(&[alex.clone()]).await.unwrap();

        let replace = |id: &str| json!({"content": "prefers coffee", "account_id": "u-other", "replace_ids": [id]});
        let (theirs, body) = post(&state, "/api/memory/add", replace(&alex.id)).await;
        let (missing, missing_body) = post(&state, "/api/memory/add", replace("nope")).await;
        assert_eq!(theirs, 404, "{body}");
        assert_eq!(missing, 404, "{missing_body}");
        assert_eq!(body["code"], missing_body["code"]);
        assert!(!body.to_string().contains("tea"), "{body}");

        let (status, body) = post(
            &state,
            "/api/memory/update",
            json!({"id": alex.id, "content": "prefers coffee", "account": "u-other"}),
        )
        .await;
        assert_eq!(status, 404, "{body}");
        assert!(!body.to_string().contains("tea"), "{body}");

        let kept = state.store.get(&alex.id).await.unwrap().unwrap();
        assert!(kept.expired_at.is_none());
        assert_eq!(kept.content, "Alex prefers tea");
        assert_eq!(state.store.count().await.unwrap(), 1);
        assert_eq!(state.episodic.count().await.unwrap(), 0);
    }

    /// A person naming their own row gets past the check (and on to the
    /// embedder, which this test state does not have).
    #[tokio::test]
    async fn a_person_may_replace_their_own_row() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = scratch_state(dir.path()).await;
        let alex = Memory::new("Alex prefers tea", MemoryType::Fact, Origin::Derived);
        let alex = Memory {
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
        let rows: Vec<Memory> = req.facts.into_iter().filter_map(batch_row).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].account_id.as_deref(), Some("u-other"));
        assert_eq!(rows[0].account_name.as_deref(), Some("Sam"));
        assert_eq!(rows[1].account_id, None);
    }
}
