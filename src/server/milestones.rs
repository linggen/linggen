//! Setup milestones — one-time facts about this install, written as
//! real-life quests any app can read (`~/.linggen/quests/linggen.json`).
//!
//! Apps publish quests as `~/.linggen/quests/<app>.json`; a game (or any
//! reader) lists every file there. The daemon is the one thing awake that
//! can see setup facts — a phone paired, a model added, the browser
//! extension dialled in — so it keeps the `linggen` file.
//!
//! One static table of milestones, each with its own check. A minute tick
//! runs the checks still open; what they see is history:
//! - every milestone has an entry, `done_at: null` until first seen;
//! - `done_at` is set once and never changed — a fact true once stays done;
//! - entries are never removed, and entries this build doesn't know are kept.
//!
//! Most facts are daemon state read on the tick. Two are moments nothing
//! persists — a paired device reaching home over the relay, a device's chat
//! arriving — so the code that already handles them calls [`saw`], and the
//! next tick records it.

use super::ServerState;
use serde_json::{json, Value};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const APP: &str = "linggen";
const REWARD: u32 = 50;
const STAMINA: u32 = 20;
/// ling-mem rows that count as "Linggen remembers you".
const MEMORY_ROWS: u64 = 5;
const FIRST_TICK: Duration = Duration::from_secs(30);
const TICK: Duration = Duration::from_secs(60);

type CheckFuture = Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send>>;
type Check = fn(Arc<ServerState>) -> CheckFuture;

pub(super) struct Milestone {
    id: &'static str,
    /// Where the player does it: `mac`, `phone` or `both`.
    device: &'static str,
    zh: &'static str,
    en: &'static str,
    /// The Linggen page where it is done.
    open: &'static str,
    check: Check,
    /// A phone fact kind that also does it, stamped at the fact's own time
    /// (`facts.rs`). This table is the engine's own, so naming a kind here
    /// is the engine speaking for itself.
    fact: Option<&'static str>,
}

/// Milestones taken out of the table: their entries leave the file, done or
/// not, so no app keeps offering them (linggen-skill, linggen-mcp — Hanli
/// 2026-09-24).
const RETIRED: &[&str] = &["linggen-skill", "linggen-mcp"];

const MILESTONES: &[Milestone] = &[
    Milestone {
        id: "linggen-pair",
        device: "both",
        zh: "结缘 · 与手机结契",
        en: "Bind a phone · pair your phone",
        open: "/settings?tab=phone",
        check: paired_device,
        fact: None,
    },
    Milestone {
        id: "linggen-model",
        device: "mac",
        zh: "请灵 · 接入一位模型",
        en: "Summon a spirit · add a model",
        open: "/settings?tab=models",
        check: own_model,
        fact: None,
    },
    Milestone {
        id: "linggen-chatgpt",
        device: "mac",
        zh: "借天火 · 接入 ChatGPT",
        en: "Borrow heaven's fire · sign in with ChatGPT",
        open: "/settings?tab=models",
        check: chatgpt_signed_in,
        fact: None,
    },
    Milestone {
        id: "linggen-account",
        device: "mac",
        zh: "入籍 · 登录灵根账号",
        en: "Enrol · sign in to linggen.dev",
        open: "/",
        check: account_signed_in,
        fact: None,
    },
    Milestone {
        id: "linggen-browser",
        device: "mac",
        zh: "开天眼 · 装上浏览器插件",
        en: "Open the heavenly eye · connect the browser extension",
        open: "/settings?tab=tools",
        check: browser_connected,
        fact: None,
    },
    Milestone {
        id: "linggen-mission",
        device: "mac",
        zh: "定时 · 开启一个定时任务",
        en: "Keep the hours · turn on a mission",
        open: "/settings?tab=mission",
        check: user_enabled_mission,
        fact: None,
    },
    Milestone {
        id: "linggen-voice",
        device: "mac",
        zh: "唤银月 · 听见她的声音",
        en: "Call Yinyue · hear her voice",
        open: "/settings?tab=general",
        check: voice_heard,
        fact: None,
    },
    Milestone {
        id: "linggen-phone-chat",
        device: "phone",
        zh: "通灵 · 在手机上跟银月说句话",
        en: "Speak across · talk to Yinyue on your phone",
        open: "/settings?tab=phone",
        check: phone_chat,
        fact: Some("yinyue-chat"),
    },
    Milestone {
        id: "linggen-remote",
        device: "phone",
        zh: "远游 · 远程连回家",
        en: "Travel far · reach home remotely",
        open: "/settings?tab=phone",
        check: remote_peer,
        fact: None,
    },
    Milestone {
        id: "linggen-memory",
        device: "mac",
        zh: "记心 · 让 Linggen 记住你",
        en: "Be remembered · your first memories",
        open: "/",
        check: memory_rows,
        fact: None,
    },
];

