pub mod scheduler;

use anyhow::{bail, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, Write};
use std::path::PathBuf;

// Records + lookup contracts live in `engine::mission`. Re-exported
// here so existing callers (`extensions::missions::Mission`, etc.)
// keep working without churning every import site.
pub use crate::engine::mission::record::{
    Mission, MissionPermission, MissionRunEntry, MISSION_AGENT_ID,
};
pub use crate::engine::mission::registry::MissionRegistry;
pub use crate::engine::mission::runs::MissionRunStore;

// ---------------------------------------------------------------------------
// Frontmatter — new (skill-shaped) format
// ---------------------------------------------------------------------------

/// YAML frontmatter for a `mission.md` file.
///
/// Mirrors SKILL.md fields (`description`, `allowed-tools`, `permission`) and
/// adds mission-specific scheduling/autonomy fields. See `doc/mission-spec.md`.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct MissionFrontmatter {
    /// Display name. Defaults to the directory name if omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    description: String,

    // Scheduling
    #[serde(default)]
    schedule: String,
    #[serde(default)]
    enabled: bool,
    /// Hours since last run before the post-turn seam fires a catch-up.
    /// Omit / None to opt out — only scheduled cron runs apply.
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
    /// Agent that runs the mission. Defaults to "ling" via the parser.
    /// Must match an agent registered in `agents/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    entry: Option<String>,

    // Capabilities (SKILL.md shape)
    #[serde(
        rename = "allowed-tools",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    permission: Option<MissionPermission>,

    // Legacy project field kept for back-compat on write; old sessions used it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    created_at: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

// ---------------------------------------------------------------------------
// Legacy frontmatter — pre-redesign format. Parser falls back to this shape
// when the new parse succeeds but looks empty, or when it fails outright.
// Migrated to the new format on next write. See doc/mission-spec.md Migration.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct LegacyFrontmatter {
    #[serde(default)]
    schedule: String,
    #[serde(default)]
    enabled: bool,
    /// Legacy: "agent" | "app" | "script". "app" is unsupported — rejected at load.
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    entry: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    project: Option<String>,
    /// Legacy: "readonly" | "standard" | "full". Maps to permission.mode.
    #[serde(default)]
    permission_tier: Option<String>,
    /// Legacy autonomy policy preset — read but ignored (no longer used).
    #[serde(default)]
    policy: Option<String>,
    #[serde(default)]
    created_at: u64,
}

// ---------------------------------------------------------------------------
// MissionDraft — builder used by CRUD to avoid unreadable positional args
// ---------------------------------------------------------------------------

/// Input to `MissionLoader::create_mission` / `update_mission`.
/// All fields optional; update applies only what's `Some`.
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

// ---------------------------------------------------------------------------
// Cron helpers
// ---------------------------------------------------------------------------

/// Convert a 5-field cron expression to the 7-field format the `cron` crate expects.
fn to_seven_field(schedule: &str) -> Result<String> {
    let fields: Vec<&str> = schedule.split_whitespace().collect();
    if fields.len() != 5 {
        bail!(
            "Invalid cron expression '{}': expected 5 fields (min hour dom month dow)",
            schedule
        );
    }
    let dow = fields[4]
        .split(',')
        .flat_map(|part| {
            if let Some((start_s, end_s)) = part.split_once('-') {
                let start_num = start_s.trim().parse::<u8>().ok();
                let end_num = end_s.trim().parse::<u8>().ok();
                match (start_num, end_num) {
                    (Some(0), Some(e)) if e >= 1 => {
                        vec![format!("1-{}", e), "7".to_string()]
                    }
                    (Some(s), Some(0)) if s >= 1 => {
                        vec![format!("{}-7", s)]
                    }
                    _ => vec![part.to_string()],
                }
            } else if part.trim() == "0" {
                vec!["7".to_string()]
            } else {
                vec![part.to_string()]
            }
        })
        .collect::<Vec<_>>()
        .join(",");

    Ok(format!(
        "0 {} {} {} {} {} *",
        fields[0], fields[1], fields[2], fields[3], dow
    ))
}

pub fn validate_cron(schedule: &str) -> Result<()> {
    let seven = to_seven_field(schedule)?;
    seven.parse::<cron::Schedule>().map_err(|e| {
        anyhow::anyhow!("Invalid cron expression '{}': {}", schedule, e)
    })?;
    Ok(())
}

