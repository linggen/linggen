//! The verbs that remove or re-own rows: `delete` one by id, `forget` by
//! filter (never an empty one), `restamp` a device's rows to an account,
//! and `accounts`, every person the store holds rows for.

use crate::error::MemError;
use crate::handle::Memory;
use crate::ops::memory::dto::{DeleteRequest, ForgetRequest, RestampRequest};
use crate::ops::memory::{pick_store, stores_for_read};
use serde_json::{json, Value};

pub async fn delete(state: &Memory, req: DeleteRequest) -> Result<Value, MemError> {
    let scope = req.scope.scope();
    for store in stores_for_read(state, req.episodic) {
        // Fetch first: a row outside the caller's account is simply not
        // theirs to remove, and it answers exactly as a missing one does.
        let Some(fact) = store.get(&req.id).await? else {
            continue;
        };
        if !scope.admits(&fact) {
            break;
        }
        if store.delete(&req.id).await? {
            return Ok(json!({"id": req.id, "removed": true}));
        }
    }
    Ok(json!({"id": req.id, "removed": false}))
}

/// `POST /api/memory/restamp` — re-own every row stamped `from` (both
/// tables, archived rows included, so provenance follows the person).
pub async fn restamp(state: &Memory, req: RestampRequest) -> Result<Value, MemError> {
    let from = req.from.trim().to_string();
    if from.is_empty() {
        return Err(MemError::bad_request("from must not be empty"));
    }
    let to = req
        .account_id
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty());
    let name = req.account_name.filter(|n| !n.trim().is_empty());
    let mut moved = 0usize;
    for store in stores_for_read(state, None) {
        moved += store
            .restamp_account(&from, to.as_deref(), name.as_deref())
            .await?;
    }
    Ok(json!({"from": from, "account_id": to, "moved": moved}))
}

/// `POST /api/memory/accounts` — every person the store holds rows for
/// besides the owner: `{account_id, account_name, rows}`. What a
/// per-account maintenance pass iterates.
pub async fn accounts(state: &Memory) -> Result<Value, MemError> {
    let mut seen: std::collections::BTreeMap<String, (Option<String>, usize)> =
        std::collections::BTreeMap::new();
    for store in stores_for_read(state, None) {
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
    Ok(json!(list))
}

pub async fn forget(state: &Memory, req: ForgetRequest) -> Result<Value, MemError> {
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
        return Err(MemError::bad_request(
            "forget refuses an empty filter — supply at least one of \
             apps, type, tier, from, outcome, since, until, source_session",
        ));
    }
    let store = pick_store(state, req.episodic);
    let removed = store.forget(&filters).await?;
    Ok(json!({"removed": removed}))
}
