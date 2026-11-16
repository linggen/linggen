//! Primary or replica: the facts behind `GET /api/sync/role` and the verb
//! behind `PUT /api/sync/primary`. The access check for `set_primary` is the
//! handler's (`local::require_local`), never this file's.

use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::error::MemError;
use crate::handle::Memory;
use crate::replica::{link, outbox, state::normalize_primary};

fn view(state: &Memory) -> Value {
    state.replica.snapshot().view(outbox::len(&state.data_dir))
}

/// Role, upstream, approved, online, last sync, queued writes and why a sync
/// did not happen. Never the token.
pub(in crate::ops) async fn role(state: &Memory) -> Result<Value, MemError> {
    Ok(view(state))
}

#[derive(Deserialize)]
pub struct SetPrimary {
    primary: String,
}

/// Become the primary (`local` or empty), or ask the machine at
/// `req.primary` to let this one follow it.
pub async fn set_primary(state: &Arc<Memory>, req: SetPrimary) -> Result<Value, MemError> {
    let wanted = req.primary.trim();
    if wanted.is_empty() || wanted.eq_ignore_ascii_case("local") {
        link::become_primary(state);
        return Ok(view(state));
    }
    let primary = normalize_primary(wanted).ok_or_else(|| {
        MemError::bad_request("an address like 192.168.1.5:9527 (the primary's Linggen), or local")
    })?;
    link::ask(state, primary)
        .await
        .map_err(|e| MemError::bad_request(format!("{e:#}")))?;
    Ok(view(state))
}
