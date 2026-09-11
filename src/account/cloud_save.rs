//! Keeping a skill's declared save in step with the account's copy on
//! linggen.dev. The ledger (`~/.linggen/sync/cloud-<skill>.json`) remembers
//! the version this machine last agreed on and the file's fingerprint then,
//! so a sync can tell "changed here" from "another device moved on".
//!
//! The account's copy wins every disagreement: a file changed here while
//! another device wrote is replaced by theirs.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};

use super::cloud::{save_get, save_put, CloudSave, PutOutcome};
use crate::engine::skill::Skill;

/// One sync at a time: a turn's push and the page's pull never interleave.
static SYNC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    pub version: u64,
    pub hash: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum Action {
    Nothing,
    Pull,
    Push { version: u64 },
}

/// What a sync must do. `local`: the file's fingerprint (None = no file);
/// `ledger`: what this machine last agreed on; `cloud`: the account's version.
pub fn decide(local: Option<u64>, ledger: Option<Ledger>, cloud: u64) -> Action {
    if cloud == 0 {
        return if local.is_some() { Action::Push { version: 0 } } else { Action::Nothing };
    }
    let (Some(hash), Some(agreed)) = (local, ledger) else {
        // No file here, or this machine never synced: the account's save is the game.
        return Action::Pull;
    };
    if cloud != agreed.version {
        return Action::Pull;
    }
    if hash != agreed.hash {
        return Action::Push { version: cloud };
    }
    Action::Nothing
}

/// A skill's save: its name on the site and its file here.
#[derive(Debug, Clone)]
pub struct SaveTarget {
    pub skill: String,
    pub file: PathBuf,
}

/// The save a skill declares, if it names a plain path inside its directory.
pub fn target(skill: &Skill) -> Option<SaveTarget> {
    let rel = skill.cloud.as_ref()?.save.as_deref()?;
    let rel = Path::new(rel);
    let inside = rel.components().all(|c| matches!(c, Component::Normal(_)));
    if !inside {
        tracing::warn!("skill '{}': cloud.save must stay inside its directory", skill.name);
        return None;
    }
    Some(SaveTarget {
        skill: skill.name.clone(),
        file: skill.skill_dir.as_ref()?.join(rel),
    })
}

/// Bring the file and the account's copy into step. Returns what was done
/// and the version they now share.
pub async fn sync(t: &SaveTarget) -> Result<(Action, u64)> {
    let _one = SYNC_LOCK.lock().await;
    let cloud = save_get(&t.skill).await?;
    let local = read_local(&t.file);
    let action = decide(local.as_ref().map(|(_, h)| *h), read_ledger(&t.skill), cloud.version);
    match action {
        Action::Nothing => Ok((action, cloud.version)),
        Action::Pull => pull(t, &cloud).map(|v| (action, v)),
        Action::Push { version } => {
            let (text, hash) = local.context("nothing to push")?;
            match save_put(&t.skill, version, &text).await? {
                PutOutcome::Saved(v) => {
                    write_ledger(&t.skill, Ledger { version: v, hash })?;
                    Ok((action, v))
                }
                PutOutcome::Stale => {
                    tracing::info!("save '{}' moved on elsewhere; taking the account's copy", t.skill);
                    let fresh = save_get(&t.skill).await?;
                    pull(t, &fresh).map(|v| (Action::Pull, v))
                }
            }
        }
    }
}

fn pull(t: &SaveTarget, cloud: &CloudSave) -> Result<u64> {
    let text = cloud.text().context("the account's save is empty")?;
    write_atomic(&t.file, &text)?;
    write_ledger(&t.skill, Ledger { version: cloud.version, hash: fingerprint(&text) })?;
    Ok(cloud.version)
}

/// Change detection only — never a security boundary.
fn fingerprint(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

fn read_local(file: &Path) -> Option<(String, u64)> {
    let text = std::fs::read_to_string(file).ok()?;
    let hash = fingerprint(&text);
    Some((text, hash))
}

fn ledger_path(skill: &str) -> PathBuf {
    crate::paths::linggen_home().join("sync").join(format!("cloud-{skill}.json"))
}

fn read_ledger(skill: &str) -> Option<Ledger> {
    let text = std::fs::read_to_string(ledger_path(skill)).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_ledger(skill: &str, ledger: Ledger) -> Result<()> {
    write_atomic(&ledger_path(skill), &serde_json::to_string(&ledger)?)
}

fn write_atomic(file: &Path, text: &str) -> Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    let tmp = file.with_extension(format!("cloud-{}.tmp", std::process::id()));
    std::fs::write(&tmp, text).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, file).with_context(|| format!("replace {}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: Ledger = Ledger { version: 3, hash: 7 };

    #[test]
    fn nothing_saved_anywhere_does_nothing_and_a_first_file_creates() {
        assert_eq!(decide(None, None, 0), Action::Nothing);
        assert_eq!(decide(Some(7), None, 0), Action::Push { version: 0 });
    }

    #[test]
    fn a_machine_that_never_synced_takes_the_accounts_save() {
        assert_eq!(decide(Some(9), None, 3), Action::Pull);
        assert_eq!(decide(None, Some(L), 3), Action::Pull);
    }

    #[test]
    fn a_change_here_is_pushed_from_the_agreed_version() {
        assert_eq!(decide(Some(8), Some(L), 3), Action::Push { version: 3 });
        assert_eq!(decide(Some(7), Some(L), 3), Action::Nothing);
    }

    #[test]
    fn another_device_moving_on_wins_even_over_a_change_here() {
        assert_eq!(decide(Some(7), Some(L), 4), Action::Pull);
        assert_eq!(decide(Some(8), Some(L), 4), Action::Pull);
    }

    #[test]
    fn a_save_path_must_stay_inside_the_skill() {
        let mut skill = crate::extensions::skills::parse_skill_text(
            "---\nname: g\ndescription: d\ncloud:\n  save: data/state.json\n  meter: g\n---\nbody",
            crate::engine::skill::SkillSource::Global,
        )
        .unwrap();
        skill.skill_dir = Some(PathBuf::from("/s/g"));
        let t = target(&skill).unwrap();
        assert_eq!(t.file, PathBuf::from("/s/g/data/state.json"));
        assert_eq!(skill.cloud.as_ref().unwrap().meter.as_deref(), Some("g"));
        for bad in ["../x.json", "/etc/passwd", "data/../../x"] {
            skill.cloud = Some(crate::engine::skill::CloudConfig { save: Some(bad.into()), meter: None });
            assert!(target(&skill).is_none(), "{bad}");
        }
    }
}