pub fn parse_cron(schedule: &str) -> Result<cron::Schedule> {
    let seven = to_seven_field(schedule)?;
    seven
        .parse::<cron::Schedule>()
        .map_err(|e| anyhow::anyhow!("Invalid cron expression '{}': {}", schedule, e))
}

// ---------------------------------------------------------------------------
// Markdown serialisation
// ---------------------------------------------------------------------------

use crate::extensions::frontmatter::split as split_frontmatter;

/// True if the YAML has legacy markers: a `permission_tier:` field, or a
/// top-level `mode:` line. `line.starts_with("mode:")` only matches at
/// column zero, so the new format's nested `permission.mode:` (indented)
/// does not trigger a false positive.
fn yaml_looks_legacy(yaml: &str) -> bool {
    yaml.contains("permission_tier:")
        || yaml.lines().any(|line| line.starts_with("mode:"))
}

/// Parse a mission `.md` file. Tries the new format first; on failure (or when
/// the YAML looks legacy) falls back to the legacy parser and maps fields.
///
/// Returns an error for missions with `mode: app` — unsupported in the redesign.
fn parse_mission_md(id: &str, content: &str) -> Result<Mission> {
    let id = id.to_string();
    let (yaml_opt, body_raw) = split_frontmatter(content);
    let body = body_raw.trim_start_matches('\n').trim_end().to_string();

    // No frontmatter → treat body as prompt, everything else default.
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
        entry: fm.entry,
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
        entry: None,
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

    // Legacy script missions: command lived in `entry`, prompt was ignored.
    // New shape: entry is the pre-agent script, prompt is the body. For
    // script-mode legacy missions we keep entry, drop body. For agent-mode
    // (the common case) the legacy `prompt` was the body of the file.
    let prompt = if fm.mode.as_deref() == Some("script") {
        String::new()
    } else {
        body
    };

    // Legacy `permission_tier` is no longer honored — missions now use
    // the per-path `permission.paths` shape exclusively. Files in the
    // legacy format will load with no permission grants; authors must
    // migrate to `permission.paths`.
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
        entry: fm.entry,
        allowed_tools: Vec::new(),
        permission,
        prompt,
        agent_id: MISSION_AGENT_ID.to_string(),
        project: fm.project,
        created_at: fm.created_at,
    })
}

/// Convert a mission to its `.md` file content in the new format.
fn mission_to_md(mission: &Mission) -> String {
    // Only serialize `agent:` when it isn't the default; keeps mission.md
    // files clean for the common case.
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
        entry: mission.entry.clone(),
        allowed_tools: mission.allowed_tools.clone(),
        permission: mission.permission.clone(),
        project: mission.project.clone(),
        created_at: mission.created_at,
    };
    let yaml = serde_yml::to_string(&fm).unwrap_or_default();
    format!("---\n{}---\n\n{}\n", yaml, mission.prompt)
}

