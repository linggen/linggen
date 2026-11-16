use crate::error::MemError;
use crate::handle::Memory;
use crate::memory::maintenance::PRUNE_OLDER_THAN_DAYS;
use serde_json::{json, Value};

pub async fn compact(state: &Memory) -> Result<Value, MemError> {
    let started = std::time::Instant::now();
    let prune = chrono::Duration::days(PRUNE_OLDER_THAN_DAYS);
    for store in [&state.store, &state.episodic] {
        store
            .optimize_storage(prune)
            .await
            .map_err(MemError::internal)?;
    }
    Ok(json!({
        "compacted": [state.store.table_name(), state.episodic.table_name()],
        "ms": started.elapsed().as_millis() as u64,
    }))
}
