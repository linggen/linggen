//! Engine's runtime records for missions.
//!
//! Parsed by `extensions::missions` from `~/.linggen/missions/<id>/
//! mission.md`; consumed by the engine for system-prompt injection
//! (`ActiveMission`), the scheduler's dispatch path, and the run-
//! history widgets.
//!
//! These structs live in `engine/` because every field is something
//! the engine reads or routes against. Disk-level concerns (the
//! `---` frontmatter splitter, legacy-format fallback, JSONL append
//! semantics) stay in `extensions::missions`.

use crate::engine::permission::Grants;
use serde::{Deserialize, Serialize};

/// The agent that always runs missions. Single canonical id so the
/// run-record `agent_id`, the session identity, and the lookup into
/// `agents/` all agree.
pub const MISSION_AGENT_ID: &str = "ling";

/// Permission block carried by a mission. The grammar is shared with
/// skills via `engine::permission::Grants`; this alias exists so
/// readers see "MissionPermission" in field types where the same
/// grammar is rendered with a different name in `mission.md`.
pub type MissionPermission = Grants;

/// A cron-scheduled mission stored as
/// `~/.linggen/missions/<id>/mission.md`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Mission {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,

    pub schedule: String,
    pub enabled: bool,
    /// Hours since last successful run before the post-turn seam fires a
    /// catch-up. `None` = opt out (only the cron `schedule` triggers it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catchup_hours: Option<u64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<MissionPermission>,

    /// Mission agent prompt — the body of the `.md` file.
    pub prompt: String,

    /// Engine agent that runs this mission. Comes from frontmatter
    /// `agent:`, defaults to "ling" via the parser. Used as session
    /// identity, event `agent_id`, and the lookup key into the
    /// `agents/` registry.
    #[serde(default = "default_mission_agent")]
    pub agent_id: String,

    /// Legacy project scoping. Prefer `cwd` for new missions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,

    pub created_at: u64,
}

fn default_mission_agent() -> String {
    MISSION_AGENT_ID.to_string()
}

/// A single entry in a mission's run history (`<id>/runs.jsonl`).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MissionRunEntry {
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub triggered_at: u64,
    pub status: String,
    pub skipped: bool,
    /// Set when an entry script ran; None for agent-only missions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_exit_code: Option<i32>,
    /// Per-run scratch dir (where entry output and agent temp files live).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
}
