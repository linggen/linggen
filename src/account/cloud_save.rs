//! Keeping a skill's declared save in step with the account's copy on
//! linggen.dev. The ledger (`~/.linggen/sync/cloud-<skill>.json`) remembers
//! the version this machine last agreed on and the save's fingerprint then,
//! so a sync can tell "changed here" from "another device moved on".
//!
//! One side moved: it is taken whole. Both moved: the copy further on wins by
//! the rule the skill declares (`cloud.further`, `further.rs`) — the same rule
//! a phone decides by — and without one the account's copy wins; a tie goes
//! to the account's. The loser is never destroyed: each file it had is kept
//! beside the save as `<file>.conflict-<unix secs>` (the account's copy too,
//! when this machine's wins) — unless it differs from the winner only in the
//! keys the skill declares as not progress (`cloud.ignore`): then both agree,
//! the ledger is settled and no copy is written. Each file keeps its newest 5
//! copies. Pulls happen only at a turn's edges and when the
//! page asks; mid-turn, after a model call, the save is only pushed. Every
//! read and write of the save's files holds the save's lockfile (`lock.rs`),
//! the one the skill's own scripts take.
//!
//! linggen.dev keeps saves for subscribers only: a 402 makes the save
//! local-only, quietly and for a while (`state.rs`) — the ledger is left as it
//! was, so a resubscribe carries on from it.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::cloud::{save_get, save_put, valid_cloud_name, CloudSave, Fetched, PutOutcome};
use crate::engine::skill::{Skill, SkillSource};

mod files;
pub mod further;
pub mod lock;
pub mod peer;
pub mod state;

use files::{plain, Files, Ignore, Layout};

/// One sync at a time: a turn's push and the page's pull never interleave.
static SYNC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    pub version: u64,
    pub hash: u64,
}

/// What a sync did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum Action {
    Nothing,
    Pull,
    Push {
        version: u64,
    },
    /// The account keeps no cloud save: nothing asked of the site.
    Local,
}

/// What a sync must do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Nothing,
    Pull,
    Push {
        version: u64,
    },
    /// Both moved since they last agreed (or never agreed): the rule decides.
    Both,
}

/// What a sync must do. `local`: the save's fingerprint (None = no files);
/// `ledger`: what this machine last agreed on; `cloud`: the account's version.
pub fn decide(local: Option<u64>, ledger: Option<Ledger>, cloud: u64) -> Step {
    if cloud == 0 {
        return if local.is_some() {
            Step::Push { version: 0 }
        } else {
            Step::Nothing
        };
    }
    let Some(hash) = local else {
        return Step::Pull; // no file here: the account's save is the game
    };
    let Some(agreed) = ledger else {
        return Step::Both; // a save here that never met the account's
    };
    match (cloud != agreed.version, hash != agreed.hash) {
        (false, false) => Step::Nothing,
        (false, true) => Step::Push { version: cloud },
        (true, false) => Step::Pull,
        (true, true) => Step::Both,
    }
}

/// A skill's save: its name on the site, its files here, and its rule.
#[derive(Debug, Clone)]
pub struct SaveTarget {
    pub skill: String,
    layout: Layout,
    further: Option<further::Rule>,
}

