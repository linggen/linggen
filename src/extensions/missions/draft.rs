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
    pub entry: Option<Option<String>>,
    pub allowed_tools: Option<Vec<String>>,
    pub permission: Option<Option<MissionPermission>>,
    pub prompt: Option<String>,
    pub project: Option<Option<String>>,
}