/// Convert id like "daily-code-review" to "Daily Code Review".
fn id_to_display_name(id: &str) -> String {
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

/// Sanitize a name to a safe filename (lowercase, hyphens, no special chars).
fn name_to_filename(name: &str) -> String {
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

// ---------------------------------------------------------------------------
// MissionLoader — global mission storage at ~/.linggen/missions/
// ---------------------------------------------------------------------------

pub struct MissionLoader {
    dir: PathBuf,
    cache: std::sync::Mutex<Vec<Mission>>,
}

impl MissionLoader {
    pub fn new() -> Self {
        let store = Self {
            dir: crate::paths::global_missions_dir(),
            cache: std::sync::Mutex::new(Vec::new()),
        };
        store.reload();
        store
    }

    #[cfg(test)]
    pub fn with_dir(dir: PathBuf) -> Self {
        let store = Self {
            dir,
            cache: std::sync::Mutex::new(Vec::new()),
        };
        store.reload();
        store
    }

    pub fn reload(&self) {
        let missions = self.scan_disk().unwrap_or_default();
        *self.cache.lock().unwrap() = missions;
    }

    fn ensure_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        Ok(())
    }

    pub fn mission_dir(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    fn mission_path(&self, id: &str) -> PathBuf {
        self.dir.join(id).join("mission.md")
    }

    fn runs_path(&self, id: &str) -> PathBuf {
        self.dir.join(id).join("runs.jsonl")
    }

    /// Create a mission from a draft. Required: schedule, prompt (unless
    /// draft.entry is set, indicating a script-only mission).
    pub fn create_mission(&self, draft: MissionDraft) -> Result<Mission> {
        let schedule = draft
            .schedule
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("schedule is required"))?;
        validate_cron(schedule)?;

        let prompt = draft.prompt.clone().unwrap_or_default();
        let entry = draft.entry.clone().flatten();
        if prompt.trim().is_empty() && entry.as_deref().map(str::trim).unwrap_or("").is_empty() {
            bail!("Mission requires a prompt body or an entry script");
        }

        self.ensure_dir()?;

        let display_name = draft
            .name
            .clone()
            .unwrap_or_else(|| "new-mission".to_string());
        let mut id = name_to_filename(&display_name);
        if id.is_empty() {
            id = format!("mission-{}", crate::util::now_ts_secs());
        }
        if self.mission_dir(&id).exists() {
            let base = id.clone();
            let mut n = 2;
            loop {
                id = format!("{}-{}", base, n);
                if !self.mission_dir(&id).exists() {
                    break;
                }
                n += 1;
            }
        }

        let mission = Mission {
            id: id.clone(),
            name: Some(display_name),
            description: draft.description.clone().unwrap_or_default(),
            schedule: schedule.to_string(),
            enabled: draft.enabled.unwrap_or(true),
            catchup_hours: draft.catchup_hours.clone().flatten(),
            cwd: draft.cwd.clone().flatten(),
            model: draft.model.clone().flatten(),
            entry,
            allowed_tools: draft.allowed_tools.clone().unwrap_or_default(),
            permission: draft.permission.clone().flatten(),
            prompt,
            agent_id: draft
                .agent
                .clone()
                .unwrap_or_else(|| MISSION_AGENT_ID.to_string()),
            project: draft.project.clone().flatten(),
            created_at: crate::util::now_ts_secs(),
        };

        fs::create_dir_all(self.mission_dir(&id))?;
        fs::write(self.mission_path(&id), mission_to_md(&mission))?;
        self.reload();

        Ok(mission)
    }

    pub fn get_mission(&self, mission_id: &str) -> Result<Option<Mission>> {
        let path = self.mission_path(mission_id);
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path)?;
        let mission = parse_mission_md(mission_id, &content)?;
        Ok(Some(mission))
    }

    /// Update a mission by applying a draft. Fields left `None` are untouched.
    pub fn update_mission(&self, mission_id: &str, draft: MissionDraft) -> Result<Mission> {
        let Some(mut mission) = self.get_mission(mission_id)? else {
            bail!("Mission '{}' not found", mission_id);
        };

        if let Some(n) = draft.name {
            mission.name = Some(n);
        }
        if let Some(d) = draft.description {
            mission.description = d;
        }
        if let Some(s) = draft.schedule {
            validate_cron(&s)?;
            mission.schedule = s;
        }
        if let Some(e) = draft.enabled {
            mission.enabled = e;
        }
        if let Some(ch) = draft.catchup_hours {
            mission.catchup_hours = ch;
        }
        if let Some(cwd) = draft.cwd {
            mission.cwd = cwd;
        }
        if let Some(m) = draft.model {
            mission.model = m;
        }
        if let Some(a) = draft.agent {
            mission.agent_id = a;
        }
        if let Some(e) = draft.entry {
            mission.entry = e;
        }
        if let Some(t) = draft.allowed_tools {
            mission.allowed_tools = t;
        }
        if let Some(perm) = draft.permission {
            mission.permission = perm;
        }
        if let Some(p) = draft.prompt {
            mission.prompt = p;
        }
        if let Some(p) = draft.project {
            mission.project = p;
        }

        fs::write(self.mission_path(mission_id), mission_to_md(&mission))?;
        self.reload();
        Ok(mission)
    }

    pub fn delete_mission(&self, mission_id: &str) -> Result<()> {
        let dir = self.mission_dir(mission_id);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        self.reload();
        Ok(())
    }

    /// Read the raw `mission.md` content for a mission, or `None` if no
    /// such file exists. Used by the raw-markdown editor in the Web UI.
    pub fn read_mission_raw(&self, mission_id: &str) -> Result<Option<String>> {
        let path = self.mission_path(mission_id);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(fs::read_to_string(&path)?))
    }

    /// Validate and write `mission.md` content for a mission. The mission
    /// id is taken from the filesystem layout (`<dir>/<id>/mission.md`)
    /// rather than the frontmatter `name`, matching how `scan_disk`
    /// already reads them back. Creates the mission dir if absent.
    pub fn write_mission_raw(&self, mission_id: &str, content: &str) -> Result<Mission> {
        let mission = parse_mission_md(mission_id, content)?;
        fs::create_dir_all(self.mission_dir(mission_id))?;
        fs::write(self.mission_path(mission_id), content)?;
        self.reload();
        Ok(mission)
    }

    pub fn list_all_missions(&self) -> Result<Vec<Mission>> {
        Ok(self.cache.lock().unwrap().clone())
    }

    pub fn list_enabled_missions(&self) -> Result<Vec<Mission>> {
        Ok(self
            .cache
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.enabled)
            .cloned()
            .collect())
    }

    fn scan_disk(&self) -> Result<Vec<Mission>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let mut missions = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let mission_file = path.join("mission.md");
            if !mission_file.exists() {
                continue;
            }
            let id = path.file_name().unwrap().to_string_lossy().to_string();
            let content = match fs::read_to_string(&mission_file) {
                Ok(c) => c,
                Err(_) => continue,
            };
            match parse_mission_md(&id, &content) {
                Ok(m) => missions.push(m),
                Err(e) => {
                    tracing::warn!("Skipping corrupt mission dir {}: {}", id, e);
                }
            }
        }
        missions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(missions)
    }

    pub fn append_mission_run(
        &self,
        mission_id: &str,
        entry: &MissionRunEntry,
    ) -> Result<()> {
        fs::create_dir_all(self.mission_dir(mission_id))?;
        let path = self.runs_path(mission_id);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let line = serde_json::to_string(entry)?;
        writeln!(file, "{}", line)?;
        Ok(())
    }

    pub fn list_mission_runs(&self, mission_id: &str) -> Result<Vec<MissionRunEntry>> {
        self.list_mission_runs_paginated(mission_id, None, None)
    }

    /// List mission runs newest-first with optional pagination.
    pub fn list_mission_runs_paginated(
        &self,
        mission_id: &str,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<MissionRunEntry>> {
        let path = self.runs_path(mission_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&path)?;
        let reader = std::io::BufReader::new(file);
        let mut entries = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<MissionRunEntry>(&line) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    tracing::warn!("Skipping corrupt mission run entry: {}", e);
                }
            }
        }
        let total = entries.len();
        entries.reverse();
        let off = offset.unwrap_or(0);
        if off >= total {
            return Ok(Vec::new());
        }
        if off > 0 {
            entries = entries.into_iter().skip(off).collect();
        }
        if let Some(lim) = limit {
            entries.truncate(lim);
        }
        Ok(entries)
    }

    /// Remove the run entry whose `session_id` matches, rewriting `runs.jsonl`.
    pub fn remove_run_by_session(
        &self,
        mission_id: &str,
        session_id: &str,
    ) -> Result<()> {
        let entries = self.list_mission_runs(mission_id)?;
        let filtered: Vec<&MissionRunEntry> = entries
            .iter()
            .filter(|e| e.session_id.as_deref() != Some(session_id))
            .collect();
        let path = self.runs_path(mission_id);
        let mut file = fs::File::create(&path)?;
        for entry in filtered {
            serde_json::to_writer(&mut file, entry)?;
            std::io::Write::write_all(&mut file, b"\n")?;
        }
        Ok(())
    }

    /// Look up the most recent completed (non-skipped) run for a mission.
    /// Used by the scheduler to set `MISSION_LAST_RUN_AT` env for the entry script.
    pub fn last_successful_run_at(&self, mission_id: &str) -> Option<u64> {
        self.list_mission_runs(mission_id)
            .ok()?
            .into_iter()
            .find(|e| !e.skipped && e.status == "completed")
            .map(|e| e.triggered_at)
    }
}

