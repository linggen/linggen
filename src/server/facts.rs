//! Phone facts — a paired device saying "this kind of thing happened, at
//! this time" (`doc/phone-facts-design.md`). The engine turns a fact into
//! quest stamps and never learns what any kind means.
//!
//! - The wire is one control-channel message: `{"type":"fact","kind":…,"at":…}`.
//!   The device is the channel's bound identity, never the payload; an
//!   anonymous peer's fact is dropped.
//! - A fact is a reading: the phone keeps the latest `at` per kind and resends
//!   them all on every connect. No acks.
//! - Who stamps: the engine's own milestones that name the kind
//!   (`milestones::for_fact`), and every skill whose `quests.facts` declares
//!   it — through the skill's own writer (`engine::skill::tools::stamp_quest`).
//! - The ledger `~/.linggen/facts/<device>.json` remembers, per kind, the
//!   latest `at` and what each target was stamped with. A target advances only
//!   when its stamp succeeded, so a failed stamp is tried again on the next
//!   resend, and a skill that declares a kind later gets the latest value.
//!
//! Only `kind`, `at` and target ids are kept — no content has a slot. The
//! ledger stays on this Mac: never telemetry, the activity log or memory.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use super::ServerState;
use crate::engine::skill::Skill;

const MAX_KIND: usize = 48;
const FUTURE_SLACK_MIN: i64 = 5;
const MAX_AGE_DAYS: i64 = 30;
const STAMP_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";

/// A checked fact: a kind from the closed pattern, and a UTC time to the second.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fact {
    pub kind: String,
    pub at: String,
}

/// Why a fact was dropped.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Drop {
    Anonymous,
    BadKind,
    BadTime,
    TooNew,
    TooOld,
}

/// A `fact` control message arrived on a peer bound to `device` (None when
/// the peer never identified). Checked here; stamped off the event loop.
pub(crate) fn heard(state: &Arc<ServerState>, device: Option<String>, msg: &Value) {
    let (device, fact) = match accept(device, msg, chrono::Utc::now()) {
        Ok(ok) => ok,
        Err(why) => {
            tracing::debug!("[facts] dropped a fact: {why:?}");
            return;
        }
    };
    let state = Arc::clone(state);
    tokio::spawn(async move { receive(&state, &device, &fact).await });
}

