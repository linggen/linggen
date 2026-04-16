use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, Write};
use std::path::PathBuf;

/// The agent that always runs missions.
pub const MISSION_AGENT_ID: &str = "ling";

/// YAML frontmatter fields for a mission `.md` file.
#[derive(Debug, Serialize, Deserialize, Clone)]
struct MissionFrontmatter {
    #[serde(default)]
    schedule: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    /// Permission tier: "readonly", "standard", "full". Default: "full".
    #[serde(default = "default_permission_tier", skip_serializing_if = "is_default_permission_tier")]
    permission_tier: String,
    #[serde(default)]
    created_at: u64,
}

fn default_permission_tier() -> String {
    "full".to_string()
}

fn is_default_permission_tier(s: &str) -> bool {
    s == "full"
}

/// A cron-scheduled mission stored as `~/.linggen/missions/<id>/mission.md`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Mission {
    /// The mission ID — directory name under `~/.linggen/missions/`.
    pub id: String,
    /// Display name (same as id for md-based missions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub schedule: String,
    /// Always "mission".
    #[serde(default = "default_mission_agent")]
    pub agent_id: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Optional project path this mission is scoped to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Permission tier: "readonly", "standard", "full". Default: "full".
    #[serde(default = "default_permission_tier")]
    pub permission_tier: String,
    pub enabled: bool,
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

/// Validate a 5-field cron expression.
pub fn validate_cron(schedule: &str) -> Result<()> {
    let seven = to_seven_field(schedule)?;
    seven.parse::<cron::Schedule>().map_err(|e| {
        anyhow::anyhow!("Invalid cron expression '{}': {}", schedule, e)
    })?;
    Ok(())
}

/// Parse a 5-field cron expression into a `cron::Schedule`.
pub fn parse_cron(schedule: &str) -> Result<cron::Schedule> {
    let seven = to_seven_field(schedule)?;
    seven
        .parse::<cron::Schedule>()
        .map_err(|e| anyhow::anyhow!("Invalid cron expression '{}': {}", schedule, e))
}

// ---------------------------------------------------------------------------
// Markdown serialisation helpers
// ---------------------------------------------------------------------------

/// Parse a mission `.md` file: YAML frontmatter + markdown body = prompt.
fn parse_mission_md(id: &str, content: &str) -> Result<Mission> {
    let id = id.to_string();

    let (fm, body) = if content.starts_with("---") {
        // Find closing ---
        if let Some(end) = content[3..].find("\n---") {
            let yaml = &content[3..3 + end];
            let body = &content[3 + end + 4..]; // skip "\n---"
            let fm: MissionFrontmatter = serde_yml::from_str(yaml.trim())
                .map_err(|e| anyhow::anyhow!("Bad frontmatter in {}: {}", id, e))?;
            (fm, body.trim().to_string())
        } else {
            // No closing --- — treat entire file as prompt
            (
                MissionFrontmatter {
                    schedule: String::new(),
                    enabled: false,
                    model: None,
                    project: None,
                    permission_tier: default_permission_tier(),
                    created_at: 0,
                },
                content.to_string(),
            )
        }
    } else {
        // No frontmatter at all
        (
            MissionFrontmatter {
                schedule: String::new(),
                enabled: false,
                model: None,
                project: None,
                permission_tier: default_permission_tier(),
                created_at: 0,
            },
            content.to_string(),
        )
    };

    Ok(Mission {
        name: Some(id_to_display_name(&id)),
        id,
        schedule: fm.schedule,
        agent_id: MISSION_AGENT_ID.to_string(),
        prompt: body,
        model: fm.model,
        project: fm.project,
        permission_tier: fm.permission_tier,
        enabled: fm.enabled,
        created_at: fm.created_at,
    })
}