// ---------------------------------------------------------------------------
// Moments the code that handles them reports
// ---------------------------------------------------------------------------

/// A moment no state keeps, reported where it happens.
#[derive(Clone, Copy)]
pub(crate) enum Sighting {
    /// A paired device identified itself on a relay-signalled peer.
    RemotePeer,
    /// A paired device sent a chat (or an inference turn) over its channel.
    PhoneChat,
}

static SEEN: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];

/// Note a moment; the next tick records it.
pub(crate) fn saw(s: Sighting) {
    SEEN[s as usize].store(true, Ordering::Relaxed);
}

fn seen(s: Sighting) -> CheckFuture {
    let v = SEEN[s as usize].load(Ordering::Relaxed);
    Box::pin(async move { Ok(v) })
}

// ---------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------

fn paired_device(_: Arc<ServerState>) -> CheckFuture {
    Box::pin(async { Ok(!crate::state_fs::devices::load_devices().is_empty()) })
}

/// A model the user configured — not an engine built-in, not a proxy room's.
fn own_model(state: Arc<ServerState>) -> CheckFuture {
    Box::pin(async move {
        let cfg = state.manager.get_config_snapshot().await;
        Ok(cfg.models.iter().any(is_own_model))
    })
}

fn is_own_model(m: &crate::config::ModelConfig) -> bool {
    use crate::provider::models::{CHATGPT_BUILTIN_MODEL_IDS, LINGGEN_CLOUD_MODEL_ID};
    !m.is_builtin
        && m.provider != "proxy"
        && m.auth_mode.as_deref() != Some("chatgpt_oauth")
        && m.id != LINGGEN_CLOUD_MODEL_ID
        && !CHATGPT_BUILTIN_MODEL_IDS.contains(&m.id.as_str())
}

fn chatgpt_signed_in(_: Arc<ServerState>) -> CheckFuture {
    Box::pin(async {
        use crate::provider::codex_auth::{codex_auth_file, CodexAuthTokens};
        Ok(CodexAuthTokens::load(&codex_auth_file()).is_valid())
    })
}

fn account_signed_in(_: Arc<ServerState>) -> CheckFuture {
    Box::pin(async { Ok(crate::account::load_account().is_some_and(|a| !a.api_token.is_empty())) })
}

fn browser_connected(state: Arc<ServerState>) -> CheckFuture {
    Box::pin(async move { Ok(state.bridge.has_connected().await) })
}

/// A mission the user turned on — theirs, a skill's, or a built-in: enabled
/// now, where what shipped it had it off. A mission on by default (the
/// built-in dream) is not the user's doing; their own ships nothing, so it
/// counts once enabled.
fn user_enabled_mission(state: Arc<ServerState>) -> CheckFuture {
    Box::pin(async move {
        let store = &state.missions;
        let missions = store.list_all_missions()?;
        Ok(missions
            .iter()
            .any(|m| turned_on(m.enabled, shipped_md(store, m).as_deref())))
    })
}

/// Enabled, and not by what shipped it. Pure.
fn turned_on(enabled: bool, shipped: Option<&str>) -> bool {
    enabled && !shipped.is_some_and(enabled_in)
}

/// The `mission.md` a mission shipped with: the skill's file (the user's
/// on/off lives beside it, in the mission's state), or the built-in's
/// embedded one. `None` for the user's own.
fn shipped_md(
    store: &crate::extensions::missions::MissionLoader,
    m: &crate::extensions::missions::Mission,
) -> Option<String> {
    if m.skill.is_some() {
        return store.read_mission_raw(&m.id).ok().flatten();
    }
    crate::cli::init::builtin_mission_md(&m.id)
}

