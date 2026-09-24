//! Keeping a skill's declared save in step with the account's copy on
//! linggen.dev. The ledger (`~/.linggen/sync/cloud-<skill>.json`) remembers
//! the version this machine last agreed on and the save's fingerprint then,
//! so a sync can tell "changed here" from "another device moved on".
//!
//! The account's copy wins every disagreement — but a pull never destroys a
//! change made here: each file it replaces is first copied beside itself
//! (`<file>.conflict-<unix secs>`). Pulls happen only at a turn's edges and
//! when the page asks; mid-turn, after a model call, the save is only pushed.
//! Every read and write of the save's files holds the save's lockfile
//! (`lock.rs`), the one the skill's own scripts take.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::cloud::{save_get, save_put, valid_cloud_name, CloudSave, PutOutcome};
use crate::engine::skill::{Skill, SkillSource};

mod files;
pub mod lock;

use files::{plain, Files, Layout};

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

/// What a sync must do. `local`: the save's fingerprint (None = no files);
/// `ledger`: what this machine last agreed on; `cloud`: the account's version.
pub fn decide(local: Option<u64>, ledger: Option<Ledger>, cloud: u64) -> Action {
    if cloud == 0 {
        return if local.is_some() {
            Action::Push { version: 0 }
        } else {
            Action::Nothing
        };
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

/// A skill's save: its name on the site and its files here.
#[derive(Debug, Clone)]
pub struct SaveTarget {
    pub skill: String,
    layout: Layout,
}

impl SaveTarget {
    /// The save's first path — where its lock sits.
    pub fn file(&self) -> PathBuf {
        self.layout.first()
    }
}

/// What one sync did.
#[derive(Debug, Clone, Serialize)]
pub struct Synced {
    #[serde(flatten)]
    pub action: Action,
    pub version: u64,
    /// Copies of changes made here that a pull replaced.
    pub conflicts: Vec<PathBuf>,
}

impl Synced {
    fn plain(action: Action, version: u64) -> Self {
        Synced {
            action,
            version,
            conflicts: Vec::new(),
        }
    }

    pub fn pulled(&self) -> bool {
        self.action == Action::Pull
    }
}

/// The save an installed skill declares, if every path stays inside its
/// directory and its name can stand in a URL.
pub fn target(skill: &Skill) -> Option<SaveTarget> {
    let paths = skill.cloud.as_ref()?.save.as_ref()?;
    if !matches!(skill.source, SkillSource::Global) || !valid_cloud_name(&skill.name) {
        tracing::warn!(
            "skill '{}': only an installed skill named [a-z0-9-] keeps a cloud save",
            skill.name
        );
        return None;
    }
    let entries: Vec<String> = paths
        .paths()
        .iter()
        .map(|p| p.trim_end_matches('/').to_string())
        .collect();
    if entries.is_empty() || !entries.iter().all(|e| plain(e)) {
        tracing::warn!(
            "skill '{}': cloud.save must stay inside its directory",
            skill.name
        );
        return None;
    }
    Some(SaveTarget {
        skill: skill.name.clone(),
        layout: Layout {
            root: skill.skill_dir.clone()?,
            entries,
            one: paths.is_one(),
            skip: skill.cloud.as_ref()?.skip.clone(),
        },
    })
}

/// The save's files and their fingerprint, read under the save's lock.
async fn read_locked(t: &SaveTarget) -> Result<(Files, u64)> {
    let _held = lock::acquire(&t.file()).await?;
    let files = t.layout.read();
    let hash = t.layout.fingerprint(&files);
    Ok((files, hash))
}

/// Bring the files and the account's copy into step.
pub async fn sync(t: &SaveTarget) -> Result<Synced> {
    let _one = SYNC_LOCK.lock().await;
    let cloud = save_get(&t.skill).await?;
    let (files, hash) = read_locked(t).await?;
    let local = (!files.is_empty()).then_some(hash);
    let action = decide(local, read_ledger(&t.skill), cloud.version);
    match action {
        Action::Nothing => Ok(Synced::plain(action, cloud.version)),
        Action::Pull => pull(t, &cloud).await,
        Action::Push { version } => match push_files(t, version, &files, hash).await? {
            Some(v) => Ok(Synced::plain(action, v)),
            None => {
                tracing::info!(
                    "save '{}' moved on elsewhere; taking the account's copy",
                    t.skill
                );
                pull(t, &save_get(&t.skill).await?).await
            }
        },
    }
}

/// Push a change made here, never pull: the path taken mid-turn, right after
/// a model call, while the tool call that follows may be writing the save. A
/// save that moved on elsewhere is left for the turn's edge to reconcile.
pub async fn push(t: &SaveTarget) -> Result<Action> {
    let _one = SYNC_LOCK.lock().await;
    let Some(agreed) = read_ledger(&t.skill) else {
        return Ok(Action::Nothing); // never synced here: the turn's edge pulls first
    };
    let (files, hash) = read_locked(t).await?;
    if files.is_empty() || hash == agreed.hash {
        return Ok(Action::Nothing);
    }
    let action = Action::Push {
        version: agreed.version,
    };
    if push_files(t, agreed.version, &files, hash).await?.is_none() {
        tracing::info!(
            "save '{}' moved on elsewhere; the turn's edge will reconcile",
            t.skill
        );
        return Ok(Action::Nothing);
    }
    Ok(action)
}

/// Write the files as the version after `version`; `None` when stale.
async fn push_files(t: &SaveTarget, version: u64, files: &Files, hash: u64) -> Result<Option<u64>> {
    match save_put(&t.skill, version, &t.layout.encode(files)).await? {
        PutOutcome::Saved(v) => {
            write_ledger(&t.skill, Ledger { version: v, hash })?;
            Ok(Some(v))
        }
        PutOutcome::Stale => Ok(None),
    }
}

/// Take the account's copy. Under the lock, the files are read again: a
/// change made here since the ledger's agreement is kept as conflict copies
/// before it is replaced.
async fn pull(t: &SaveTarget, cloud: &CloudSave) -> Result<Synced> {
    let incoming = t
        .layout
        .decode(cloud.body.as_ref().context("the account's save is empty")?)?;
    let _held = lock::acquire(&t.file()).await?;
    let current = t.layout.read();
    let changed_here = !current.is_empty()
        && read_ledger(&t.skill).map(|l| l.hash) != Some(t.layout.fingerprint(&current));
    let conflicts = if changed_here {
        t.layout.keep_conflicts(&current, &incoming)?
    } else {
        Vec::new()
    };
    if !conflicts.is_empty() {
        tracing::warn!(
            "save '{}': a change made here was replaced by the account's copy; kept as {:?}",
            t.skill,
            conflicts
        );
    }
    t.layout.write(&current, &incoming)?;
    write_ledger(
        &t.skill,
        Ledger {
            version: cloud.version,
            hash: t.layout.fingerprint(&incoming),
        },
    )?;
    Ok(Synced {
        action: Action::Pull,
        version: cloud.version,
        conflicts,
    })
}

fn ledger_path(skill: &str) -> PathBuf {
    crate::paths::linggen_home()
        .join("sync")
        .join(format!("cloud-{skill}.json"))
}

fn read_ledger(skill: &str) -> Option<Ledger> {
    let text = std::fs::read_to_string(ledger_path(skill)).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_ledger(skill: &str, ledger: Ledger) -> Result<()> {
    files::write_atomic(
        &ledger_path(skill),
        serde_json::to_string(&ledger)?.as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: Ledger = Ledger {
        version: 3,
        hash: 7,
    };

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
        assert_eq!(t.file(), PathBuf::from("/s/g/data/state.json"));
        assert_eq!(skill.cloud.as_ref().unwrap().meter.as_deref(), Some("g"));
        for bad in ["../x.json", "/etc/passwd", "data/../../x"] {
            skill.cloud = Some(crate::engine::skill::CloudConfig {
                save: Some(bad.into()),
                meter: None,
                skip: Vec::new(),
            });
            assert!(target(&skill).is_none(), "{bad}");
        }
    }
}