/// Convert a mission to its `.md` file content.
fn mission_to_md(mission: &Mission) -> String {
    let fm = MissionFrontmatter {
        schedule: mission.schedule.clone(),
        enabled: mission.enabled,
        model: mission.model.clone(),
        project: mission.project.clone(),
        permission_tier: mission.permission_tier.clone(),
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
    // Collapse multiple hyphens
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
// MissionStore — global mission storage at ~/.linggen/missions/
// ---------------------------------------------------------------------------

pub struct MissionStore {
    dir: PathBuf,
    cache: std::sync::Mutex<Vec<Mission>>,
}

impl MissionStore {
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

    /// Reload missions from disk into the in-memory cache.
    pub fn reload(&self) {
        let missions = self.scan_disk().unwrap_or_default();
        *self.cache.lock().unwrap() = missions;
    }

    fn ensure_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        Ok(())
    }

    fn mission_dir(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    fn mission_path(&self, id: &str) -> PathBuf {
        self.dir.join(id).join("mission.md")
    }

    fn runs_path(&self, id: &str) -> PathBuf {
        self.dir.join(id).join("runs.jsonl")
    }

    pub fn create_mission(
        &self,
        name: Option<String>,
        schedule: &str,
        prompt: &str,
        model: Option<String>,
        project: Option<String>,
        permission_tier: Option<String>,
    ) -> Result<Mission> {
        validate_cron(schedule)?;
        self.ensure_dir()?;

        let display_name = name.unwrap_or_else(|| "new-mission".to_string());
        let mut id = name_to_filename(&display_name);
        if id.is_empty() {
            id = format!("mission-{}", crate::util::now_ts_secs());
        }

        // Ensure unique directory name
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

        let tier = permission_tier.unwrap_or_else(|| "full".to_string());
        let mission = Mission {
            id: id.clone(),
            name: Some(display_name),
            schedule: schedule.to_string(),
            agent_id: MISSION_AGENT_ID.to_string(),
            prompt: prompt.to_string(),
            model,
            project,
            permission_tier: tier,
            enabled: true,
            created_at: crate::util::now_ts_secs(),
        };

        fs::create_dir_all(self.mission_dir(&id))?;
        let content = mission_to_md(&mission);
        fs::write(self.mission_path(&id), content)?;
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

    pub fn update_mission(
        &self,
        mission_id: &str,
        name: Option<Option<String>>,
        schedule: Option<&str>,
        prompt: Option<&str>,
        model: Option<Option<String>>,
        project: Option<Option<String>>,
        enabled: Option<bool>,
        permission_tier: Option<String>,
    ) -> Result<Mission> {
        let Some(mut mission) = self.get_mission(mission_id)? else {
            bail!("Mission '{}' not found", mission_id);
        };

        if let Some(n) = name {
            mission.name = n;
        }
        if let Some(s) = schedule {
            validate_cron(s)?;
            mission.schedule = s.to_string();
        }
        if let Some(p) = prompt {
            mission.prompt = p.to_string();
        }
        if let Some(m) = model {
            mission.model = m;
        }
        if let Some(p) = project {
            mission.project = p;
        }
        if let Some(e) = enabled {
            mission.enabled = e;
        }
        if let Some(t) = permission_tier {
            mission.permission_tier = t;
        }

        let content = mission_to_md(&mission);
        fs::write(self.mission_path(mission_id), content)?;
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

    /// Scan disk for all missions. Used by reload().
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

    /// List mission runs with optional pagination.
    /// Results are in chronological order (oldest first) from the JSONL file.
    /// `limit` and `offset` apply after reading. For newest-first with limit,
    /// callers should reverse the result.
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
        // Reverse to newest-first, then apply offset/limit
        entries.reverse();
        let off = offset.unwrap_or(0);
        if off > 0 && off < total {
            entries = entries.into_iter().skip(off).collect();
        } else if off >= total {
            return Ok(Vec::new());
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
}


#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (MissionStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = MissionStore::with_dir(dir.path().to_path_buf());
        (store, dir)
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
    fn test_create_and_list_missions() {
        let (store, _dir) = temp_store();

        let m1 = store
            .create_mission(Some("Check Status".into()), "*/30 * * * *", "Check status", None, None, None)
            .unwrap();
        assert_eq!(m1.id, "check-status");
        assert!(m1.enabled);
        assert_eq!(m1.agent_id, MISSION_AGENT_ID);

        let m2 = store
            .create_mission(Some("Review Code".into()), "0 9 * * 1-5", "Review code", Some("gpt-4".into()), Some("/tmp/proj".into()), None)
            .unwrap();
        assert_eq!(m2.id, "review-code");
        assert_eq!(m2.project, Some("/tmp/proj".to_string()));

        let all = store.list_all_missions().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_md_roundtrip() {
        let (store, _dir) = temp_store();

        let created = store
            .create_mission(Some("Daily Cleanup".into()), "0 9 * * *", "Clean up old files\n\nRemove build artifacts.", Some("gpt-4".into()), Some("/tmp/proj".into()), None)
            .unwrap();

        let loaded = store.get_mission("daily-cleanup").unwrap().unwrap();
        assert_eq!(loaded.schedule, "0 9 * * *");
        assert_eq!(loaded.prompt, "Clean up old files\n\nRemove build artifacts.");
        assert_eq!(loaded.model, Some("gpt-4".to_string()));
        assert_eq!(loaded.project, Some("/tmp/proj".to_string()));
        assert_eq!(loaded.enabled, true);
        assert_eq!(loaded.created_at, created.created_at);
    }

    #[test]
    fn test_update_delete() {
        let (store, _dir) = temp_store();

        let m = store
            .create_mission(Some("Test".into()), "0 * * * *", "Hello", None, None, None)
            .unwrap();

        let updated = store
            .update_mission(&m.id, None, Some("*/15 * * * *"), Some("Updated prompt"), None, None, Some(false), None)
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
            .create_mission(Some("Test".into()), "0 * * * *", "Test", None, None, None)
            .unwrap();

        let entry1 = MissionRunEntry {
            run_id: "run-1".into(),
            session_id: Some("sess-1".into()),
            triggered_at: 1000,
            status: "completed".into(),
            skipped: false,
        };
        let entry2 = MissionRunEntry {
            run_id: "run-2".into(),
            session_id: None,
            triggered_at: 2000,
            status: "skipped".into(),
            skipped: true,
        };

        store.append_mission_run(&m.id, &entry1).unwrap();
        store.append_mission_run(&m.id, &entry2).unwrap();

        let runs = store.list_mission_runs(&m.id).unwrap();
        assert_eq!(runs.len(), 2);
        // Newest first (reverse chronological)
        assert_eq!(runs[0].run_id, "run-2");
        assert_eq!(runs[1].run_id, "run-1");
        assert!(runs[0].skipped);
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

        let m1 = store.create_mission(Some("Test".into()), "0 * * * *", "First", None, None, None).unwrap();
        assert_eq!(m1.id, "test");

        let m2 = store.create_mission(Some("Test".into()), "0 * * * *", "Second", None, None, None).unwrap();
        assert_eq!(m2.id, "test-2");
    }

    #[test]
    fn test_update_invalid_cron_rejected() {
        let (store, _dir) = temp_store();

        let m = store
            .create_mission(Some("Test".into()), "0 * * * *", "Test", None, None, None)
            .unwrap();

        let result = store.update_mission(&m.id, None, Some("bad cron"), None, None, None, None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_directory_structure() {
        let (store, dir) = temp_store();
        let root = dir.path().to_path_buf();

        let m = store
            .create_mission(Some("Test Dir".into()), "0 * * * *", "Hello", None, None, None)
            .unwrap();

        // Verify directory-based layout
        assert!(root.join("test-dir").is_dir());
        assert!(root.join("test-dir").join("mission.md").exists());

        // Runs go inside the same directory
        let entry = MissionRunEntry {
            run_id: "r1".into(),
            session_id: None,
            triggered_at: 1000,
            status: "completed".into(),
            skipped: false,
        };
        store.append_mission_run(&m.id, &entry).unwrap();
        assert!(root.join("test-dir").join("runs.jsonl").exists());

        // Delete removes entire directory
        store.delete_mission(&m.id).unwrap();
        assert!(!root.join("test-dir").exists());
    }
}