/// Whether a mission file's frontmatter turns it on (absent = off).
fn enabled_in(md: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct Defaults {
        #[serde(default)]
        enabled: bool,
    }
    let (Some(yaml), _) = crate::extensions::frontmatter::split(md) else {
        return false;
    };
    serde_norway::from_str::<Defaults>(yaml).is_ok_and(|d| d.enabled)
}

/// Her voice is on and a surface holds her — she is being heard.
fn voice_heard(state: Arc<ServerState>) -> CheckFuture {
    Box::pin(async move {
        let pet = state.manager.get_config_snapshot().await.pet;
        Ok(pet.enabled && !pet.muted && state.yinyue_holder().is_some())
    })
}

fn phone_chat(_: Arc<ServerState>) -> CheckFuture {
    seen(Sighting::PhoneChat)
}

fn remote_peer(_: Arc<ServerState>) -> CheckFuture {
    seen(Sighting::RemotePeer)
}

/// ling-mem's row count — a metadata call. Unreachable is an error (skip).
fn memory_rows(state: Arc<ServerState>) -> CheckFuture {
    Box::pin(async move {
        let url = state.manager.get_config_snapshot().await.agent.ling_mem_url;
        let url = format!("{}/api/memory/count", url.trim_end_matches('/'));
        let body: Value = reqwest::Client::new()
            .post(&url)
            .json(&json!({}))
            .timeout(Duration::from_secs(3))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(body["data"]["count"].as_u64().unwrap_or(0) >= MEMORY_ROWS)
    })
}

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

pub(super) async fn milestone_loop(state: Arc<ServerState>) {
    tokio::time::sleep(FIRST_TICK).await;
    let path = quests_file();
    loop {
        let open = tick(&path, MILESTONES, |m| (m.check)(state.clone())).await;
        if open == 0 {
            return; // every milestone is done — nothing left to watch
        }
        tokio::time::sleep(TICK).await;
    }
}

fn quests_file() -> PathBuf {
    crate::paths::linggen_home()
        .join("quests")
        .join(format!("{APP}.json"))
}

/// One pass: run the checks still open, record what they saw. Returns how
/// many milestones remain open (a file it can't read counts as all open).
async fn tick<F>(path: &Path, table: &'static [Milestone], probe: F) -> usize
where
    F: Fn(&'static Milestone) -> CheckFuture,
{
    let doc = match load(path) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(
                "[milestones] {} unreadable ({e}) — left alone",
                path.display()
            );
            return table.len();
        }
    };
    let mut done = Vec::new();
    for m in table.iter().filter(|m| !is_done(&doc, m.id)) {
        match probe(m).await {
            Ok(true) => done.push(m.id),
            Ok(false) => {}
            Err(e) => tracing::debug!("[milestones] {} check skipped: {e}", m.id),
        }
    }
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let stamps: Vec<(&str, &str)> = done.iter().map(|id| (*id, now.as_str())).collect();
    match update(path, table, &stamps) {
        Ok(doc) => table.iter().filter(|m| !is_done(&doc, m.id)).count(),
        Err(e) => {
            tracing::warn!("[milestones] could not write {}: {e}", path.display());
            table.len()
        }
    }
}

/// One writer at a time for the file: the tick and a phone fact both
/// read-merge-write it, so neither may write over what the other just did.
static FILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Re-read the file, bring it up to the table with `stamps` (id → done_at),
/// and write it when anything changed. Returns what the file now holds.
fn update(path: &Path, table: &[Milestone], stamps: &[(&str, &str)]) -> std::io::Result<Value> {
    let _one = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut doc = load(path)?;
    if record(&mut doc, table, stamps) {
        let bytes = serde_json::to_vec_pretty(&doc).unwrap_or_default();
        crate::state_fs::devices::write_atomic(path, &bytes)?;
        let ids: Vec<&str> = stamps.iter().map(|(id, _)| *id).collect();
        tracing::info!("[milestones] recorded {}", ids.join(", "));
    }
    Ok(doc)
}

