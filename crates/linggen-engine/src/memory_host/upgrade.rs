//! The move from the v1 store to the notes, at the engine's start: once, when
//! the person's memory is v2 (the default since 3.0), a v1 store has rows and
//! no note exists yet. What moves, and the one-time record, is
//! `own_memory::migrate`; this is only where the rows come from — the store
//! this process hosts — and the log line.

use crate::config::MemoryConfig;
use crate::own_memory::migrate::{upgrade_once, Upgrade, V1Row};
use crate::own_memory::Store;
use serde_json::json;
use std::time::Duration;

/// Longer than a verb's usual wait: every row of the store comes back.
const LIST_WAIT: Duration = Duration::from_secs(60);

/// Look at the move now, after the store is open. Never fails the start: what
/// went wrong is logged and the next start looks again.
pub async fn upgrade_to_v2(memory: &MemoryConfig) {
    let wants_v2 = memory.wants_v2();
    let Some(store) = Store::this_machine() else {
        return;
    };
    // `upgrade_once` asks for the rows only when the move is going to run;
    // it waits for them here, off the engine's own workers.
    let engine = tokio::runtime::Handle::current();
    let outcome = tokio::task::spawn_blocking(move || upgrade_once(&store, wants_v2, || engine.block_on(v1_rows()))).await;
    match outcome {
        Ok(Ok(Upgrade::Moved(report))) => {
            tracing::info!("memory: moved the v1 rows into notes — {}", report.lines(false).join("; "))
        }
        Ok(Ok(other)) => tracing::debug!("memory: v1 move not needed ({other:?})"),
        Ok(Err(why)) => tracing::warn!("memory: the v1 move did not run: {why}"),
        Err(e) => tracing::warn!("memory: the v1 move did not run: {e}"),
    }
}

/// Every row of the owner's v1 store (the `list` verb as the owner, no limit).
async fn v1_rows() -> Result<Vec<V1Row>, String> {
    let (status, envelope) = super::post_verb(ling_mem::Who::Owner, "list", json!({ "limit": usize::MAX / 4 }), LIST_WAIT)
        .await
        .map_err(|e| e.to_string())?;
    if !(200..300).contains(&status) {
        return Err(envelope["error"].as_str().unwrap_or("memory did not answer").to_string());
    }
    let (rows, unreadable) = V1Row::from_list(&envelope["data"]);
    if unreadable > 0 {
        tracing::warn!("memory: {unreadable} v1 entries could not be read for the move");
    }
    Ok(rows)
}
