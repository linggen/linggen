//! `MissionDraft` — input shape for create/update on `MissionLoader`.
//!
//! Builder used by CRUD to avoid unreadable positional args. All
//! fields optional; `update_mission` applies only what's `Some`,
//! which lets the HTTP layer pass partial patches without
//! reconstructing every field from the prior mission.

use crate::engine::mission::record::MissionPermission;

#[derive(Debug, Default, Clone)]
pub struct MissionDraft {
    pub name: Option<String>,
    pub description: Option<String>,
    pub schedule: Option<String>,
    pub enabled: Option<bool>,
    pub catchup_hours: Option<Option<u64>>,
    pub cwd: Option<Option<String>>,
    pub model: Option<Option<String>>,
    pub agent: Option<String>,
    pub kickoff: Option<Vec<String>>,
    pub allowed_tools: Option<Vec<String>>,
    pub permission: Option<Option<MissionPermission>>,
    pub prompt: Option<String>,
    pub project: Option<Option<String>>,
}

impl MissionDraft {
    /// True when the draft changes anything beyond on/off and the schedule —
    /// the part of a skill mission that belongs to its skill.
    pub fn touches_definition(&self) -> bool {
        self.name.is_some()
            || self.description.is_some()
            || self.catchup_hours.is_some()
            || self.cwd.is_some()
            || self.model.is_some()
            || self.agent.is_some()
            || self.kickoff.is_some()
            || self.allowed_tools.is_some()
            || self.permission.is_some()
            || self.prompt.is_some()
            || self.project.is_some()
    }
}