/// The milestones a phone fact of `kind` does.
pub(crate) fn for_fact(kind: &str) -> Vec<&'static str> {
    MILESTONES
        .iter()
        .filter(|m| m.fact == Some(kind))
        .map(|m| m.id)
        .collect()
}

/// A phone fact did milestone `id` at `at`: stamp it then — not at the tick —
/// when it is still open. A done milestone stays as it is (set once). Ok
/// once the file holds a `done_at` for it.
pub(crate) fn stamp_fact(id: &str, at: &str) -> std::io::Result<()> {
    stamp_fact_in(&quests_file(), MILESTONES, id, at)
}

fn stamp_fact_in(path: &Path, table: &[Milestone], id: &str, at: &str) -> std::io::Result<()> {
    let doc = update(path, table, &[(id, at)])?;
    if is_done(&doc, id) {
        return Ok(());
    }
    Err(std::io::Error::other(format!("no milestone '{id}'")))
}

/// The quests file: missing = a fresh one; anything unparseable, or without a
/// `quests` list, is an error — never overwritten as empty.
fn load(path: &Path) -> std::io::Result<Value> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(json!({ "app": APP, "quests": [] }))
        }
        Err(e) => return Err(e),
    };
    let doc: Value = serde_json::from_str(&text).map_err(std::io::Error::other)?;
    if !doc["quests"].is_array() {
        return Err(std::io::Error::other("no quests list"));
    }
    Ok(doc)
}

fn entry<'a>(doc: &'a Value, id: &str) -> Option<&'a Value> {
    doc["quests"].as_array()?.iter().find(|q| q["id"] == id)
}

fn is_done(doc: &Value, id: &str) -> bool {
    entry(doc, id).is_some_and(|q| q["done_at"].is_string())
}

/// Bring the file up to the table: drop retired milestones, add each missing
/// one (done at its stamp if just seen), and stamp `done_at` on an open one
/// just seen. An entry with a `done_at` is never otherwise touched, and
/// entries the table doesn't know are kept. True when anything changed.
fn record(doc: &mut Value, table: &[Milestone], stamps: &[(&str, &str)]) -> bool {
    let mut changed = drop_retired(doc);
    for m in table {
        let stamp = stamps.iter().find(|(id, _)| *id == m.id).map(|(_, at)| *at);
        let Some(quests) = doc["quests"].as_array_mut() else {
            return changed;
        };
        match quests.iter_mut().find(|q| q["id"] == m.id) {
            None => {
                quests.push(new_entry(m, stamp));
                changed = true;
            }
            Some(q) if !q["done_at"].is_string() && stamp.is_some() => {
                q["done_at"] = json!(stamp);
                changed = true;
            }
            Some(_) => {}
        }
    }
    changed
}

fn drop_retired(doc: &mut Value) -> bool {
    let Some(quests) = doc["quests"].as_array_mut() else {
        return false;
    };
    let before = quests.len();
    quests.retain(|q| !RETIRED.iter().any(|id| q["id"] == *id));
    quests.len() != before
}

