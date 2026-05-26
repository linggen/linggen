//! Mission runtime records + lookup contracts.
//!
//! - `record` — `Mission`, `MissionRunEntry`, `MissionPermission`,
//!   `MISSION_AGENT_ID`. Pure data types the engine reads.
//! - `registry` — `MissionRegistry` trait. Spec lookup contract.
//! - `runs` — `MissionRunStore` trait. Run-history persistence
//!   contract.
//!
//! Mirrors `engine::agent` (`record` + `registry` + concrete `RunStore`)
//! and `engine::skill` (`record` + `registry`). Disk loading lives in
//! `extensions::missions`; that module's `MissionLoader` is the
//! production impl for both traits.

pub mod record;
pub mod registry;
pub mod runs;

pub use record::{Mission, MissionPermission, MissionRunEntry, MISSION_AGENT_ID};
pub use registry::MissionRegistry;
pub use runs::MissionRunStore;
