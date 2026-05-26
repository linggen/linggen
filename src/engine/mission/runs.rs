//! Engine's contract for mission run records.
//!
//! Mission runs are persisted to `<id>/runs.jsonl` (one line per
//! invocation, append-only). The scheduler, the catch-up logic on
//! the turn seam, and the run-history widgets all read against this
//! contract.
//!
//! Distinct from `engine::agent::RunStore`:
//!   - Agent runs are in-memory, process-lifetime only.
//!   - Mission runs are disk-persistent and survive restarts —
//!     that's how the scheduler decides whether a catch-up is due.
//!
//! `extensions::missions::MissionLoader` is the production
//! implementer. The methods are synchronous because the underlying
//! I/O is local-disk and small (the JSONL is bounded per mission);
//! async would just add a layer with no real concurrency win.

use crate::engine::mission::record::MissionRunEntry;
use anyhow::Result;

pub trait MissionRunStore: Send + Sync {
    /// Append a run entry to the mission's `runs.jsonl`, creating
    /// the mission directory if it doesn't exist yet.
    fn append(&self, mission_id: &str, entry: &MissionRunEntry) -> Result<()>;

    /// All run entries for a mission, newest-first. Empty `Vec` when
    /// the file doesn't exist or has no entries.
    fn list(&self, mission_id: &str) -> Result<Vec<MissionRunEntry>>;

    /// Newest-first run entries with optional pagination. `limit` /
    /// `offset` are applied after the reverse — `offset=0` returns
    /// the most recent runs first.
    fn list_paginated(
        &self,
        mission_id: &str,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<MissionRunEntry>>;

    /// Remove the run entry whose `session_id` matches, rewriting
    /// `runs.jsonl` in place. Used when a mission session is deleted
    /// from the UI and the run history should follow.
    fn remove_by_session(&self, mission_id: &str, session_id: &str) -> Result<()>;

    /// `triggered_at` of the most recent run with `status="completed"`
    /// and `skipped=false`. Used by the scheduler to compute the
    /// catch-up window and to set `MISSION_LAST_RUN_AT` for the
    /// entry script. `None` when there's no eligible run.
    fn last_successful_run_at(&self, mission_id: &str) -> Option<u64>;
}