fn new_entry(m: &Milestone, done_at: Option<&str>) -> Value {
    json!({
        "id": m.id,
        "period": "once",
        "due": true,
        "done_at": done_at,
        "reward": REWARD,
        "stamina": STAMINA,
        "open": m.open,
        "title": { "zh": m.zh, "en": m.en },
        "device": m.device,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(v: bool) -> CheckFuture {
        Box::pin(async move { Ok(v) })
    }

    fn table_ids() -> Vec<&'static str> {
        MILESTONES.iter().map(|m| m.id).collect()
    }

    #[test]
    fn table_ids_are_unique_and_devices_known() {
        let mut ids = table_ids();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), MILESTONES.len());
        for m in MILESTONES {
            assert!(["mac", "phone", "both"].contains(&m.device), "{}", m.id);
            assert!(m.open.starts_with('/'), "{}", m.id);
        }
    }

    #[tokio::test]
    async fn fact_table_becomes_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quests").join("linggen.json");
        let open = tick(&path, MILESTONES, |m| ok(m.id == "linggen-pair")).await;
        assert_eq!(open, MILESTONES.len() - 1);

        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc["app"], "linggen");
        let quests = doc["quests"].as_array().unwrap();
        assert_eq!(quests.len(), MILESTONES.len());
        let pair = entry(&doc, "linggen-pair").unwrap();
        assert!(pair["done_at"].is_string());
        assert_eq!(pair["period"], "once");
        assert_eq!(pair["device"], "both");
        assert_eq!(pair["due"], true);
        assert_eq!(pair["reward"], REWARD);
        assert_eq!(pair["stamina"], STAMINA);
        assert_eq!(pair["title"]["zh"], "结缘 · 与手机结契");
        assert!(entry(&doc, "linggen-model").unwrap()["done_at"].is_null());
    }

    #[test]
    fn a_retired_milestone_leaves_the_file() {
        let mut doc = json!({ "app": "linggen", "quests": [
            { "id": "linggen-skill", "done_at": null },
            { "id": "linggen-mcp", "done_at": "2026-09-01T00:00:00.000Z" },
            { "id": "somebody-else", "done_at": null }
        ]});
        assert!(record(&mut doc, &[], &[]));
        assert!(entry(&doc, "linggen-skill").is_none());
        assert!(
            entry(&doc, "linggen-mcp").is_none(),
            "done or not, it leaves"
        );
        assert!(entry(&doc, "somebody-else").is_some());
        assert!(!MILESTONES.iter().any(|m| RETIRED.contains(&m.id)));
    }

    #[tokio::test]
    async fn existing_entries_are_never_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("linggen.json");
        let old = json!({ "app": "linggen", "quests": [
            { "id": "linggen-pair", "done_at": "2026-01-01T00:00:00.000Z", "reward": 7, "custom": 1 },
            { "id": "somebody-else", "done_at": null }
        ]});
        std::fs::write(&path, old.to_string()).unwrap();

        // Everything is true now — the old stamp must stay.
        tick(&path, MILESTONES, |_| ok(true)).await;
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let pair = entry(&doc, "linggen-pair").unwrap();
        assert_eq!(pair["done_at"], "2026-01-01T00:00:00.000Z");
        assert_eq!(pair["reward"], 7);
        assert_eq!(pair["custom"], 1);
        assert!(
            entry(&doc, "somebody-else").is_some(),
            "unknown entries are kept"
        );
        assert_eq!(
            doc["quests"].as_array().unwrap().len(),
            MILESTONES.len() + 1
        );

        // A fact that goes false later stays done.
        let stamped = entry(&doc, "linggen-model").unwrap()["done_at"].clone();
        let open = tick(&path, MILESTONES, |_| ok(false)).await;
        assert_eq!(open, 0);
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(entry(&doc, "linggen-model").unwrap()["done_at"], stamped);
    }

    #[tokio::test]
    async fn open_entry_gets_stamped_once_seen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("linggen.json");
        tick(&path, MILESTONES, |_| ok(false)).await;
        assert!(entry(&load(&path).unwrap(), "linggen-voice").unwrap()["done_at"].is_null());
        tick(&path, MILESTONES, |m| ok(m.id == "linggen-voice")).await;
        assert!(entry(&load(&path).unwrap(), "linggen-voice").unwrap()["done_at"].is_string());
    }

    #[tokio::test]
    async fn a_check_that_errors_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("linggen.json");
        let open = tick(&path, MILESTONES, |m| {
            if m.id == "linggen-memory" {
                Box::pin(async { anyhow::bail!("ling-mem unreachable") })
            } else {
                ok(true)
            }
        })
        .await;
        assert_eq!(open, 1);
        let doc = load(&path).unwrap();
        assert!(entry(&doc, "linggen-memory").unwrap()["done_at"].is_null());
        assert!(entry(&doc, "linggen-pair").unwrap()["done_at"].is_string());
    }

    #[tokio::test]
    async fn an_unreadable_file_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("linggen.json");
        std::fs::write(&path, "{ half a file").unwrap();
        tick(&path, MILESTONES, |_| ok(true)).await;
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ half a file");
    }

    #[tokio::test]
    async fn write_is_atomic_and_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("linggen.json");
        tick(&path, MILESTONES, |_| ok(false)).await;
        tick(&path, MILESTONES, |_| ok(true)).await;
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["linggen.json".to_string()]);
        assert!(load(&path).is_ok());
    }

    #[test]
    fn own_models_exclude_builtins_and_proxies() {
        let base: crate::config::ModelConfig = serde_json::from_value(json!({
            "id": "qwen", "provider": "ollama", "url": "http://x", "model": "qwen",
            "api_key": null, "keep_alive": null
        }))
        .unwrap();
        assert!(is_own_model(&base));
        let proxy = crate::config::ModelConfig {
            provider: "proxy".into(),
            ..base.clone()
        };
        let oauth = crate::config::ModelConfig {
            auth_mode: Some("chatgpt_oauth".into()),
            ..base.clone()
        };
        let cloud = crate::config::ModelConfig {
            id: crate::provider::models::LINGGEN_CLOUD_MODEL_ID.into(),
            ..base.clone()
        };
        let builtin = crate::config::ModelConfig {
            is_builtin: true,
            ..base
        };
        for m in [proxy, oauth, cloud, builtin] {
            assert!(!is_own_model(&m), "{}", m.id);
        }
    }

    #[test]
    fn builtin_missions_are_known() {
        assert!(crate::cli::init::builtin_mission_ids().contains(&"dream".to_string()));
    }

    /// The dream ships on, so it never counts; a skill mission shipped off
    /// (cfo:watch) counts once the user turns it on; the user's own counts
    /// once enabled.
    #[test]
    fn only_a_mission_the_user_turned_on_counts() {
        let dream = crate::cli::init::builtin_mission_md("dream").unwrap();
        assert!(enabled_in(&dream), "the dream is on by default");
        assert!(!enabled_in(
            "---\nschedule: \"0 * * * *\"\nenabled: false\n---\nbody"
        ));
        assert!(!enabled_in("---\nschedule: \"0 * * * *\"\n---\nbody"));
        assert!(!enabled_in("no frontmatter"));

        let watch = "---\nschedule: \"0 3 * * *\"\nenabled: false\n---\nWatch.";
        assert!(
            !turned_on(false, Some(watch)),
            "shipped off, not yet turned on"
        );
        assert!(turned_on(true, Some(watch)), "the user turned cfo:watch on");
        assert!(
            !turned_on(true, Some(&dream)),
            "on by default is not the user's doing"
        );
        assert!(turned_on(true, None), "the user's own mission, enabled");
        assert!(!turned_on(false, None));
    }

    /// A phone fact stamps its milestone at the fact's own time, not the
    /// tick's; a done milestone stays as it was.
    #[test]
    fn a_fact_stamps_its_milestone_at_the_facts_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("linggen.json");
        assert_eq!(for_fact("yinyue-chat"), vec!["linggen-phone-chat"]);
        assert!(for_fact("zz-other").is_empty());

        stamp_fact_in(
            &path,
            MILESTONES,
            "linggen-phone-chat",
            "2026-09-20T08:00:00Z",
        )
        .unwrap();
        let doc = load(&path).unwrap();
        assert_eq!(
            entry(&doc, "linggen-phone-chat").unwrap()["done_at"],
            "2026-09-20T08:00:00Z"
        );
        assert!(entry(&doc, "linggen-pair").unwrap()["done_at"].is_null());
        assert_eq!(doc["quests"].as_array().unwrap().len(), MILESTONES.len());

        stamp_fact_in(
            &path,
            MILESTONES,
            "linggen-phone-chat",
            "2026-09-24T08:00:00Z",
        )
        .unwrap();
        let doc = load(&path).unwrap();
        assert_eq!(
            entry(&doc, "linggen-phone-chat").unwrap()["done_at"],
            "2026-09-20T08:00:00Z",
            "set once"
        );
        assert!(stamp_fact_in(&path, MILESTONES, "nope", "2026-09-24T08:00:00Z").is_err());
    }

    #[tokio::test]
    async fn sightings_are_remembered() {
        saw(Sighting::RemotePeer);
        assert!(seen(Sighting::RemotePeer).await.unwrap());
    }
}
