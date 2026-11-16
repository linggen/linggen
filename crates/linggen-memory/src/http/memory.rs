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
use crate::ops;
pub(crate) use crate::ops::memory::dto::AddRequest;
use crate::ops::memory::dto::{
    AddBatchRequest, CountRequest, DeleteRequest, ForgetRequest, GetRequest, ListRequest,
    RestampRequest, SearchRequest, UpdateRequest,
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

async fn get(
    State(state): State<SharedState>,
    Json(req): Json<GetRequest>,
) -> Result<Response, ApiError> {
    Ok(ok(ops::memory::read::get(&state, req).await?))
}

async fn search(
    State(state): State<SharedState>,
    Json(req): Json<SearchRequest>,
) -> Result<Response, ApiError> {
    Ok(ok(ops::memory::read::search(&state, req).await?))
}

async fn list(
    State(state): State<SharedState>,
    Json(req): Json<ListRequest>,
) -> Result<Response, ApiError> {
    Ok(ok(ops::memory::read::list(&state, req).await?))
}

async fn count(
    State(state): State<SharedState>,
    Json(req): Json<CountRequest>,
) -> Result<Response, ApiError> {
    Ok(ok(ops::memory::read::count(&state, req).await?))
}

async fn update(
    State(state): State<SharedState>,
    Json(req): Json<UpdateRequest>,
) -> Result<Response, ApiError> {
    Ok(ok(ops::memory::update::update(&state, req).await?))
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
