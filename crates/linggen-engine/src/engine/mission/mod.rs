//! Mission runtime records + lookup contracts.
//!
//! - `record` — `Mission`, `MissionRunEntry`, `MissionPermission`,
//!   `MISSION_AGENT_ID`. Pure data types the engine reads.
//! - `registry` — `MissionRegistry` trait. Spec lookup contract.
//!
//! Run history (`runs.jsonl`) has no engine consumer — the scheduler
//! that records it lives beside the loader in `extensions::missions`.
//!
//! Mirrors `engine::agent` (`record` + `registry` + concrete `RunStore`)
//! and `engine::skill` (`record` + `registry`). Disk loading lives in
//! `extensions::missions`; that module's `MissionLoader` is the
//! production impl of `MissionRegistry`.

pub mod record;
pub mod registry;

pub use registry::MissionRegistry;
