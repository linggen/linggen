use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use super::ProjectStore;

/// A cron-scheduled mission — one entry in the project's "crontab".
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Mission {
    pub id: String,
    pub schedule: String,
    pub agent_id: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub enabled: bool,
    pub created_at: u64,
}

/// A single entry in a mission's run history (`runs.jsonl`).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MissionRunEntry {
    pub run_id: String,
    pub triggered_at: u64,
    pub status: String,
    pub skipped: bool,
}

/// Convert a 5-field cron expression to the 7-field format the `cron` crate expects.
/// Also normalizes day-of-week: standard cron uses 0=Sunday, but the crate uses 1=Sunday/7=Sunday.
fn to_seven_field(schedule: &str) -> Result<String> {
    let fields: Vec<&str> = schedule.split_whitespace().collect();
    if fields.len() != 5 {
        bail!(
            "Invalid cron expression '{}': expected 5 fields (min hour dom month dow)",
            schedule
        );
    }
    // Normalize dow field: standard cron 0=Sunday → crate's 7=Sunday.
    // Handle ranges specially: "0-5" → "1-5,7" (expand Sunday out of range).
    let dow = fields[4]
        .split(',')
        .flat_map(|part| {
            if let Some((start_s, end_s)) = part.split_once('-') {
                let start_num = start_s.trim().parse::<u8>().ok();
                let end_num = end_s.trim().parse::<u8>().ok();
                match (start_num, end_num) {
                    (Some(0), Some(e)) if e >= 1 => {
                        // "0-N" → "1-N,7" (pull Sunday out, remap to 7)
                        vec![format!("1-{}", e), "7".to_string()]
                    }
                    (Some(s), Some(0)) if s >= 1 => {
                        // "N-0" (unusual but handle) → "N-7"
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

fn generate_mission_id() -> String {
    format!(
        "mission-{}-{}",
        crate::util::now_ts_secs(),
        &uuid::Uuid::new_v4().to_string()[..8]
    )
}

impl ProjectStore {
    pub fn missions_dir(&self, project_path: &str) -> PathBuf {
        self.project_dir(project_path).join("missions")
    }

    fn mission_dir(&self, project_path: &str, mission_id: &str) -> Result<PathBuf> {
        anyhow::ensure!(
            !mission_id.contains('/') && !mission_id.contains('\\') && !mission_id.contains(".."),
            "invalid mission_id: must not contain path separators"
        );
        Ok(self.missions_dir(project_path).join(mission_id))
    }

    pub fn create_mission(
        &self,
        project_path: &str,
        schedule: &str,
        agent_id: &str,
        prompt: &str,
        model: Option<String>,
    ) -> Result<Mission> {
        validate_cron(schedule)?;

        let mission = Mission {
            id: generate_mission_id(),
            schedule: schedule.to_string(),
            agent_id: agent_id.to_string(),
            prompt: prompt.to_string(),
            model,
            enabled: true,
            created_at: crate::util::now_ts_secs(),
        };

        let dir = self.mission_dir(project_path, &mission.id)?;
        fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(&mission)?;
        fs::write(dir.join("mission.json"), json)?;

        Ok(mission)
    }

    pub fn get_mission_by_id(
        &self,
        project_path: &str,
        mission_id: &str,
    ) -> Result<Option<Mission>> {
        let path = self
            .mission_dir(project_path, mission_id)?
            .join("mission.json");
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path)?;
        let mission: Mission = serde_json::from_str(&content)?;
        Ok(Some(mission))
    }

    pub fn update_mission(
        &self,
        project_path: &str,
        mission_id: &str,
        schedule: Option<&str>,
        agent_id: Option<&str>,
        prompt: Option<&str>,
        model: Option<Option<String>>,
        enabled: Option<bool>,
    ) -> Result<Mission> {
        let Some(mut mission) = self.get_mission_by_id(project_path, mission_id)? else {
            bail!("Mission '{}' not found", mission_id);
        };

        if let Some(s) = schedule {
            validate_cron(s)?;
            mission.schedule = s.to_string();
        }
        if let Some(a) = agent_id {
            mission.agent_id = a.to_string();
        }
        if let Some(p) = prompt {
            mission.prompt = p.to_string();
        }
        if let Some(m) = model {
            mission.model = m;
        }
        if let Some(e) = enabled {
            mission.enabled = e;
        }

        let dir = self.mission_dir(project_path, &mission.id)?;
        let json = serde_json::to_string_pretty(&mission)?;
        fs::write(dir.join("mission.json"), json)?;

        Ok(mission)
    }

    pub fn delete_mission(&self, project_path: &str, mission_id: &str) -> Result<()> {
        let dir = self.mission_dir(project_path, mission_id)?;
        let mission_file = dir.join("mission.json");
        if mission_file.exists() {
            fs::remove_file(&mission_file)?;
        }
        // Keep runs.jsonl (spec: "Run history is preserved")
        Ok(())
    }

    pub fn list_all_missions(&self, project_path: &str) -> Result<Vec<Mission>> {
        let dir = self.missions_dir(project_path);
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut missions = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let mission_file = entry.path().join("mission.json");
            if !mission_file.exists() {
                continue;
            }
            let content = match fs::read_to_string(&mission_file) {
                Ok(c) => c,
                Err(_) => continue,
            };
            match serde_json::from_str::<Mission>(&content) {
                Ok(m) => missions.push(m),
                Err(e) => {
                    tracing::warn!(
                        "Skipping corrupt mission.json at {}: {}",
                        mission_file.display(),
                        e
                    );
                }
            }
        }
        missions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(missions)
    }

    pub fn list_enabled_missions(&self, project_path: &str) -> Result<Vec<Mission>> {
        Ok(self
            .list_all_missions(project_path)?
            .into_iter()
            .filter(|m| m.enabled)
            .collect())
    }

    pub fn append_mission_run(
        &self,
        project_path: &str,
        mission_id: &str,
        entry: &MissionRunEntry,
    ) -> Result<()> {
        let dir = self.mission_dir(project_path, mission_id)?;
        fs::create_dir_all(&dir)?;
        let path = dir.join("runs.jsonl");
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let line = serde_json::to_string(entry)?;
        writeln!(file, "{}", line)?;
        Ok(())
    }

    pub fn list_mission_runs(
        &self,
        project_path: &str,
        mission_id: &str,
    ) -> Result<Vec<MissionRunEntry>> {
        let path = self
            .mission_dir(project_path, mission_id)?
            .join("runs.jsonl");
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
        Ok(entries)
    }

    /// Migrate old-format missions (flat `{timestamp}.json` files) to new
    /// directory-per-mission format.
    pub fn migrate_old_missions(&self, project_path: &str) -> Result<()> {
        let dir = self.missions_dir(project_path);
        if !dir.exists() {
            return Ok(());
        }

        // Also migrate legacy single mission.json at project root
        let legacy = self.project_dir(project_path).join("mission.json");
        if legacy.exists() {
            if let Ok(content) = fs::read_to_string(&legacy) {
                if let Ok(old) = serde_json::from_str::<OldMission>(&content) {
                    self.migrate_one_old_mission(project_path, &old)?;
                }
            }
            let _ = fs::remove_file(&legacy);
        }

        // Scan for flat {timestamp}.json files in missions/
        let mut old_files = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) && path.is_file() {
                // It's a flat file, not a directory — old format
                old_files.push(path);
            }
        }

        for path in old_files {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(old) = serde_json::from_str::<OldMission>(&content) {
                    let _ = self.migrate_one_old_mission(project_path, &old);
                }
            }
            let _ = fs::remove_file(&path);
        }

        Ok(())
    }

    fn migrate_one_old_mission(&self, project_path: &str, old: &OldMission) -> Result<Mission> {
        let agent_id = old
            .agents
            .first()
            .map(|a| a.id.clone())
            .unwrap_or_else(|| "ling".to_string());

        let mission = Mission {
            id: generate_mission_id(),
            schedule: "0 * * * *".to_string(), // safe default: every hour
            agent_id,
            prompt: old.text.clone(),
            model: None,
            enabled: false, // disabled — user should configure cron and re-enable
            created_at: old.created_at,
        };

        let dir = self.mission_dir(project_path, &mission.id)?;
        fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(&mission)?;
        fs::write(dir.join("mission.json"), json)?;

        Ok(mission)
    }
}

/// Old mission format for migration.
#[derive(Deserialize)]
struct OldMission {
    text: String,
    created_at: u64,
    #[allow(dead_code)]
    active: bool,
    #[serde(default)]
    agents: Vec<OldMissionAgent>,
}

#[derive(Deserialize)]
struct OldMissionAgent {
    id: String,
    #[allow(dead_code)]
    #[serde(default)]
    idle_prompt: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    idle_interval_secs: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (ProjectStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::with_root(dir.path().to_path_buf());
        (store, dir)
    }

    #[test]
    fn test_validate_cron() {
        assert!(validate_cron("*/30 * * * *").is_ok());
        assert!(validate_cron("0 9 * * 1-5").is_ok());
        assert!(validate_cron("0 0 * * 0").is_ok()); // 0 = Sunday (normalized to 7)
        assert!(validate_cron("0 0 * * SUN").is_ok());
        assert!(validate_cron("0 */2 * * *").is_ok());
        assert!(validate_cron("0 9 * * 0-5").is_ok()); // Sun-Fri range with 0
        assert!(validate_cron("0 9 * * 0,3,5").is_ok()); // Sun,Wed,Fri list with 0
        assert!(validate_cron("invalid").is_err());
        assert!(validate_cron("").is_err());
        assert!(validate_cron("* * *").is_err()); // only 3 fields
    }

    #[test]
    fn test_create_and_list_missions() {
        let (store, _dir) = temp_store();
        store
            .add_project("/tmp/p".into(), "p".into())
            .unwrap();

        let m1 = store
            .create_mission("/tmp/p", "*/30 * * * *", "ling", "Check status", None)
            .unwrap();
        assert!(m1.id.starts_with("mission-"));
        assert!(m1.enabled);

        let m2 = store
            .create_mission("/tmp/p", "0 9 * * 1-5", "coder", "Review code", Some("gpt-4".into()))
            .unwrap();

        let all = store.list_all_missions("/tmp/p").unwrap();
        assert_eq!(all.len(), 2);

        let enabled = store.list_enabled_missions("/tmp/p").unwrap();
        assert_eq!(enabled.len(), 2);
    }

    #[test]
    fn test_get_update_delete_mission() {
        let (store, _dir) = temp_store();
        store
            .add_project("/tmp/p".into(), "p".into())
            .unwrap();

        let m = store
            .create_mission("/tmp/p", "0 * * * *", "ling", "Hello", None)
            .unwrap();

        // Get
        let loaded = store.get_mission_by_id("/tmp/p", &m.id).unwrap().unwrap();
        assert_eq!(loaded.prompt, "Hello");

        // Update
        let updated = store
            .update_mission("/tmp/p", &m.id, Some("*/15 * * * *"), None, Some("Updated"), None, Some(false))
            .unwrap();
        assert_eq!(updated.schedule, "*/15 * * * *");
        assert_eq!(updated.prompt, "Updated");
        assert!(!updated.enabled);

        // Enabled list should be empty
        assert_eq!(store.list_enabled_missions("/tmp/p").unwrap().len(), 0);

        // Delete
        store.delete_mission("/tmp/p", &m.id).unwrap();
        assert!(store.get_mission_by_id("/tmp/p", &m.id).unwrap().is_none());
    }

    #[test]
    fn test_mission_run_history() {
        let (store, _dir) = temp_store();
        store
            .add_project("/tmp/p".into(), "p".into())
            .unwrap();

        let m = store
            .create_mission("/tmp/p", "0 * * * *", "ling", "Test", None)
            .unwrap();

        let entry1 = MissionRunEntry {
            run_id: "run-1".into(),
            triggered_at: 1000,
            status: "completed".into(),
            skipped: false,
        };
        let entry2 = MissionRunEntry {
            run_id: "run-2".into(),
            triggered_at: 2000,
            status: "completed".into(),
            skipped: true,
        };

        store.append_mission_run("/tmp/p", &m.id, &entry1).unwrap();
        store.append_mission_run("/tmp/p", &m.id, &entry2).unwrap();

        let runs = store.list_mission_runs("/tmp/p", &m.id).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].run_id, "run-1");
        assert!(!runs[0].skipped);
        assert!(runs[1].skipped);
    }

    #[test]
    fn test_update_invalid_cron_rejected() {
        let (store, _dir) = temp_store();
        store
            .add_project("/tmp/p".into(), "p".into())
            .unwrap();

        let m = store
            .create_mission("/tmp/p", "0 * * * *", "ling", "Test", None)
            .unwrap();

        let result = store.update_mission("/tmp/p", &m.id, Some("bad cron"), None, None, None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_migrate_old_flat_missions() {
        let (store, _dir) = temp_store();
        store
            .add_project("/tmp/p".into(), "p".into())
            .unwrap();

        // Create old-format flat file
        let missions_dir = store.missions_dir("/tmp/p");
        fs::create_dir_all(&missions_dir).unwrap();
        let old_json = r#"{"text":"Old mission","created_at":1000,"active":true,"agents":[{"id":"coder"}]}"#;
        fs::write(missions_dir.join("1000.json"), old_json).unwrap();

        store.migrate_old_missions("/tmp/p").unwrap();

        // Old flat file should be gone
        assert!(!missions_dir.join("1000.json").exists());

        // New mission should exist
        let all = store.list_all_missions("/tmp/p").unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].prompt, "Old mission");
        assert_eq!(all[0].agent_id, "coder");
        assert!(!all[0].enabled); // migrated as disabled
        assert_eq!(all[0].schedule, "0 * * * *");
    }
}
