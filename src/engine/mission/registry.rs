//! Engine's contract for looking up missions.
//!
//! Lets the engine and a chat turn bound to a mission resolve mission
//! specs without knowing how they're stored. Missions are not project-
//! scoped (they live globally under `~/.linggen/missions/` or inside a
//! skill), so the surface is a lookup by id, a fresh re-read,
//! and the folder a mission's files live in. `AgentManager` holds an
//! `Arc<dyn MissionRegistry>`; editing and scheduling stay on the loader.
//!
//! `extensions::missions::MissionLoader` is the production
//! implementer; tests can stub against a smaller in-memory impl.
//! Returns owned `Mission` records — cloning is cheap and avoids
//! threading a borrow through async tasks.

use crate::engine::mission::record::Mission;
use anyhow::Result;
use std::path::PathBuf;

pub trait MissionRegistry: Send + Sync {
    /// Look up a mission by id from the loaded set.
    fn get_mission(&self, mission_id: &str) -> Result<Option<Mission>>;

    /// Re-read one mission from disk and return it; `None` if it's gone.
    fn reload_one(&self, mission_id: &str) -> Option<Mission>;

    /// The folder a mission's own files live in (`$MISSION_DIR`).
    fn mission_dir(&self, mission_id: &str) -> PathBuf;
}

/// An `Arc` of a registry is a registry — so `&manager.missions` passes
/// wherever a `&dyn MissionRegistry` is taken.
impl<T: MissionRegistry + ?Sized> MissionRegistry for std::sync::Arc<T> {
    fn get_mission(&self, mission_id: &str) -> Result<Option<Mission>> {
        (**self).get_mission(mission_id)
    }
    fn reload_one(&self, mission_id: &str) -> Option<Mission> {
        (**self).reload_one(mission_id)
    }
    fn mission_dir(&self, mission_id: &str) -> PathBuf {
        (**self).mission_dir(mission_id)
    }
}