impl SaveTarget {
    /// The save's first path — where its lock sits.
    pub fn file(&self) -> PathBuf {
        self.layout.first()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Synced {
    #[serde(flatten)]
    pub action: Action,
    pub version: u64,
    /// Copies kept of what lost: changes made here a pull replaced, or the
    /// account's copy this machine's replaced.
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

    fn local() -> Self {
        Self::plain(Action::Local, 0)
    }

    pub fn pulled(&self) -> bool {
        self.action == Action::Pull
    }
}

/// The save an installed skill declares, if every path stays inside its
/// directory and its name can stand in a URL.
pub fn target(skill: &Skill) -> Option<SaveTarget> {
    let cloud = skill.cloud.as_ref()?;
    let paths = cloud.save.as_ref()?;
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
    let root = skill.skill_dir.clone()?;
    let further = cloud
        .further
        .as_ref()
        .and_then(|v| further::rule(&skill.name, &root, v));
    let ignore = ignore_of(&skill.name, &cloud.ignore, further.as_ref());
    Some(SaveTarget {
        skill: skill.name.clone(),
        layout: Layout {
            root,
            entries,
            one: paths.is_one(),
            skip: cloud.skip.clone(),
            ignore,
        },
        further,
    })
}

/// `cloud.ignore` read against the rule's file: without a readable `further`
/// there is no file for the keys to belong to, and the save is compared raw.
fn ignore_of(skill: &str, keys: &[String], rule: Option<&further::Rule>) -> Option<Ignore> {
    if keys.is_empty() {
        return None;
    }
    let Some(rule) = rule else {
        tracing::warn!("skill '{skill}': cloud.ignore needs a readable cloud.further; not used");
        return None;
    };
    if keys.iter().any(|k| k.is_empty()) {
        tracing::warn!("skill '{skill}': cloud.ignore holds an empty key; not used");
        return None;
    }
    Some(Ignore {
        file: rule.file().to_string(),
        keys: keys.to_vec(),
    })
}

/// The save's files and their fingerprint (progress only: `files.rs`
/// `progress_fingerprint`), read under the save's lock.
async fn read_locked(t: &SaveTarget) -> Result<(Files, u64)> {
    let _held = lock::acquire(&t.file()).await?;
    let files = t.layout.read();
    let hash = t.layout.progress_fingerprint(&files);
    Ok((files, hash))
}

/// What this machine last agreed on, its hash brought to the progress
/// fingerprint. A ledger written before `cloud.ignore` holds the raw one: it
/// proves the save unmoved when the files still hash to it that way (and is
/// then rewritten); a save that does not hash to it is taken as moved — it
/// cannot be told from a stamp alone, so it goes through the rule once.
fn agreed_ledger(t: &SaveTarget, files: &Files, progress: u64) -> Option<Ledger> {
    let l = read_ledger(&t.skill)?;
    Some(match brought_to_progress(&t.layout, l, files, progress) {
        Some(up) => {
            let _ = write_ledger(&t.skill, up);
            up
        }
        None => l,
    })
}

/// `l` with its hash as the progress fingerprint, when `l` holds the raw one
/// of exactly these files; None when it needs no change or cannot be proved.
fn brought_to_progress(layout: &Layout, l: Ledger, files: &Files, progress: u64) -> Option<Ledger> {
    (l.hash != progress
        && layout.has_ignore()
        && !files.is_empty()
        && layout.fingerprint(files) == l.hash)
        .then_some(Ledger {
            version: l.version,
            hash: progress,
        })
}

/// The ledger that settles a conflict when the account's copy holds the same
/// progress as ours (stamps aside): nothing to choose, nothing to keep.
fn settled(layout: &Layout, version: u64, hash: u64, theirs: &Files) -> Option<Ledger> {
    (layout.progress_fingerprint(theirs) == hash).then_some(Ledger { version, hash })
}

/// Bring the files and the account's copy into step.
pub async fn sync(t: &SaveTarget) -> Result<Synced> {
    let _one = SYNC_LOCK.lock().await;
    let now = crate::util::now_ts_secs();
    let (files, hash) = read_locked(t).await?;
    note_rise(t, &files, now);
    if state::backed_off(&state::read(&t.skill), now, state::who()) {
        return Ok(Synced::local());
    }
    let cloud = match save_get(&t.skill).await? {
        Fetched::Save(c) => c,
        Fetched::Refused(r) => {
            state::refused(&t.skill, &r, now);
            return Ok(Synced::local());
        }
    };
    let local = (!files.is_empty()).then_some(hash);
    let done = match decide(local, agreed_ledger(t, &files, hash), cloud.version) {
        Step::Nothing => Synced::plain(Action::Nothing, cloud.version),
        Step::Pull => pull(t, &cloud).await?,
        Step::Push { version } => push_or_pull(t, version, &files, hash).await?,
        Step::Both => both(t, &cloud, &files, hash).await?,
    };
    if done.action != Action::Local {
        state::reached(&t.skill, now);
    }
    Ok(done)
}

/// A change made here, pushed; a save that moved on meanwhile is taken.
async fn push_or_pull(t: &SaveTarget, version: u64, files: &Files, hash: u64) -> Result<Synced> {
    match push_files(t, version, files, hash).await? {
        Pushed::At(v) => Ok(Synced::plain(Action::Push { version }, v)),
        Pushed::Refused => Ok(Synced::local()),
        Pushed::Stale => {
            tracing::info!("save '{}' moved on elsewhere; deciding again", t.skill);
            match save_get(&t.skill).await? {
                Fetched::Save(c) => both(t, &c, files, hash).await,
                Fetched::Refused(r) => {
                    state::refused(&t.skill, &r, crate::util::now_ts_secs());
                    Ok(Synced::local())
                }
            }
        }
    }
}

/// Both moved: the same files agree at once; else the copy further on wins,
/// and the other is kept beside the save.
async fn both(t: &SaveTarget, cloud: &CloudSave, files: &Files, hash: u64) -> Result<Synced> {
    let body = cloud.body.as_ref().context("the account's save is empty")?;
    let theirs = t.layout.decode(body)?;
    // The same progress (stamps aside): nothing is lost either way, no copy.
    if let Some(agreed) = settled(&t.layout, cloud.version, hash, &theirs) {
        write_ledger(&t.skill, agreed)?;
        return Ok(Synced::plain(Action::Nothing, cloud.version));
    }
    let here_wins = t
        .further
        .as_ref()
        .is_some_and(|r| r.further(files, &theirs));
    if !here_wins {
        return pull(t, cloud).await;
    }
    let action = Action::Push {
        version: cloud.version,
    };
    match push_files(t, cloud.version, files, hash).await? {
        Pushed::At(v) => {
            let _held = lock::acquire(&t.file()).await?;
            let conflicts = t.layout.keep_conflicts(&theirs, files)?;
            tracing::info!(
                "save '{}': this machine's copy was further on and replaced the account's; kept as {:?}",
                t.skill,
                conflicts
            );
            Ok(Synced {
                action,
                version: v,
                conflicts,
            })
        }
        // Written again meanwhile: the next edge decides on what it is then.
        Pushed::Stale => Ok(Synced::plain(Action::Nothing, cloud.version)),
        Pushed::Refused => Ok(Synced::local()),
    }
}

/// Push a change made here, never pull: the path taken mid-turn, right after
/// a model call, while the tool call that follows may be writing the save. A
/// save that moved on elsewhere is left for the turn's edge to reconcile.
pub async fn push(t: &SaveTarget) -> Result<Action> {
    let _one = SYNC_LOCK.lock().await;
    let now = crate::util::now_ts_secs();
    if state::backed_off(&state::read(&t.skill), now, state::who()) {
        return Ok(Action::Local);
    }
    if read_ledger(&t.skill).is_none() {
        return Ok(Action::Nothing); // never synced here: the turn's edge pulls first
    }
    let (files, hash) = read_locked(t).await?;
    let Some(agreed) = agreed_ledger(t, &files, hash) else {
        return Ok(Action::Nothing);
    };
    if files.is_empty() || hash == agreed.hash {
        return Ok(Action::Nothing);
    }
    match push_files(t, agreed.version, &files, hash).await? {
        Pushed::At(_) => {
            state::reached(&t.skill, now);
            Ok(Action::Push {
                version: agreed.version,
            })
        }
        Pushed::Refused => Ok(Action::Local),
        Pushed::Stale => {
            tracing::info!(
                "save '{}' moved on elsewhere; the turn's edge will reconcile",
                t.skill
            );
            Ok(Action::Nothing)
        }
    }
}

enum Pushed {
    At(u64),
    Stale,
    Refused,
}

/// Write the files as the version after `version`.
async fn push_files(t: &SaveTarget, version: u64, files: &Files, hash: u64) -> Result<Pushed> {
    match save_put(&t.skill, version, &t.layout.encode(files)).await? {
        PutOutcome::Saved(v) => {
            write_ledger(&t.skill, Ledger { version: v, hash })?;
            Ok(Pushed::At(v))
        }
        PutOutcome::Stale => Ok(Pushed::Stale),
        PutOutcome::Refused(r) => {
            state::refused(&t.skill, &r, crate::util::now_ts_secs());
            Ok(Pushed::Refused)
        }
    }
}

/// Keep the save's standing on its rule's `rise` entries; the first move up
/// while the save is local-only is noted (`rose_local_at`), a fact for the
/// page and the agent.
fn note_rise(t: &SaveTarget, files: &Files, now: u64) {
    let Some(rule) = &t.further else { return };
    let Some(standing) = rule.standing(files) else {
        return;
    };
    let mut s = state::read(&t.skill);
    if s.seen.as_ref() == Some(&standing) {
        return;
    }
    let rose = s
        .seen
        .as_ref()
        .is_some_and(|seen| rule.rose(seen, &standing));
    if rose && s.local_only() && s.rose_local_at.is_none() {
        s.rose_local_at = Some(now);
    }
    s.seen = Some(standing);
    state::write(&t.skill, &s);
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
    let here = t.layout.progress_fingerprint(&current);
    let changed_here =
        !current.is_empty() && agreed_ledger(t, &current, here).map(|l| l.hash) != Some(here);
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
            hash: t.layout.progress_fingerprint(&incoming),
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
        assert_eq!(decide(None, None, 0), Step::Nothing);
        assert_eq!(decide(Some(7), None, 0), Step::Push { version: 0 });
    }

    #[test]
    fn no_file_here_takes_the_accounts_and_a_save_that_never_met_it_is_decided() {
        assert_eq!(decide(None, Some(L), 3), Step::Pull);
        assert_eq!(decide(None, None, 3), Step::Pull);
        assert_eq!(decide(Some(9), None, 3), Step::Both);
    }

    #[test]
    fn a_change_here_is_pushed_from_the_agreed_version() {
        assert_eq!(decide(Some(8), Some(L), 3), Step::Push { version: 3 });
        assert_eq!(decide(Some(7), Some(L), 3), Step::Nothing);
    }

    #[test]
    fn another_device_moving_on_is_taken_and_both_moving_is_decided() {
        assert_eq!(decide(Some(7), Some(L), 4), Step::Pull);
        assert_eq!(decide(Some(8), Some(L), 4), Step::Both);
    }

    #[test]
    fn a_further_rule_declared_as_a_json_flow_in_the_frontmatter_is_read() {
        let root = std::env::temp_dir().join(format!("cloud-further-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("ladder.json"),
            r#"{"tiers": [{"id": "a"}, {"id": "b"}]}"#,
        )
        .unwrap();
        // The shape skills/lingjing/SKILL.md declares: a JSON flow across lines under `further:`.
        let md = "---\nname: g\ndescription: d\ncloud:\n  save: [data/state.json]\n  skip: [art]\n  further:\n    {\"file\": \"data/state.json\", \"by\": [\n      {\"count\": \"done\"},\n      {\"rank\": \"tier\", \"order\": {\"file\": \"ladder.json\", \"list\": \"tiers\", \"key\": \"id\"}, \"rise\": true}\n    ]}\n---\nbody";
        let mut skill =
            crate::extensions::skills::parse_skill_text(md, SkillSource::Global).unwrap();
        skill.skill_dir = Some(root.clone());
        let t = target(&skill).unwrap();
        let rule = t.further.as_ref().expect("the rule is read");
        let at = |tier: &str| {
            let mut f = Files::new();
            f.insert(
                "data/state.json".into(),
                format!(r#"{{"done": [], "tier": "{tier}"}}"#).into_bytes(),
            );
            f
        };
        assert!(rule.further(&at("b"), &at("a")));
        assert!(!rule.further(&at("a"), &at("b")));
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
                further: None,
                ignore: Vec::new(),
            });
            assert!(target(&skill).is_none(), "{bad}");
        }
    }

