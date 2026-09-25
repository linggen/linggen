pub mod cron;
pub mod draft;
pub mod enter;
pub mod parser;
mod report;
pub mod scheduler;
mod skill_missions;

use crate::util::LockExt;
use anyhow::{bail, Result};
use std::fs;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use crate::provider::models::RunUsage;

// Records + lookup contracts live in `engine::mission`. Re-exported
// here so existing callers (`extensions::missions::Mission`, etc.)
// keep working without churning every import site.
pub use crate::engine::mission::record::{
    Mission, MissionPermission, MissionRunEntry, MISSION_AGENT_ID,
};
pub use crate::engine::mission::registry::MissionRegistry;

pub use cron::{parse_cron, validate_cron};
pub use draft::MissionDraft;
use parser::{mission_to_md, name_to_filename, parse_mission_md};
use skill_missions::UserChoice;

/// A change refused because the mission ships with a skill: its file is the
/// skill's. Only on/off and the schedule belong to the user.
#[derive(Debug)]
pub struct SkillOwned {
    pub id: String,
    pub skill: String,
    pub hint: &'static str,
}

impl std::fmt::Display for SkillOwned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "'{}' ships with the {} skill — {}",
            self.id, self.skill, self.hint
        )
    }
}

impl std::error::Error for SkillOwned {}

// ---------------------------------------------------------------------------
// MissionLoader — the user's missions at ~/.linggen/missions/, plus the
// missions installed skills ship in `<skill>/missions/`
// ---------------------------------------------------------------------------

pub struct MissionLoader {
    dir: PathBuf,
    skills_dir: PathBuf,
    cache: std::sync::Mutex<Vec<Mission>>,
}

impl MissionLoader {
    pub fn new() -> Self {
        Self::with_dirs(
            crate::paths::global_missions_dir(),
            crate::paths::global_skills_dir(),
        )
    }

    #[cfg(test)]
    pub fn with_dir(dir: PathBuf) -> Self {
        let skills_dir = dir.join("no-skills");
        Self::with_dirs(dir, skills_dir)
    }

    fn with_dirs(dir: PathBuf, skills_dir: PathBuf) -> Self {
        let store = Self {
            dir,
            skills_dir,
            cache: std::sync::Mutex::new(Vec::new()),
        };
        store.reload();
        store
    }

    pub fn reload(&self) {
        let missions = self.scan_disk().unwrap_or_default();
        *self.cache.lock_ok() = missions;
    }