/// The device and the fact, or why it is dropped. Only `kind` and `at` are
/// read from the message; anything else in it is ignored and never kept.
pub(crate) fn accept(
    device: Option<String>,
    msg: &Value,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(String, Fact), Drop> {
    let device = device.filter(|d| valid_device(d)).ok_or(Drop::Anonymous)?;
    let kind = msg["kind"].as_str().unwrap_or("");
    if !valid_kind(kind) {
        return Err(Drop::BadKind);
    }
    let at = checked_time(msg["at"].as_str().unwrap_or(""), now)?;
    Ok((
        device,
        Fact {
            kind: kind.to_string(),
            at,
        },
    ))
}

/// `<app>-<verb>`: lowercase words joined by single dashes, at most 48.
fn valid_kind(kind: &str) -> bool {
    kind.len() <= MAX_KIND
        && kind.contains('-')
        && kind.split('-').all(|w| {
            !w.is_empty()
                && w.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// A device id names a file, so it is held to a filename-safe charset.
fn valid_device(d: &str) -> bool {
    (1..=64).contains(&d.len())
        && d.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Strict RFC 3339, within the window, as `YYYY-MM-DDTHH:MM:SSZ` (UTC).
fn checked_time(raw: &str, now: chrono::DateTime<chrono::Utc>) -> Result<String, Drop> {
    let at = chrono::DateTime::parse_from_rfc3339(raw)
        .map_err(|_| Drop::BadTime)?
        .with_timezone(&chrono::Utc);
    if at > now + chrono::Duration::minutes(FUTURE_SLACK_MIN) {
        return Err(Drop::TooNew);
    }
    if at < now - chrono::Duration::days(MAX_AGE_DAYS) {
        return Err(Drop::TooOld);
    }
    Ok(at.format(STAMP_FORMAT).to_string())
}

// ---------------------------------------------------------------------------
// Targets
// ---------------------------------------------------------------------------

/// Where a fact lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// One of the engine's own milestones (`linggen.json`).
    Milestone(&'static str),
    /// A skill's quest, stamped by the skill's own writer.
    Quest { skill: String, id: String },
}

impl Target {
    /// The ledger's key for this target.
    fn key(&self) -> String {
        match self {
            Target::Milestone(id) => format!("engine:{id}"),
            Target::Quest { skill, id } => format!("skill:{skill}/{id}"),
        }
    }
}

/// Everything that takes a fact of `kind`: the engine's milestones, then each
/// skill that declares the kind with a well-formed quest id.
fn targets(kind: &str, skills: &[Skill]) -> Vec<Target> {
    let milestones = super::milestones::for_fact(kind)
        .into_iter()
        .map(Target::Milestone);
    let quests = skills.iter().filter_map(|s| {
        let id = s.quests.as_ref()?.facts.get(kind)?;
        crate::engine::skill::tools::valid_quest_id(id).then(|| Target::Quest {
            skill: s.name.clone(),
            id: id.clone(),
        })
    });
    milestones.chain(quests).collect()
}

// ---------------------------------------------------------------------------
// The ledger
// ---------------------------------------------------------------------------

/// `~/.linggen/facts/<device>.json`: per kind, the latest `at` and the `at`
/// each target was last stamped with.
#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Ledger {
    #[serde(default)]
    kinds: BTreeMap<String, KindRow>,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct KindRow {
    at: String,
    #[serde(default)]
    stamped: BTreeMap<String, String>,
}

fn ledger_file(device: &str) -> PathBuf {
    crate::paths::linggen_home()
        .join("facts")
        .join(format!("{device}.json"))
}

/// A missing or unreadable ledger starts empty: the worst it costs is a
/// stamp run again, and every writer keeps `done_at` from going back.
fn load(path: &std::path::Path) -> Ledger {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(path: &std::path::Path, ledger: &Ledger) {
    let bytes = serde_json::to_vec_pretty(ledger).unwrap_or_default();
    if let Err(e) = crate::state_fs::devices::write_atomic(path, &bytes) {
        tracing::warn!("[facts] could not write {}: {e}", path.display());
    }
}

/// Take `fact` into the ledger and stamp every target not yet stamped with
/// the kind's latest `at`. A target is marked only when its stamp succeeds.
pub(crate) async fn apply<S, F>(ledger: &mut Ledger, fact: &Fact, targets: &[Target], stamp: S)
where
    S: Fn(Target, String) -> F,
    F: Future<Output = Result<(), String>>,
{
    let row = ledger.kinds.entry(fact.kind.clone()).or_default();
    if fact.at > row.at {
        row.at = fact.at.clone();
    }
    let at = row.at.clone();
    for t in targets {
        let key = t.key();
        if row.stamped.get(&key).is_some_and(|s| *s >= at) {
            continue;
        }
        match stamp(t.clone(), at.clone()).await {
            Ok(()) => {
                tracing::info!("[facts] {} → {key} at {at}", fact.kind);
                row.stamped.insert(key, at.clone());
            }
            Err(e) => tracing::warn!(
                "[facts] {} → {key} not stamped (next connect retries): {e}",
                fact.kind
            ),
        }
    }
}

/// One fact at a time across devices: stamps run the skills' writers, which
/// each hold their own lock, but the ledger's read-merge-write wants one.
static RECEIVE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn receive(state: &Arc<ServerState>, device: &str, fact: &Fact) {
    let _one = RECEIVE.lock().await;
    let skills = state.manager.skills.list_skills().await;
    let targets = targets(&fact.kind, &skills);
    let path = ledger_file(device);
    let mut ledger = load(&path);
    apply(&mut ledger, fact, &targets, |t, at| {
        stamp_target(&skills, t, at)
    })
    .await;
    save(&path, &ledger);
}

async fn stamp_target(skills: &[Skill], target: Target, at: String) -> Result<(), String> {
    match target {
        Target::Milestone(id) => {
            tokio::task::spawn_blocking(move || super::milestones::stamp_fact(id, &at))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())
        }
        Target::Quest { skill, id } => {
            let Some(s) = skills.iter().find(|s| s.name == skill) else {
                return Err(format!("no skill '{skill}'"));
            };
            crate::engine::skill::tools::stamp_quest(s, &id, &at)
                .await
                .map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-09-24T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    fn msg(kind: &str, at: &str) -> Value {
        json!({ "type": "fact", "kind": kind, "at": at })
    }

    fn fact(kind: &str, at: &str) -> Fact {
        Fact {
            kind: kind.into(),
            at: at.into(),
        }
    }

    fn dev() -> Option<String> {
        Some("3f2a-device".into())
    }

    #[test]
    fn a_good_fact_is_accepted_and_normalised_to_utc_seconds() {
        let (d, f) = accept(
            dev(),
            &msg("cfo-import", "2026-09-24T13:03:11.482+02:00"),
            now(),
        )
        .unwrap();
        assert_eq!(d, "3f2a-device");
        assert_eq!(f, fact("cfo-import", "2026-09-24T11:03:11Z"));
    }

    /// The phone may speak before its `identify` lands: such a fact has no
    /// device, so it is dropped — never pinned on anyone. The phone's resend
    /// on the next connect brings it again, and then the channel knows.
    #[test]
    fn a_fact_before_identify_is_dropped_not_misattributed() {
        let wire = json!({"type": "fact", "kind": "cfo-import", "at": "2026-09-24T11:03:11Z"});
        assert_eq!(accept(None, &wire, now()), Err(Drop::Anonymous));
        let with_claim = json!({
            "type": "fact", "kind": "cfo-import", "at": "2026-09-24T11:03:11Z",
            "device": "someone-else", "from_device": "someone-else"
        });
        assert_eq!(accept(None, &with_claim, now()), Err(Drop::Anonymous));
        let (d, _) = accept(dev(), &with_claim, now()).unwrap();
        assert_eq!(
            d, "3f2a-device",
            "the channel's identity, never the payload's"
        );
    }

    #[test]
    fn anonymous_bad_kind_and_bad_time_are_dropped() {
        let good = "2026-09-24T11:00:00Z";
        assert_eq!(
            accept(None, &msg("cfo-import", good), now()),
            Err(Drop::Anonymous)
        );
        assert_eq!(
            accept(Some("../etc".into()), &msg("cfo-import", good), now()),
            Err(Drop::Anonymous)
        );
        for kind in [
            "",
            "cfo",
            "CFO-import",
            "cfo_import",
            "-cfo",
            "cfo-",
            "cfo--import",
            "cfo import",
            "cfo-import;rm",
            &"a-".repeat(25),
        ] {
            assert_eq!(
                accept(dev(), &msg(kind, good), now()),
                Err(Drop::BadKind),
                "{kind}"
            );
        }
        assert_eq!(
            accept(
                dev(),
                &json!({"type": "fact", "kind": 3, "at": good}),
                now()
            ),
            Err(Drop::BadKind)
        );
        for at in [
            "",
            "yesterday",
            "2026-09-24",
            "2026-09-24 11:00:00",
            "1727000000",
        ] {
            assert_eq!(
                accept(dev(), &msg("cfo-import", at), now()),
                Err(Drop::BadTime),
                "{at}"
            );
        }
        assert_eq!(
            accept(dev(), &msg("cfo-import", "2026-09-24T12:06:00Z"), now()),
            Err(Drop::TooNew)
        );
        assert!(accept(dev(), &msg("cfo-import", "2026-09-24T12:04:00Z"), now()).is_ok());
        assert_eq!(
            accept(dev(), &msg("cfo-import", "2026-08-24T11:00:00Z"), now()),
            Err(Drop::TooOld)
        );
    }

    fn skill_with(name: &str, facts: &[(&str, &str)]) -> Skill {
        let mut text = format!(
            "---\nname: {name}\ndescription: d\nquests:\n  stamp: w {{id}} {{at}}\n  facts:\n"
        );
        for (k, v) in facts {
            text.push_str(&format!("    {k}: {v}\n"));
        }
        text.push_str("---\nb\n");
        crate::extensions::skills::parse_skill_text(
            &text,
            crate::engine::skill::SkillSource::Global,
        )
        .unwrap()
    }

    #[test]
    fn targets_are_the_engine_milestones_and_the_declaring_skills() {
        let skills = vec![
            skill_with("zz-a", &[("zz-seen", "zz-a-seen")]),
            skill_with("zz-b", &[("zz-other", "zz-b-x")]),
            skill_with("zz-c", &[("zz-seen", "Bad Id")]),
        ];
        assert_eq!(
            targets("zz-seen", &skills),
            vec![Target::Quest {
                skill: "zz-a".into(),
                id: "zz-a-seen".into()
            }]
        );
        assert_eq!(
            targets("yinyue-chat", &[]),
            vec![Target::Milestone("linggen-phone-chat")]
        );
    }

    type Calls = Arc<Mutex<Vec<(String, String)>>>;

    /// A stamper that records each call and fails for keys in `failing`.
    fn stamper(
        calls: &Calls,
        failing: &'static [&'static str],
    ) -> impl Fn(Target, String) -> std::future::Ready<Result<(), String>> {
        let calls = calls.clone();
        move |t, at| {
            let key = t.key();
            calls.lock().unwrap().push((key.clone(), at));
            std::future::ready(if failing.contains(&key.as_str()) {
                Err("writer failed".into())
            } else {
                Ok(())
            })
        }
    }

    fn quest(id: &str) -> Target {
        Target::Quest {
            skill: "zz".into(),
            id: id.into(),
        }
    }

    #[tokio::test]
    async fn stamps_advance_only_forward_per_kind_and_target() {
        let calls: Calls = Default::default();
        let mut l = Ledger::default();
        let ts = [quest("zz-q")];
        apply(
            &mut l,
            &fact("zz-seen", "2026-09-22T10:00:00Z"),
            &ts,
            stamper(&calls, &[]),
        )
        .await;
        // The same fact again (a resend) and an older one: nothing runs.
        apply(
            &mut l,
            &fact("zz-seen", "2026-09-22T10:00:00Z"),
            &ts,
            stamper(&calls, &[]),
        )
        .await;
        apply(
            &mut l,
            &fact("zz-seen", "2026-09-21T10:00:00Z"),
            &ts,
            stamper(&calls, &[]),
        )
        .await;
        // A newer one stamps again.
        apply(
            &mut l,
            &fact("zz-seen", "2026-09-23T10:00:00Z"),
            &ts,
            stamper(&calls, &[]),
        )
        .await;
        let calls = calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            vec![
                (
                    "skill:zz/zz-q".to_string(),
                    "2026-09-22T10:00:00Z".to_string()
                ),
                (
                    "skill:zz/zz-q".to_string(),
                    "2026-09-23T10:00:00Z".to_string()
                ),
            ]
        );
        assert_eq!(l.kinds["zz-seen"].at, "2026-09-23T10:00:00Z");
    }

    #[tokio::test]
    async fn a_failed_stamp_is_tried_again_on_the_next_resend() {
        let calls: Calls = Default::default();
        let mut l = Ledger::default();
        let ts = [quest("zz-ok"), quest("zz-bad")];
        let f = fact("zz-seen", "2026-09-22T10:00:00Z");
        apply(&mut l, &f, &ts, stamper(&calls, &["skill:zz/zz-bad"])).await;
        assert!(!l.kinds["zz-seen"].stamped.contains_key("skill:zz/zz-bad"));
        apply(&mut l, &f, &ts, stamper(&calls, &[])).await;
        let calls = calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 3, "ok once, bad twice: {calls:?}");
        assert_eq!(calls[2].0, "skill:zz/zz-bad");
        assert_eq!(l.kinds["zz-seen"].stamped["skill:zz/zz-bad"], f.at);
    }

    #[tokio::test]
    async fn a_target_declared_later_gets_the_latest_value() {
        let calls: Calls = Default::default();
        let mut l = Ledger::default();
        let f = fact("zz-seen", "2026-09-22T10:00:00Z");
        apply(&mut l, &f, &[], stamper(&calls, &[])).await;
        apply(
            &mut l,
            &fact("zz-seen", "2026-09-20T10:00:00Z"),
            &[quest("zz-new")],
            stamper(&calls, &[]),
        )
        .await;
        assert_eq!(
            calls.lock().unwrap().clone(),
            vec![("skill:zz/zz-new".to_string(), f.at.clone())]
        );
    }

    /// The wire and the ledger carry what, when and which device — nothing
    /// else, whatever else a message holds.
    #[tokio::test]
    async fn only_kind_time_and_target_are_kept() {
        let noisy = json!({
            "type": "fact", "kind": "zz-seen", "at": "2026-09-24T11:00:00Z",
            "title": "Secret Song", "amount": 4200, "text": "hello there"
        });
        let (_, f) = accept(dev(), &noisy, now()).unwrap();
        let mut l = Ledger::default();
        let calls: Calls = Default::default();
        apply(&mut l, &f, &[quest("zz-q")], stamper(&calls, &[])).await;
        let text = serde_json::to_string(&l).unwrap();
        for leak in ["Secret", "4200", "hello", "title", "amount", "text"] {
            assert!(!text.contains(leak), "{leak} leaked: {text}");
        }
        let v: Value = serde_json::from_str(&text).unwrap();
        let row = v["kinds"]["zz-seen"].as_object().unwrap();
        let mut keys: Vec<_> = row.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, vec!["at", "stamped"]);
    }

    #[test]
    fn the_ledger_round_trips_through_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("facts").join("d.json");
        assert_eq!(load(&path), Ledger::default());
        let mut l = Ledger::default();
        l.kinds.insert(
            "zz-seen".into(),
            KindRow {
                at: "2026-09-22T10:00:00Z".into(),
                stamped: BTreeMap::from([("skill:zz/q".into(), "2026-09-22T10:00:00Z".into())]),
            },
        );
        save(&path, &l);
        assert_eq!(load(&path), l);
        std::fs::write(&path, "{ torn").unwrap();
        assert_eq!(load(&path), Ledger::default());
    }
}
