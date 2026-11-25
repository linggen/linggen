//! The read verbs: `get` one row, `search` by meaning (the configured
//! recall floor when the call names none), `list` a page across both
//! tables, `count`; and the filter block they share, `day` folded into
//! `since`/`until`.

use crate::embed::Patience;
use crate::error::MemError;
use crate::handle::Memory;
use crate::memory::{Filters, Hit};
use crate::ops::memory::dto::{
    fact_public, facts_public, scored_facts_public, CountRequest, FilterDTO, GetRequest,
    ListRequest, SearchRequest, SearchTable,
};
use crate::ops::memory::stores_for_read;
use serde_json::{json, Value};

impl FilterDTO {
    /// Fold the `day` sugar into `since`/`until`. Called by every
    /// handler before `into_filters` — a bad day string is a 400, not a
    /// silently-ignored filter.
    fn resolve_day(&mut self) -> Result<(), MemError> {
        let Some(day) = self.day.take() else {
            return Ok(());
        };
        let date = crate::ops::days::parse_day(&day)?;
        let (start, end) = crate::ops::days::local_day_bounds(date);
        if self.since.is_none() {
            self.since = Some(start);
        }
        if self.until.is_none() {
            self.until = Some(end);
        }
        Ok(())
    }

    pub(crate) fn into_filters(mut self) -> Result<Filters, MemError> {
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
            unsilenced_only: false,
        })
    }
}

pub(in crate::ops) async fn get(state: &Memory, req: GetRequest) -> Result<Value, MemError> {
    let scope = req.scope.scope();
    for store in stores_for_read(state, req.episodic) {
        if let Some(fact) = store.get(&req.id).await? {
            if !scope.admits(&fact) {
                break;
            }
            return Ok(fact_public(&fact));
        }
    }
    Err(MemError::not_found(format!("no fact with id {}", req.id)))
}

pub(in crate::ops) async fn search(state: &Memory, req: SearchRequest) -> Result<Value, MemError> {
    if req.query.trim().is_empty() {
        return Err(MemError::bad_request("query must not be empty"));
    }

    let patience = match req.skip_if_warming {
        true => Patience::AnswerNow,
        false => Patience::WaitForWarmup,
    };
    let vector = state
        .embedder_with(patience)
        .await?
        .embed_query_serialized(req.query.clone())
        .await
        .map_err(MemError::internal)?;
    let mut filters = req.filters.into_filters()?;
    filters.unsilenced_only = req.push;

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
        state, &table, &vector, &req.query, &filters, req.limit, min_score,
    )
    .await?;
    Ok(Value::Array(scored_facts_public(&results)))
}

async fn scored(
    state: &Memory,
    table: &SearchTable,
    vector: &[f32],
    query: &str,
    filters: &Filters,
    limit: usize,
    min_score: Option<f32>,
) -> Result<Vec<Hit>, MemError> {
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

pub(in crate::ops) async fn list(state: &Memory, mut req: ListRequest) -> Result<Value, MemError> {
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
    let created_since = unjudged_since(state, &req).await?;
    let mut filters = req.filters.into_filters()?;
    filters.created_since = created_since;
    let sort = req.sort.into();
    let mut combined = Vec::new();
    // For each in-scope store, pull `limit + offset` rows so the post-
    // merge sort+page still has enough material; bounded by the per-
    // store list cap.
    let take = req.limit.saturating_add(req.offset);
    for store in stores_for_read(state, req.episodic) {
        let rows = store.list(&filters, sort, take, 0).await?;
        combined.extend(rows);
    }
    sort_combined(&mut combined, sort);
    let paged: Vec<_> = combined
        .into_iter()
        .skip(req.offset)
        .take(req.limit)
        .collect();
    Ok(Value::Array(facts_public(&paged)))
}

/// `unjudged` resolved to its cutoff: the listed day's `remembered_at`.
/// The flag names a day's worklist, so it needs `day`.
async fn unjudged_since(
    state: &Memory,
    req: &ListRequest,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, MemError> {
    if !req.unjudged {
        return Ok(None);
    }
    let Some(day) = req.filters.day.as_deref() else {
        return Err(MemError::bad_request("unjudged needs a day"));
    };
    let key = crate::ops::days::day_key(day)?;
    let days = crate::ops::days::load(&state.data_dir).await;
    Ok(days.days.get(&key).and_then(|r| r.remembered_at))
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

pub(in crate::ops) async fn count(state: &Memory, req: CountRequest) -> Result<Value, MemError> {
    let filters = req.filters.into_filters()?;
    let stores = stores_for_read(state, req.episodic);

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

    Ok(json!({
        // Key kept for the dashboard's existing contract; value is now the
        // activity timestamp (updated_at ?? created_at), so an edited row
        // counts as "latest".
        "count": total,
        "latest_created_at": latest,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryType;

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
}