    /// Re-parse one mission's `mission.md` from disk and update the cache
    /// entry for it. Returns the fresh `Mission`, or `None` when the file
    /// is gone / corrupt / unreadable. Called at the top of each scheduler
    /// dispatch so an in-flight edit takes effect on the next run without
    /// requiring a daemon restart.
    pub fn reload_one(&self, id: &str) -> Option<Mission> {
        let mission_file = self.mission_path(id);
        if !mission_file.exists() {
            // Drop a stale cache entry if the dir was deleted out from under us.
            let mut cache = self.cache.lock_ok();
            cache.retain(|m| m.id != id);
            return None;
        }
        let content = fs::read_to_string(&mission_file).ok()?;
        let parsed = match self.parse(id, &content) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("reload_one: parse failed for '{}': {}", id, e);
                return None;
            }
        };
        let mut cache = self.cache.lock_ok();
        if let Some(slot) = cache.iter_mut().find(|m| m.id == id) {
            *slot = parsed.clone();
        } else {
            cache.push(parsed.clone());
        }
        Some(parsed)
    }

    fn ensure_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        Ok(())
    }

    /// The folder holding `mission.md` — inside the skill for a skill
    /// mission. `$MISSION_DIR` in a body resolves here.
    pub fn mission_dir(&self, id: &str) -> PathBuf {
        match skill_missions::split_id(id) {
            Some((skill, name)) => skill_missions::definition_dir(&self.skills_dir, skill, name),
            None => self.dir.join(id),
        }
    }

    /// Where the user's side of a mission lives: run history, and a skill
    /// mission's on/off + schedule.
    fn state_dir(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    fn mission_path(&self, id: &str) -> PathBuf {
        self.mission_dir(id).join("mission.md")
    }

    fn runs_path(&self, id: &str) -> PathBuf {
        self.state_dir(id).join("runs.jsonl")
    }

    /// Parse `mission.md` for `id`. A skill mission is stamped with its
    /// skill and gets the user's choices over the file's defaults.
    fn parse(&self, id: &str, content: &str) -> Result<Mission> {
        let mut mission = parse_mission_md(id, content)?;
        if let Some((skill, _)) = skill_missions::split_id(id) {
            mission.skill = Some(skill.to_string());
            UserChoice::load(&self.state_dir(id)).apply(&mut mission);
        }
        Ok(mission)
    }

    /// Refuse a change to a skill mission's file. `Ok` for user missions.
    fn ensure_user_owned(&self, id: &str, hint: &'static str) -> Result<()> {
        match skill_missions::split_id(id) {
            Some((skill, _)) => Err(SkillOwned {
                id: id.to_string(),
                skill: skill.to_string(),
                hint,
            }
            .into()),
            None => Ok(()),
        }
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
        if prompt.trim().is_empty() {
            bail!("Mission requires a prompt body");
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
            catchup_hours: draft.catchup_hours.flatten(),
            cwd: draft.cwd.clone().flatten(),
            model: draft.model.clone().flatten(),
            kickoff: draft.kickoff.clone().unwrap_or_default(),
            kickoff_day: Vec::new(),
            kickoff_attended: Vec::new(),
            kickoff_stop: Vec::new(),
            kickoff_fresh: false,
            kickoff_then: Default::default(),
            allowed_tools: draft.allowed_tools.clone().unwrap_or_default(),
            permission: draft.permission.clone().flatten(),
            prompt,
            agent_id: draft
                .agent
                .clone()
                .unwrap_or_else(|| MISSION_AGENT_ID.to_string()),
            project: draft.project.clone().flatten(),
            skill: None,
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
        let mission = self.parse(mission_id, &content)?;
        Ok(Some(mission))
    }

    /// Update a mission by applying a draft. Fields left `None` are untouched.
    /// A skill mission takes only on/off and the schedule.
    pub fn update_mission(&self, mission_id: &str, draft: MissionDraft) -> Result<Mission> {
        let Some(mut mission) = self.get_mission(mission_id)? else {
            bail!("Mission '{}' not found", mission_id);
        };
        if mission.skill.is_some() {
            return self.choose_for_skill_mission(mission_id, draft);
        }

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
        if let Some(k) = draft.kickoff {
            mission.kickoff = k;
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

    /// Record the user's on/off and schedule for a skill mission.
    fn choose_for_skill_mission(&self, id: &str, draft: MissionDraft) -> Result<Mission> {
        if draft.touches_definition() {
            self.ensure_user_owned(id, "only on/off and the schedule can change here")?;
        }
        let state_dir = self.state_dir(id);
        let mut choice = UserChoice::load(&state_dir);
        if let Some(schedule) = draft.schedule {
            validate_cron(&schedule)?;
            choice.schedule = Some(schedule);
        }
        if let Some(enabled) = draft.enabled {
            choice.enabled = Some(enabled);
        }
        choice.save(&state_dir)?;
        self.reload();
        self.get_mission(id)?
            .ok_or_else(|| anyhow::anyhow!("Mission '{}' not found", id))
    }

    /// Drop what the user kept for a removed skill's missions — choices and
    /// run history — so a reinstall starts from the skill's defaults. Past
    /// run sessions stay in the session store.
    pub fn forget_skill(&self, skill: &str) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().to_string();
            if skill_missions::split_id(&id).is_some_and(|(s, _)| s == skill) {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
        self.reload();
    }

    pub fn delete_mission(&self, mission_id: &str) -> Result<()> {
        self.ensure_user_owned(mission_id, "remove the skill to remove its mission")?;
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
        self.ensure_user_owned(mission_id, "change its mission.md in the skill")?;
        let mission = parse_mission_md(mission_id, content)?;
        fs::create_dir_all(self.mission_dir(mission_id))?;
        fs::write(self.mission_path(mission_id), content)?;
        self.reload();
        Ok(mission)
    }

    pub fn list_all_missions(&self) -> Result<Vec<Mission>> {
        Ok(self.cache.lock_ok().clone())
    }

    pub fn list_enabled_missions(&self) -> Result<Vec<Mission>> {
        Ok(self
            .cache
            .lock_ok()
            .iter()
            .filter(|m| m.enabled)
            .cloned()
            .collect())
    }

    fn scan_disk(&self) -> Result<Vec<Mission>> {
        let mut ids = self.user_mission_ids();
        ids.extend(skill_missions::discover(&self.skills_dir));
        let mut missions: Vec<Mission> = ids.iter().filter_map(|id| self.read_one(id)).collect();
        missions.sort_by_key(|m| std::cmp::Reverse(m.created_at));
        Ok(missions)
    }

    /// Folders under `~/.linggen/missions/` holding a `mission.md`. A skill
    /// mission's state folder has none, and a skill-shaped id is never
    /// read from here.
    fn user_mission_ids(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter(|e| e.path().join("mission.md").is_file())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|id| skill_missions::split_id(id).is_none())
            .collect()
    }

    fn read_one(&self, id: &str) -> Option<Mission> {
        let content = fs::read_to_string(self.mission_path(id)).ok()?;
        match self.parse(id, &content) {
            Ok(m) => Some(m),
            Err(e) => {
                tracing::warn!("Skipping corrupt mission {}: {}", id, e);
                None
            }
        }
    }

    pub fn append_mission_run(&self, mission_id: &str, entry: &MissionRunEntry) -> Result<()> {
        fs::create_dir_all(self.state_dir(mission_id))?;
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

    pub fn update_mission_run_status(
        &self,
        mission_id: &str,
        run_id: &str,
        status: &str,
    ) -> Result<()> {
        self.finish_mission_run(mission_id, run_id, status, None)
    }

    /// Rewrite the run entry matching `run_id` with its status and, when
    /// given, what the run spent — keeping file order and unknown lines
    /// intact. Missing file or run_id is a no-op — finalizing a run whose
    /// record was healed or removed in the meantime must not error the
    /// dispatch path.
    pub fn finish_mission_run(
        &self,
        mission_id: &str,
        run_id: &str,
        status: &str,
        usage: Option<RunUsage>,
    ) -> Result<()> {
        let path = self.runs_path(mission_id);
        if !path.exists() {
            return Ok(());
        }
        let content = fs::read_to_string(&path)?;
        let mut out = String::new();
        let mut changed = false;
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<MissionRunEntry>(line) {
                Ok(mut e) if e.run_id == run_id => {
                    if e.status != status {
                        e.status = status.to_string();
                        changed = true;
                    }
                    if usage.is_some() && e.usage != usage {
                        e.usage = usage.clone();
                        changed = true;
                    }
                    out.push_str(&serde_json::to_string(&e)?);
                    out.push('\n');
                }
                _ => {
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        if changed {
            fs::write(&path, out)?;
        }
        Ok(())
    }

    /// Startup reconciliation: a fresh daemon has no live runs, so any
    /// entry still `running` belongs to a dead process (hang, crash,
    /// restart). Flip them to `interrupted` so run history shows the
    /// truth and the catch-up sweep treats the slot as still missed.
    pub fn mark_running_runs_interrupted(&self) {
        let Ok(dirs) = fs::read_dir(&self.dir) else {
            return;
        };
        for entry in dirs.flatten() {
            let mission_id = entry.file_name().to_string_lossy().to_string();
            let Ok(runs) = self.list_mission_runs(&mission_id) else {
                continue;
            };
            for run in runs.iter().filter(|r| r.status == "running") {
                match self.update_mission_run_status(&mission_id, &run.run_id, "interrupted") {
                    Ok(()) => tracing::info!(
                        "mission '{}': run {} from a previous process marked interrupted",
                        mission_id,
                        run.run_id
                    ),
                    Err(e) => tracing::warn!(
                        "mission '{}': failed to heal run {}: {}",
                        mission_id,
                        run.run_id,
                        e
                    ),
                }
            }
        }
    }
}

impl MissionRegistry for MissionLoader {
    fn get_mission(&self, mission_id: &str) -> Result<Option<Mission>> {
        MissionLoader::get_mission(self, mission_id)
    }

    fn reload_one(&self, mission_id: &str) -> Option<Mission> {
        MissionLoader::reload_one(self, mission_id)
    }

    fn mission_dir(&self, mission_id: &str) -> PathBuf {
        MissionLoader::mission_dir(self, mission_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dream_starts_each_turn_fresh_and_finishes_in_stages() {
        let dream =
            parse_mission_md("dream", include_str!("../../../missions/dream/mission.md")).unwrap();
        assert!(dream.kickoff_fresh);
        assert_eq!(dream.kickoff_stop, ["DONE", "STALLED"]);
        let finish = &dream.kickoff_then["CLEAR"];
        assert_eq!(finish.len(), 3);
        assert!(finish[2].ends_with("DONE."), "{}", finish[2]);
        assert!(dream
            .kickoff
            .iter()
            .all(|item| !item.contains("memory_chains")));
        let md = mission_to_md(&dream);
        assert!(md.contains("kickoff-fresh: true"), "{md}");
        let again = parse_mission_md("dream", &md).unwrap();
        assert!(again.kickoff_fresh);
        assert_eq!(again.kickoff_then, dream.kickoff_then);
        let plain =
            parse_mission_md("plain", "---\nschedule: \"0 3 * * *\"\n---\n\nBody\n").unwrap();
        assert!(!plain.kickoff_fresh);
        assert!(!mission_to_md(&plain).contains("kickoff-fresh"));
    }

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
        assert_eq!(
            loaded.prompt,
            "Clean up old files\n\nRemove build artifacts."
        );
        assert_eq!(loaded.model, Some("gpt-4".to_string()));
        assert_eq!(loaded.cwd, Some("/tmp/proj".to_string()));
        assert_eq!(
            loaded.allowed_tools,
            vec!["Read".to_string(), "Bash".to_string()]
        );
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
        let plain_md = std::fs::read_to_string(store.mission_path(&plain.id)).unwrap();
        assert!(
            !plain_md.contains("agent:"),
            "default agent must not be serialized"
        );
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
            usage: None,
        };
        let entry2 = MissionRunEntry {
            run_id: "run-2".into(),
            session_id: None,
            triggered_at: 2000,
            status: "skipped".into(),
            skipped: true,
            usage: None,
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
    fn test_run_status_update_and_startup_heal() {
        let (store, _dir) = temp_store();
        let m = store
            .create_mission(draft_min("Test", "0 * * * *", "Test"))
            .unwrap();

        let run = |id: &str, status: &str| MissionRunEntry {
            run_id: id.into(),
            session_id: None,
            triggered_at: 1000,
            status: status.into(),
            skipped: false,
            usage: None,
        };
        store
            .append_mission_run(&m.id, &run("run-1", "running"))
            .unwrap();
        store
            .append_mission_run(&m.id, &run("run-2", "running"))
            .unwrap();

        // Normal finalize: running → completed with what it spent, other
        // rows untouched.
        let spent = RunUsage {
            calls: 3,
            prompt: 21_000,
            cached: 9_000,
            output: 700,
            models: vec!["flash".into()],
            ..Default::default()
        };
        store
            .finish_mission_run(&m.id, "run-1", "completed", Some(spent.clone()))
            .unwrap();
        let by_id = |runs: &[MissionRunEntry], id: &str| {
            runs.iter().find(|r| r.run_id == id).unwrap().status.clone()
        };
        let runs = store.list_mission_runs(&m.id).unwrap();
        assert_eq!(by_id(&runs, "run-1"), "completed");
        assert_eq!(by_id(&runs, "run-2"), "running");
        let usage_of = |id: &str| runs.iter().find(|r| r.run_id == id).unwrap().usage.clone();
        assert_eq!(usage_of("run-1"), Some(spent));
        assert_eq!(usage_of("run-2"), None);

        // Unknown run id / missing file: no-op, not an error.
        store
            .update_mission_run_status(&m.id, "run-x", "failed")
            .unwrap();
        store
            .update_mission_run_status("no-such-mission", "run-1", "failed")
            .unwrap();

        // Startup heal flips only rows still `running`.
        store.mark_running_runs_interrupted();
        let runs = store.list_mission_runs(&m.id).unwrap();
        assert_eq!(by_id(&runs, "run-1"), "completed");
        assert_eq!(by_id(&runs, "run-2"), "interrupted");
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
        // Legacy script mode used to set entry + drop body. The entry
        // field is now removed entirely; legacy files load but the
        // entry command is no longer honored, and the body still drops
        // to match the prior "script mode = no agent" intent.
        let content = "---\n\
            schedule: 0 9 * * *\n\
            enabled: true\n\
            mode: script\n\
            entry: echo hi\n\
            ---\n\
            \n\
            Some ignored body.\n";
        let m = parse_mission_md("s", content).unwrap();
        assert_eq!(m.prompt, "");
        assert!(m.kickoff.is_empty());
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
        let m1 = store
            .create_mission(draft_min("Test", "0 * * * *", "First"))
            .unwrap();
        assert_eq!(m1.id, "test");
        let m2 = store
            .create_mission(draft_min("Test", "0 * * * *", "Second"))
            .unwrap();
        assert_eq!(m2.id, "test-2");
    }

    #[test]
    fn test_builtin_dream_mission_parses() {
        // Guard against YAML regressions in the bundled dream mission.
        // If this fails the daemon will silently skip dream at startup —
        // exactly the symptom we hit when kickoff strings with embedded
        // JSON were left as plain scalars.
        let dream_md = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/missions/dream/mission.md"
        ))
        .expect("missions/dream/mission.md should exist");
        let mission = parse_mission_md("dream", &dream_md)
            .expect("dream mission.md must parse — daemon scan_disk skips invalid files silently");
        assert!(
            !mission.kickoff.is_empty(),
            "dream mission must have a kickoff list (got empty)"
        );
        assert!(mission.enabled, "dream must be enabled by default");
        assert!(!mission.prompt.is_empty(), "dream must have a body");
    }

    fn skill_store() -> (MissionLoader, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let skill_mission = dir.path().join("skills/cfo/missions/reports");
        std::fs::create_dir_all(&skill_mission).unwrap();
        std::fs::write(
            skill_mission.join("mission.md"),
            "---\nname: Reports\nschedule: \"0 9 * * 1-5\"\nenabled: false\n---\nCheck reports.\n",
        )
        .unwrap();
        let store =
            MissionLoader::with_dirs(dir.path().join("missions"), dir.path().join("skills"));
        (store, dir)
    }

    #[test]
    fn a_skill_mission_is_found_in_its_skill_and_keeps_state_with_the_user() {
        let (store, dir) = skill_store();
        let missions = store.list_all_missions().unwrap();
        assert_eq!(missions.len(), 1);
        let m = &missions[0];
        assert_eq!(m.id, "cfo:reports");
        assert_eq!(m.skill.as_deref(), Some("cfo"));
        assert!(!m.enabled);
        assert_eq!(
            store.mission_dir(&m.id),
            dir.path().join("skills/cfo/missions/reports")
        );

        let run = MissionRunEntry {
            run_id: "r1".into(),
            session_id: None,
            triggered_at: 1,
            status: "completed".into(),
            skipped: false,
            usage: None,
        };
        store.append_mission_run(&m.id, &run).unwrap();
        assert!(dir.path().join("missions/cfo:reports/runs.jsonl").exists());
        assert!(!dir
            .path()
            .join("skills/cfo/missions/reports/runs.jsonl")
            .exists());
    }

    #[test]
    fn the_users_choice_outlives_a_skill_update() {
        let (store, dir) = skill_store();
        let on = store
            .update_mission(
                "cfo:reports",
                MissionDraft {
                    enabled: Some(true),
                    schedule: Some("0 18 * * 1-5".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(on.enabled);
        assert_eq!(on.schedule, "0 18 * * 1-5");

        // The skill ships a new version of the file.
        std::fs::write(
            dir.path().join("skills/cfo/missions/reports/mission.md"),
            "---\nschedule: \"0 9 * * *\"\nenabled: false\n---\nNew body.\n",
        )
        .unwrap();
        store.reload();
        let m = store.get_mission("cfo:reports").unwrap().unwrap();
        assert!(m.enabled);
        assert_eq!(m.schedule, "0 18 * * 1-5");
        assert_eq!(m.prompt, "New body.");
    }

    #[test]
    fn a_skill_missions_file_is_the_skills() {
        let (store, _dir) = skill_store();
        let owned = |e: anyhow::Error| e.downcast_ref::<SkillOwned>().is_some();
        let body_edit = MissionDraft {
            prompt: Some("mine".into()),
            ..Default::default()
        };
        assert!(owned(
            store.update_mission("cfo:reports", body_edit).unwrap_err()
        ));
        assert!(owned(store.delete_mission("cfo:reports").unwrap_err()));
        assert!(owned(
            store
                .write_mission_raw("cfo:reports", "---\nschedule: \"0 * * * *\"\n---\nx\n")
                .unwrap_err()
        ));
        assert_eq!(store.list_all_missions().unwrap().len(), 1);
    }

    #[test]
    fn forgetting_a_skill_drops_its_missions_state() {
        let (store, dir) = skill_store();
        store
            .update_mission(
                "cfo:reports",
                MissionDraft {
                    enabled: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
        std::fs::remove_dir_all(dir.path().join("skills/cfo")).unwrap();
        store.forget_skill("cfo");
        assert!(!dir.path().join("missions/cfo:reports").exists());
        assert!(store.list_all_missions().unwrap().is_empty());
    }

    #[test]
    fn test_create_requires_prompt() {
        let (store, _dir) = temp_store();
        let err = store.create_mission(MissionDraft {
            name: Some("empty".into()),
            schedule: Some("0 * * * *".into()),
            ..Default::default()
        });
        assert!(err.is_err());
    }

    #[test]
    fn test_update_invalid_cron_rejected() {
        let (store, _dir) = temp_store();
        let m = store
            .create_mission(draft_min("Test", "0 * * * *", "Test"))
            .unwrap();
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
        let m = store
            .create_mission(draft_min("Test Dir", "0 * * * *", "Hello"))
            .unwrap();
        assert!(root.join("test-dir").is_dir());
        assert!(root.join("test-dir").join("mission.md").exists());

        let entry = MissionRunEntry {
            run_id: "r1".into(),
            session_id: None,
            triggered_at: 1000,
            status: "completed".into(),
            skipped: false,
            usage: None,
        };
        store.append_mission_run(&m.id, &entry).unwrap();
        assert!(root.join("test-dir").join("runs.jsonl").exists());

        store.delete_mission(&m.id).unwrap();
        assert!(!root.join("test-dir").exists());
    }
}
