//! The row verbs behind `/api/memory/<verb>` (add, get, search, list,
//! update, delete, …), without HTTP. `http/memory.rs` only adapts them.

use crate::handle::Memory;
use crate::memory::MemoryStore;
use std::sync::Arc;

pub mod dto;
mod place;
pub mod read;
pub mod update;
pub mod write;

/// Pick the semantic or episodic store based on the request's `episodic`
/// flag. Centralized so every CRUD endpoint routes consistently.
pub(crate) fn pick_store(state: &Memory, episodic: bool) -> Arc<MemoryStore> {
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
pub(crate) fn stores_for_read(state: &Memory, episodic: Option<bool>) -> Vec<Arc<MemoryStore>> {
    match episodic {
        Some(true) => vec![Arc::clone(&state.episodic)],
        Some(false) => vec![Arc::clone(&state.store)],
        None => vec![Arc::clone(&state.store), Arc::clone(&state.episodic)],
    }
}
