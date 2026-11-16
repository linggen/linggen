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
use crate::memory::{Filters, Hit};
use crate::ops;
pub(crate) use crate::ops::memory::dto::AddRequest;
use crate::ops::memory::dto::{
    fact_public, facts_public, scored_facts_public, AddBatchRequest, CountRequest,
    DeleteRequest, FilterDTO, ForgetRequest, GetRequest, ListRequest, RestampRequest,
    SearchRequest, SearchTable, UpdateRequest,
};
use crate::ops::memory::{pick_store, stores_for_read};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

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

async fn add(
    State(state): State<SharedState>,
    Json(req): Json<AddRequest>,
) -> Result<Response, ApiError> {
    Ok(ok(ops::memory::write::add(&state, req).await?))
}

async fn add_batch(
    State(state): State<SharedState>,
    Json(req): Json<AddBatchRequest>,
) -> Result<Response, ApiError> {
    Ok(ok(ops::memory::write::add_batch(&state, req).await?))
}

async fn update(
    State(state): State<SharedState>,
    Json(req): Json<UpdateRequest>,
) -> Result<Response, ApiError> {
    Ok(ok(ops::memory::update::update(&state, req).await?))
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
    use crate::memory::{Memory, MemoryStore, MemoryType, Origin, Tier};
    use std::sync::Arc;

    #[test]
    fn an_old_phone_naming_contexts_any_gets_its_apps() {
        let req: ListRequest =
            serde_json::from_value(json!({"contexts_any": ["dj", "cfo"], "contexts": ["x"]}))
                .unwrap();
        let f = req.filters.into_filters().unwrap();
        assert_eq!(f.apps, ["dj", "cfo"]);
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

}
