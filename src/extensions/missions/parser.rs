//! Mission `.md` parser + serializer.
//!
//! Reads two on-disk shapes:
//! - **New format** — frontmatter mirrors SKILL.md (`description`,
//!   `allowed-tools`, `permission`) plus mission-specific
//!   scheduling fields (`schedule`, `enabled`, `catchup_hours`,
//!   `cwd`, `agent`, `kickoff`).
//! - **Legacy format** — pre-redesign shape with `mode:` and
//!   `permission_tier:`. Detected by sentinel fields, mapped on
//!   read, migrated on next write. `mode: app` is rejected (no
//!   longer supported).
//!
//! Writes only the new format. `mission_to_md` reconstructs the
//! markdown from a `Mission`; round-tripping a legacy file therefore
//! migrates it next time the loader saves.

use crate::engine::mission::record::{Mission, MissionPermission, MISSION_AGENT_ID};
use crate::extensions::frontmatter::split as split_frontmatter;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// YAML frontmatter for a `mission.md` file in the new (skill-shaped)
/// format. Internal to the parser — the engine consumes `Mission`.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct MissionFrontmatter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    description: String,

    #[serde(default)]
    schedule: String,
    #[serde(default)]
    enabled: bool,
    #[serde(
        rename = "catchup_hours",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    catchup_hours: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    kickoff: Vec<String>,

    #[serde(
        rename = "kickoff-day",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    kickoff_day: Vec<String>,

    #[serde(
        rename = "allowed-tools",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    permission: Option<MissionPermission>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    created_at: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// Pre-redesign shape with `mode:` / `permission_tier:`. Parser
/// falls back to this when the new parse looks empty or fails.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct LegacyFrontmatter {
    #[serde(default)]
    schedule: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    permission_tier: Option<String>,
    #[serde(default)]
    policy: Option<String>,
    #[serde(default)]
    created_at: u64,
}

/// True if the YAML has legacy markers: a `permission_tier:` field,
/// or a top-level `mode:` line. `starts_with("mode:")` only matches
/// at column zero, so the new format's nested `permission.mode:`
/// does not trigger a false positive.
fn yaml_looks_legacy(yaml: &str) -> bool {
    yaml.contains("permission_tier:")
        || yaml.lines().any(|line| line.starts_with("mode:"))
}

/// Parse a mission `.md` file. Tries the new format first; on
/// failure (or when the YAML looks legacy) falls back to the
/// legacy parser. Returns an error for `mode: app`.
pub(super) fn parse_mission_md(id: &str, content: &str) -> Result<Mission> {
    let id = id.to_string();
    let (yaml_opt, body_raw) = split_frontmatter(content);
    let body = body_raw.trim_start_matches('\n').trim_end().to_string();

    let Some(yaml) = yaml_opt else {
        return Ok(default_mission(id, content.to_string()));
    };

    if yaml_looks_legacy(yaml) {
        return parse_legacy(&id, yaml, body);
    }

    let fm: MissionFrontmatter = serde_yml::from_str(yaml)
        .map_err(|e| anyhow::anyhow!("Bad frontmatter in {}: {}", id, e))?;

    Ok(Mission {
        id: id.clone(),
        name: fm.name.clone().or_else(|| Some(id_to_display_name(&id))),
        description: fm.description,
        schedule: fm.schedule,
        enabled: fm.enabled,
        catchup_hours: fm.catchup_hours,
        cwd: fm.cwd,
        model: fm.model,
        kickoff: fm.kickoff,
        kickoff_day: fm.kickoff_day,
        allowed_tools: fm.allowed_tools,
        permission: fm.permission,
        prompt: body,
        agent_id: fm.agent.unwrap_or_else(|| MISSION_AGENT_ID.to_string()),
        project: fm.project,
        created_at: fm.created_at,
    })
}

fn default_mission(id: String, prompt: String) -> Mission {
    Mission {
        name: Some(id_to_display_name(&id)),
        id,
        description: String::new(),
        schedule: String::new(),
        enabled: false,
        catchup_hours: None,
        cwd: None,
        model: None,
        kickoff: Vec::new(),
        kickoff_day: Vec::new(),
        allowed_tools: Vec::new(),
        permission: None,
        prompt,
        agent_id: MISSION_AGENT_ID.to_string(),
        project: None,
        created_at: 0,
    }
}

fn parse_legacy(id: &str, yaml: &str, body: String) -> Result<Mission> {
    let fm: LegacyFrontmatter = serde_yml::from_str(yaml)
        .map_err(|e| anyhow::anyhow!("Bad legacy frontmatter in {}: {}", id, e))?;

    if fm.mode.as_deref() == Some("app") {
        bail!(
            "Mission '{}' uses legacy mode: app — no longer supported. \
             Convert to a script-only mission or remove.",
            id
        );
    }

    let prompt = if fm.mode.as_deref() == Some("script") {
        String::new()
    } else {
        body
    };

    // Legacy `permission_tier` is no longer honored; missions now
    // use the per-path `permission.paths` shape exclusively. Files
    // in the legacy format load with no permission grants — authors
    // must migrate to `permission.paths`.
    let permission = None::<MissionPermission>;

    Ok(Mission {
        id: id.to_string(),
        name: Some(id_to_display_name(id)),
        description: String::new(),
        schedule: fm.schedule,
        enabled: fm.enabled,
        catchup_hours: None,
        cwd: fm.project.clone(),
        model: fm.model,
        kickoff: Vec::new(),
        kickoff_day: Vec::new(),
        allowed_tools: Vec::new(),
        permission,
        prompt,
        agent_id: MISSION_AGENT_ID.to_string(),
        project: fm.project,
        created_at: fm.created_at,
    })
}

/// Convert a mission to its `.md` file content in the new format.
/// `agent:` is omitted from the frontmatter when it's the default
/// (`MISSION_AGENT_ID`) — keeps mission.md files clean for the
/// common case.
pub(super) fn mission_to_md(mission: &Mission) -> String {
    let agent_fm = if mission.agent_id == MISSION_AGENT_ID {
        None
    } else {
        Some(mission.agent_id.clone())
    };
    let fm = MissionFrontmatter {
        name: mission.name.clone(),
        description: mission.description.clone(),
        schedule: mission.schedule.clone(),
        enabled: mission.enabled,
        catchup_hours: mission.catchup_hours,
        cwd: mission.cwd.clone(),
        model: mission.model.clone(),
        agent: agent_fm,
        kickoff: mission.kickoff.clone(),
        kickoff_day: mission.kickoff_day.clone(),
        allowed_tools: mission.allowed_tools.clone(),
        permission: mission.permission.clone(),
        project: mission.project.clone(),
        created_at: mission.created_at,
    };
    let yaml = serde_yml::to_string(&fm).unwrap_or_default();
    format!("---\n{}---\n\n{}\n", yaml, mission.prompt)
}

/// Convert id like "daily-code-review" to "Daily Code Review".
pub(super) fn id_to_display_name(id: &str) -> String {
    id.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + &chars.collect::<String>(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Sanitize a name to a safe filename (lowercase, hyphens, no
/// special chars). Collapses runs of hyphens; trims leading/trailing.
pub(super) fn name_to_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(c)
            } else if c == ' ' || c == '_' {
                '-'
            } else {
                '-'
            }
        })
        .collect();
    let mut result = String::new();
    let mut prev_hyphen = false;
    for c in sanitized.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push('-');
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }
    result.trim_matches('-').to_string()
}