#[async_trait]
impl MissionRegistry for MissionLoader {
    async fn list(&self) -> Result<Vec<Mission>> {
        self.list_all_missions()
    }

    async fn get(&self, mission_id: &str) -> Result<Option<Mission>> {
        self.get_mission(mission_id)
    }
}

impl MissionRunStore for MissionLoader {
    fn append(&self, mission_id: &str, entry: &MissionRunEntry) -> Result<()> {
        self.append_mission_run(mission_id, entry)
    }

    fn list(&self, mission_id: &str) -> Result<Vec<MissionRunEntry>> {
        self.list_mission_runs(mission_id)
    }

    fn list_paginated(
        &self,
        mission_id: &str,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<MissionRunEntry>> {
        self.list_mission_runs_paginated(mission_id, limit, offset)
    }

    fn remove_by_session(&self, mission_id: &str, session_id: &str) -> Result<()> {
        self.remove_run_by_session(mission_id, session_id)
    }

    fn last_successful_run_at(&self, mission_id: &str) -> Option<u64> {
        Self::last_successful_run_at(self, mission_id)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (MissionLoader, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = MissionLoader::with_dir(dir.path().to_path_buf());
        (store, dir)
    }

    fn draft_min(name: &str, schedule: &str, prompt: &str) -> MissionDraft {
        MissionDraft {
            name: Some(name.to_string()),
            schedule: Some(schedule.to_string()),
            prompt: Some(prompt.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_validate_cron() {
        assert!(validate_cron("*/30 * * * *").is_ok());
        assert!(validate_cron("0 9 * * 1-5").is_ok());
        assert!(validate_cron("0 0 * * 0").is_ok());
        assert!(validate_cron("0 0 * * SUN").is_ok());
        assert!(validate_cron("0 */2 * * *").is_ok());
        assert!(validate_cron("0 9 * * 0-5").is_ok());
        assert!(validate_cron("0 9 * * 0,3,5").is_ok());
        assert!(validate_cron("invalid").is_err());
        assert!(validate_cron("").is_err());
        assert!(validate_cron("* * *").is_err());
    }

    #[test]
    fn test_create_and_list() {
        let (store, _dir) = temp_store();
        let m1 = store
            .create_mission(draft_min("Check Status", "*/30 * * * *", "Check status"))
            .unwrap();
        assert_eq!(m1.id, "check-status");
        assert!(m1.enabled);
        assert_eq!(m1.agent_id, MISSION_AGENT_ID);

        let m2 = store
            .create_mission(draft_min("Review Code", "0 9 * * 1-5", "Review"))
            .unwrap();
        assert_eq!(m2.id, "review-code");

        assert_eq!(store.list_all_missions().unwrap().len(), 2);
    }

    #[test]
    fn test_md_roundtrip_new_format() {
        let (store, _dir) = temp_store();
        let draft = MissionDraft {
            name: Some("Daily Cleanup".into()),
            description: Some("Clean up old files".into()),
            schedule: Some("0 9 * * *".into()),
            prompt: Some("Clean up old files\n\nRemove build artifacts.".into()),
            model: Some(Some("gpt-4".into())),
            cwd: Some(Some("/tmp/proj".into())),
            allowed_tools: Some(vec!["Read".into(), "Bash".into()]),
            permission: Some(Some(MissionPermission {
                paths: vec![crate::extensions::skills::PathGrant {
                    path: "~/.linggen".into(),
                    mode: "admin".into(),
                }],
                warning: Some("test warn".into()),
            })),
            ..Default::default()
        };
        let created = store.create_mission(draft).unwrap();

        let loaded = store.get_mission("daily-cleanup").unwrap().unwrap();
        assert_eq!(loaded.schedule, "0 9 * * *");
        assert_eq!(loaded.prompt, "Clean up old files\n\nRemove build artifacts.");
        assert_eq!(loaded.model, Some("gpt-4".to_string()));
        assert_eq!(loaded.cwd, Some("/tmp/proj".to_string()));
        assert_eq!(loaded.allowed_tools, vec!["Read".to_string(), "Bash".to_string()]);
        let perm = loaded.permission.as_ref().unwrap();
        assert_eq!(perm.paths.len(), 1);
        assert_eq!(perm.paths[0].path, "~/.linggen");
        assert_eq!(perm.paths[0].mode, "admin");
        assert_eq!(perm.warning.as_deref(), Some("test warn"));
        assert!(loaded.enabled);
        assert_eq!(loaded.created_at, created.created_at);
    }

    #[test]
    fn test_agent_and_catchup_hours_roundtrip() {
        let (store, _dir) = temp_store();
        let draft = MissionDraft {
            name: Some("Memory Worker".into()),
            schedule: Some("0 3 * * *".into()),
            prompt: Some("Body".into()),
            agent: Some("ling-mem".into()),
            catchup_hours: Some(Some(24)),
            ..Default::default()
        };
        store.create_mission(draft).unwrap();

        let loaded = store.get_mission("memory-worker").unwrap().unwrap();
        assert_eq!(loaded.agent_id, "ling-mem");
        assert_eq!(loaded.catchup_hours, Some(24));

        // Default agent ("ling") and missing catchup_hours must round-trip
        // as their defaults — and `mission_to_md` must not emit them.
        let plain = store
            .create_mission(draft_min("Plain", "0 * * * *", "p"))
            .unwrap();
        let plain_md =
            std::fs::read_to_string(store.mission_path(&plain.id)).unwrap();
        assert!(!plain_md.contains("agent:"), "default agent must not be serialized");
        assert!(
            !plain_md.contains("catchup_hours:"),
            "missing catchup_hours must not be serialized"
        );
        let plain_loaded = store.get_mission(&plain.id).unwrap().unwrap();
        assert_eq!(plain_loaded.agent_id, MISSION_AGENT_ID);
        assert_eq!(plain_loaded.catchup_hours, None);
    }

    #[test]
    fn test_update() {
        let (store, _dir) = temp_store();
        let m = store
            .create_mission(draft_min("Test", "0 * * * *", "Hello"))
            .unwrap();

        let updated = store
            .update_mission(
                &m.id,
                MissionDraft {
                    schedule: Some("*/15 * * * *".into()),
                    prompt: Some("Updated prompt".into()),
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(updated.schedule, "*/15 * * * *");
        assert_eq!(updated.prompt, "Updated prompt");
        assert!(!updated.enabled);
        assert_eq!(store.list_enabled_missions().unwrap().len(), 0);

        store.delete_mission(&m.id).unwrap();
        assert!(store.get_mission(&m.id).unwrap().is_none());
    }

    #[test]
    fn test_run_history() {
        let (store, _dir) = temp_store();
        let m = store
            .create_mission(draft_min("Test", "0 * * * *", "Test"))
            .unwrap();

        let entry1 = MissionRunEntry {
            run_id: "run-1".into(),
            session_id: Some("sess-1".into()),
            triggered_at: 1000,
            status: "completed".into(),
            skipped: false,
            entry_exit_code: None,
            output_dir: None,
        };
        let entry2 = MissionRunEntry {
            run_id: "run-2".into(),
            session_id: None,
            triggered_at: 2000,
            status: "skipped".into(),
            skipped: true,
            entry_exit_code: None,
            output_dir: None,
        };
        store.append_mission_run(&m.id, &entry1).unwrap();
        store.append_mission_run(&m.id, &entry2).unwrap();

        let runs = store.list_mission_runs(&m.id).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].run_id, "run-2");
        assert_eq!(runs[1].run_id, "run-1");
        assert!(runs[0].skipped);
    }

    #[test]
    fn test_legacy_frontmatter_parses() {
        // Legacy mission file — permission_tier + mode + top-level policy.
        let content = "---\n\
            schedule: 0 23 * * *\n\
            enabled: true\n\
            permission_tier: standard\n\
            policy: strict\n\
            created_at: 123\n\
            ---\n\
            \n\
            Do the nightly scan.\n";
        let m = parse_mission_md("nightly", content).unwrap();
        assert_eq!(m.schedule, "0 23 * * *");
        assert!(m.enabled);
        assert!(m.permission.is_none()); // legacy permission_tier no longer honored
        assert_eq!(m.prompt, "Do the nightly scan.");
        assert_eq!(m.created_at, 123);
    }

    #[test]
    fn test_legacy_permission_tier_not_honored() {
        // Legacy `permission_tier` is no longer mapped to a mode — such
        // missions load with no permission grants; authors must migrate to
        // the per-path `permission.paths` shape.
        let ro = "---\nschedule: 0 * * * *\nenabled: true\npermission_tier: readonly\n---\nHi\n";
        let std_ = "---\nschedule: 0 * * * *\nenabled: true\npermission_tier: standard\n---\nHi\n";
        let full = "---\nschedule: 0 * * * *\nenabled: true\npermission_tier: full\n---\nHi\n";

        assert!(parse_mission_md("a", ro).unwrap().permission.is_none());
        assert!(parse_mission_md("b", std_).unwrap().permission.is_none());
        assert!(parse_mission_md("c", full).unwrap().permission.is_none());
    }

    #[test]
    fn test_legacy_app_mode_rejected() {
        let content = "---\n\
            schedule: 0 9 * * *\n\
            enabled: true\n\
            mode: app\n\
            entry: /some/url\n\
            ---\n\
            \n";
        let result = parse_mission_md("bad", content);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("mode: app"), "error was: {}", err);
    }

    #[test]
    fn test_legacy_script_mode_drops_body() {
        // Legacy script mode: entry was the command, body was unused.
        let content = "---\n\
            schedule: 0 9 * * *\n\
            enabled: true\n\
            mode: script\n\
            entry: echo hi\n\
            ---\n\
            \n\
            Some ignored body.\n";
        let m = parse_mission_md("s", content).unwrap();
        assert_eq!(m.entry.as_deref(), Some("echo hi"));
        assert_eq!(m.prompt, ""); // body dropped for script mode
    }

    #[test]
    fn test_legacy_rewrites_to_new_format_on_update() {
        let (store, dir) = temp_store();
        // Seed a legacy mission file directly on disk.
        let legacy_md = "---\n\
            schedule: 0 9 * * *\n\
            enabled: true\n\
            permission_tier: full\n\
            ---\n\
            Hello\n";
        let mdir = dir.path().join("legacy");
        std::fs::create_dir_all(&mdir).unwrap();
        std::fs::write(mdir.join("mission.md"), legacy_md).unwrap();
        store.reload();

        // Update triggers a re-serialize in the new format.
        store
            .update_mission(
                "legacy",
                MissionDraft {
                    description: Some("migrated".into()),
                    ..Default::default()
                },
            )
            .unwrap();

        let content = std::fs::read_to_string(mdir.join("mission.md")).unwrap();
        assert!(!content.contains("permission_tier"));
        assert!(content.contains("description: migrated"));
        // Legacy permission_tier is dropped, not migrated to permission.paths,
        // so the rewritten new-format file carries no `permission:` block.
        assert!(!content.contains("permission:"));
    }

    #[test]
    fn test_name_to_filename() {
        assert_eq!(name_to_filename("Daily Code Review"), "daily-code-review");
        assert_eq!(name_to_filename("clean disk"), "clean-disk");
        assert_eq!(name_to_filename("  hello  world  "), "hello-world");
        assert_eq!(name_to_filename("Test_123"), "test-123");
    }

    #[test]
    fn test_duplicate_name_gets_suffix() {
        let (store, _dir) = temp_store();
        let m1 = store.create_mission(draft_min("Test", "0 * * * *", "First")).unwrap();
        assert_eq!(m1.id, "test");
        let m2 = store.create_mission(draft_min("Test", "0 * * * *", "Second")).unwrap();
        assert_eq!(m2.id, "test-2");
    }

    #[test]
    fn test_create_requires_prompt_or_entry() {
        let (store, _dir) = temp_store();
        let err = store.create_mission(MissionDraft {
            name: Some("empty".into()),
            schedule: Some("0 * * * *".into()),
            ..Default::default()
        });
        assert!(err.is_err());

        // Entry-only mission OK.
        let ok = store.create_mission(MissionDraft {
            name: Some("script-only".into()),
            schedule: Some("0 * * * *".into()),
            entry: Some(Some("scripts/run.sh".into())),
            ..Default::default()
        });
        assert!(ok.is_ok());
    }

    #[test]
    fn test_update_invalid_cron_rejected() {
        let (store, _dir) = temp_store();
        let m = store.create_mission(draft_min("Test", "0 * * * *", "Test")).unwrap();
        let result = store.update_mission(
            &m.id,
            MissionDraft {
                schedule: Some("bad cron".into()),
                ..Default::default()
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_directory_structure() {
        let (store, dir) = temp_store();
        let root = dir.path().to_path_buf();
        let m = store.create_mission(draft_min("Test Dir", "0 * * * *", "Hello")).unwrap();
        assert!(root.join("test-dir").is_dir());
        assert!(root.join("test-dir").join("mission.md").exists());

        let entry = MissionRunEntry {
            run_id: "r1".into(),
            session_id: None,
            triggered_at: 1000,
            status: "completed".into(),
            skipped: false,
            entry_exit_code: None,
            output_dir: None,
        };
        store.append_mission_run(&m.id, &entry).unwrap();
        assert!(root.join("test-dir").join("runs.jsonl").exists());

        store.delete_mission(&m.id).unwrap();
        assert!(!root.join("test-dir").exists());
    }
}
