//! Missions a skill ships in its own `missions/<name>/mission.md`.
//!
//! The skill owns the file: a skill update updates the mission. The user
//! owns on/off and the schedule, kept in `~/.linggen/missions/<id>/user.json`
//! beside the run history, so an update never undoes a choice. See
//! `doc/mission-spec.md`, "Skill missions".

use crate::engine::mission::record::Mission;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Separates the skill from the mission name in a skill mission's id
/// (`cfo:reports`). A user mission's id is a slug and never contains it.
const SEP: char = ':';

pub(super) fn id_for(skill: &str, name: &str) -> String {
    format!("{skill}{SEP}{name}")
}

/// `(skill, name)` when `id` names a skill mission.
pub(super) fn split_id(id: &str) -> Option<(&str, &str)> {
    let (skill, name) = id.split_once(SEP)?;
    (is_segment(skill) && is_segment(name)).then_some((skill, name))
}

/// One path segment — never a way out of the skills dir.
fn is_segment(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && !s.contains(['/', '\\', SEP])
}

/// The folder holding a skill mission's `mission.md`.
pub(super) fn definition_dir(skills_dir: &Path, skill: &str, name: &str) -> PathBuf {
    skills_dir.join(skill).join("missions").join(name)
}

/// Ids of every `<skill>/missions/<name>/mission.md` under `skills_dir`.
pub(super) fn discover(skills_dir: &Path) -> Vec<String> {
    let Ok(skills) = fs::read_dir(skills_dir) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for skill in skills.flatten() {
        let skill_name = skill.file_name().to_string_lossy().to_string();
        let Ok(missions) = fs::read_dir(skill.path().join("missions")) else {
            continue;
        };
        for mission in missions.flatten() {
            let name = mission.file_name().to_string_lossy().to_string();
            if !mission.path().join("mission.md").is_file() {
                continue;
            }
            if is_segment(&skill_name) && is_segment(&name) {
                ids.push(id_for(&skill_name, &name));
            }
        }
    }
    ids
}

/// What the user chose for a skill mission. Absent fields keep the file's
/// default.
#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub(super) struct UserChoice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
}

impl UserChoice {
    fn path(state_dir: &Path) -> PathBuf {
        state_dir.join("user.json")
    }

    pub(super) fn load(state_dir: &Path) -> Self {
        fs::read_to_string(Self::path(state_dir))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub(super) fn save(&self, state_dir: &Path) -> Result<()> {
        fs::create_dir_all(state_dir)?;
        fs::write(Self::path(state_dir), serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub(super) fn apply(&self, mission: &mut Mission) {
        if let Some(enabled) = self.enabled {
            mission.enabled = enabled;
        }
        if let Some(ref schedule) = self.schedule {
            mission.schedule = schedule.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_split_back_into_skill_and_name() {
        assert_eq!(split_id("cfo:reports"), Some(("cfo", "reports")));
        assert_eq!(split_id("dream"), None);
        assert_eq!(split_id("cfo:"), None);
        assert_eq!(split_id("..:reports"), None);
        assert_eq!(split_id("cfo:a/b"), None);
    }

    #[test]
    fn discovers_only_folders_with_a_mission_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("cfo/missions/reports")).unwrap();
        fs::write(root.join("cfo/missions/reports/mission.md"), "x").unwrap();
        fs::create_dir_all(root.join("cfo/missions/empty")).unwrap();
        fs::create_dir_all(root.join("dj/scripts")).unwrap();
        assert_eq!(discover(root), vec!["cfo:reports".to_string()]);
    }

    #[test]
    fn a_choice_round_trips_and_overrides_only_what_it_holds() {
        let dir = tempfile::tempdir().unwrap();
        let choice = UserChoice {
            enabled: Some(true),
            schedule: None,
        };
        choice.save(dir.path()).unwrap();
        assert_eq!(UserChoice::load(dir.path()), choice);
        assert_eq!(
            UserChoice::load(&dir.path().join("missing")),
            UserChoice::default()
        );
    }
}