    fn ignoring_target(extra: &str) -> SaveTarget {
        let root = std::env::temp_dir().join(format!("cloud-ignore-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let md = format!(
            "---\nname: g\ndescription: d\ncloud:\n  save: [data/state.json]\n{extra}---\nbody"
        );
        let mut skill =
            crate::extensions::skills::parse_skill_text(&md, SkillSource::Global).unwrap();
        skill.skill_dir = Some(root);
        target(&skill).unwrap()
    }

    const FURTHER: &str = "  further:\n    {\"file\": \"data/state.json\", \"by\": [{\"count\": \"done\"}, {\"time\": \"updated\"}]}\n";

    fn state(text: &str) -> Files {
        let mut f = Files::new();
        f.insert("data/state.json".into(), text.as_bytes().to_vec());
        f
    }

    #[test]
    fn cloud_ignore_is_read_against_the_rules_file_and_needs_a_rule() {
        let t = ignoring_target(&format!("{FURTHER}  ignore: [updated, stamina]\n"));
        let ig = t.layout.ignore.as_ref().expect("the keys are read");
        assert_eq!(ig.file, "data/state.json");
        assert_eq!(ig.keys, vec!["updated", "stamina"]);
        // The shape skills/lingjing/SKILL.md declares: comments after `further`, a JSON list over several lines.
        let t = ignoring_target(&format!(
            "{FURTHER}  # stamps, not moves\n  # more\n  ignore: [\"updated\", \"stamina\",\n    \"shown\", \"guided\"]\n"
        ));
        assert_eq!(
            t.layout.ignore.as_ref().unwrap().keys,
            ["updated", "stamina", "shown", "guided"]
        );
        // No readable rule: no file for the keys, so the save is compared raw.
        assert!(ignoring_target("  ignore: [updated]\n")
            .layout
            .ignore
            .is_none());
        assert!(ignoring_target(FURTHER).layout.ignore.is_none());
    }

    #[test]
    fn a_stamp_is_no_move_and_a_play_is() {
        let t = ignoring_target(&format!("{FURTHER}  ignore: [updated, stamina]\n"));
        let at = |s: &str| t.layout.progress_fingerprint(&state(s));
        let agreed = Ledger {
            version: 3,
            hash: at(r#"{"done":["a"],"updated":"t1","stamina":40}"#),
        };
        let stamped = at(r#"{"stamina":41,"done":["a"],"updated":"t2"}"#);
        assert_eq!(decide(Some(stamped), Some(agreed), 3), Step::Nothing);
        assert_eq!(decide(Some(stamped), Some(agreed), 4), Step::Pull);
        let played = at(r#"{"done":["a","b"],"updated":"t2","stamina":38}"#);
        assert_eq!(
            decide(Some(played), Some(agreed), 3),
            Step::Push { version: 3 }
        );
        assert_eq!(decide(Some(played), Some(agreed), 4), Step::Both);
    }

    #[test]
    fn both_moved_to_the_same_progress_settles_with_no_conflict() {
        let t = ignoring_target(&format!("{FURTHER}  ignore: [updated, stamina]\n"));
        let here = state(r#"{"done":["a"],"updated":"t1","stamina":10}"#);
        let theirs = state(r#"{"updated":"t9","stamina":99,"done":["a"]}"#);
        let hash = t.layout.progress_fingerprint(&here);
        assert_eq!(
            settled(&t.layout, 5, hash, &theirs),
            Some(Ledger { version: 5, hash })
        );
        assert!(t.layout.keep_conflicts(&here, &theirs).unwrap().is_empty());
        // Real progress on either side is not settled.
        let ahead = state(r#"{"done":["a","b"],"updated":"t9"}"#);
        assert_eq!(settled(&t.layout, 5, hash, &ahead), None);
        // A save with no ignore list settles only on identical bytes, as before.
        let plain = ignoring_target(FURTHER);
        let h = plain.layout.progress_fingerprint(&here);
        assert_eq!(settled(&plain.layout, 5, h, &theirs), None);
        assert!(settled(&plain.layout, 5, h, &here.clone()).is_some());
    }

    #[test]
    fn a_ledger_written_raw_is_proved_unmoved_or_taken_as_moved() {
        let t = ignoring_target(&format!("{FURTHER}  ignore: [updated, stamina]\n"));
        let was = state(r#"{"done":["a"],"updated":"t1","stamina":10}"#);
        let old = Ledger {
            version: 3,
            hash: t.layout.fingerprint(&was),
        };
        // The files still hash to the old raw hash: unmoved, and the ledger moves to the new kind.
        let progress = t.layout.progress_fingerprint(&was);
        assert_eq!(
            brought_to_progress(&t.layout, old, &was, progress),
            Some(Ledger {
                version: 3,
                hash: progress
            })
        );
        // Stamped since: the raw hash no longer proves anything; left as it is, the save reads as moved.
        let stamped = state(r#"{"done":["a"],"updated":"t2","stamina":11}"#);
        let p2 = t.layout.progress_fingerprint(&stamped);
        assert_eq!(brought_to_progress(&t.layout, old, &stamped, p2), None);
        assert_eq!(decide(Some(p2), Some(old), 3), Step::Push { version: 3 });
        assert_eq!(decide(Some(p2), Some(old), 4), Step::Both);
        // A ledger already of the new kind, or a save with no ignore list, is not touched.
        let new = Ledger {
            version: 3,
            hash: progress,
        };
        assert_eq!(brought_to_progress(&t.layout, new, &was, progress), None);
        let plain = ignoring_target(FURTHER);
        let raw = Ledger {
            version: 3,
            hash: plain.layout.fingerprint(&was),
        };
        assert_eq!(
            brought_to_progress(
                &plain.layout,
                raw,
                &was,
                plain.layout.progress_fingerprint(&was)
            ),
            None
        );
    }
}
