//! Engine's contract for looking up missions.
//!
//! Lets the engine + scheduler enumerate and resolve mission specs
//! without knowing how they're stored. Missions are not project-
//! scoped (they live globally under `~/.linggen/missions/`), so the
//! surface is narrower than `AgentRegistry`: a flat `list` plus a
//! `get` by id.
//!
//! `extensions::missions::MissionStore` is the production
//! implementer; tests can stub against a smaller in-memory impl.
//! Returns owned `Mission` records — cloning is cheap and avoids
//! threading a borrow through async tasks.

use crate::engine::mission::record::Mission;
use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait MissionRegistry: Send + Sync {
    /// All missions known to the store, newest-first. Errors only
    /// when the backing store can't be read (corrupt I/O, missing
    /// permissions); a healthy store with no missions returns an
    /// empty `Vec`.
    async fn list(&self) -> Result<Vec<Mission>>;

    /// Look up a mission by id. `None` if no mission with that id is
    /// installed at the time of the call.
    async fn get(&self, mission_id: &str) -> Result<Option<Mission>>;
}
