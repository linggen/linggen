//! Setup milestones — one-time facts about this install, written as
//! real-life quests any app can read (`~/.linggen/quests/linggen.json`).
//!
//! Apps publish quests as `~/.linggen/quests/<app>.json`; a game (or any
//! reader) lists every file there. The daemon is the one thing awake that
//! can see setup facts — a phone paired, a message to Yinyue, an app's page
//! opened — so it keeps the `linggen` file: 开府, the first steps of
//! Linggen and its apps (Hanli, 2026-11-02).
//!
//! One static table of milestones, each with how it is seen. A minute tick
//! runs the checks still open; what they see is history:
//! - every milestone has an entry, `done_at: null` until first seen;
//! - `done_at` is set once and never changed — a fact true once stays done;
//! - an entry's words (title, how, open, ask, device) follow the table; its
//!   `done_at`, pay and any field the table doesn't write are kept;
//! - retired entries leave the file; entries this build doesn't know are kept.
//!
//! Most facts are daemon state read on the tick. Moments nothing persists —
//! a person's message to an agent, a tool an agent ran, an app's page or
//! audio served — are [`Sighting`]s: the code that already handles them
//! calls [`saw`] (or the event watch below sees them go by), and the next
//! tick records it. A sighting is a general fact (agent, tool, app); only
//! this table names which ones are milestones.
//!
//! Beside them, a few **recurring** rows ([`RECURRING`], 人间功课 — Hanli
//! 2026-11-02): a chore done again and again, its `done_at` the latest time
//! seen (later wins, never back). 问安 is a person's message to Yinyue that
//! day. Same file, same lock.

use super::ServerState;
use crate::engine::events::ServerEvent;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const APP: &str = "linggen";
const REWARD: u32 = 50;
const FIRST_TICK: Duration = Duration::from_secs(30);
const TICK: Duration = Duration::from_secs(60);
/// Yinyue's agent id — the engine's own companion (`config.pet.pet`).
const YINYUE: &str = "yinyue";

type CheckFuture = Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send>>;
type Check = fn(Arc<ServerState>) -> CheckFuture;

/// How a milestone is seen done.
#[derive(Clone, Copy)]
pub(super) enum Seen {
    /// Daemon state, read on the tick.
    State(Check),
    /// A moment reported where it happens.
    Saw(Sighting<'static>),
    /// Another app's own quest row was ever done (`<app>.json`, row `id`):
    /// the app keeps the record, the engine only reads it.
    Quest(&'static str, &'static str),
    /// Only a phone fact does it (`fact`).
    Phone,
}

/// Where a milestone is done: a Linggen page, or an app's own entry (its
/// SKILL.md `app.entry`, read from the registry — `/apps/<app>/` alone is the
/// shell) with an optional query.
#[derive(Clone, Copy)]
pub(super) enum Open {
    Page(&'static str),
    App(&'static str, &'static str),
}

pub(super) struct Milestone {
    id: &'static str,
    /// Where the player does it: `mac`, `phone`, `both` (either) or `pair`
    /// (needs both — listed only to a player who has a Mac and a phone).
    device: &'static str,
    zh: &'static str,
    en: &'static str,
    /// 怎么做: one short line, ≤ 20 words.
    how_zh: &'static str,
    how_en: &'static str,
    open: Open,
    /// The line the card's button says to Yinyue in one tap; `Some(("", ""))`
    /// opens a box to her instead.
    ask: Option<(&'static str, &'static str)>,
    seen: Seen,
    /// A phone fact kind that also does it, stamped at the fact's own time
    /// (`facts.rs`). This table is the engine's own, so naming a kind here
    /// is the engine speaking for itself.
    fact: Option<&'static str>,
}

/// Milestones taken out of the table: their entries leave the file, done or
/// not, so no app keeps offering them (linggen-skill, linggen-mcp — Hanli
/// 2026-09-24; the rest with the new 开府, 2026-11-02: sign-in comes before
/// the game opens, and the others were not first steps). A game keeps what it
/// already paid for them.
const RETIRED: &[&str] = &[
    "linggen-skill",
    "linggen-mcp",
    "linggen-model",
    "linggen-account",
    "linggen-memory",
    "linggen-remote",
    "linggen-mission",
    "linggen-browser",
    "linggen-chatgpt",
    "linggen-phone-chat",
];

const MILESTONES: &[Milestone] = &[
    Milestone {
        id: "linggen-chat",
        device: "both",
        zh: "问道 · 跟银月说句话",
        en: "Ask the Way · say something to Yinyue",
        how_zh: "在聊天里打一句话给她",
        how_en: "Type her a line in the chat",
        open: Open::Page("/"),
        ask: Some(("", "")),
        seen: Seen::Saw(Sighting::UserMessage { agent: YINYUE }),
        fact: Some("yinyue-chat"),
    },
    Milestone {
        id: "linggen-ask-news",
        device: "both",
        zh: "问天 · 问她今天有什么大新闻",
        en: "Ask the Heavens · ask her for today's big news",
        how_zh: "点「问问她」",
        how_en: "Tap Ask her",
        open: Open::Page("/"),
        ask: Some(("今天有什么大新闻？", "What's the big news today?")),
        seen: Seen::Saw(Sighting::ToolRan {
            agent: YINYUE,
            tool: "WebSearch",
        }),
        fact: Some("yinyue-search"),
    },
    Milestone {
        id: "linggen-voice",
        device: "mac",
        zh: "唤银月 · 听见她的声音",
        en: "Call Yinyue · hear her voice",
        how_zh: "点她话旁边的喇叭",
        how_en: "Tap the speaker beside her words",
        open: Open::Page("/settings?tab=agents"),
        ask: None,
        seen: Seen::State(voice_heard),
        fact: None,
    },
    Milestone {
        id: "cfo-open",
        device: "both",
        zh: "算账 · 打开 CFO 看看账本",
        en: "Keep the books · open CFO and look",
        how_zh: "点「去 CFO」",
        how_en: "Tap Go to CFO",
        open: Open::App("cfo", ""),
        ask: None,
        seen: Seen::Saw(Sighting::AppPage { app: "cfo" }),
        fact: Some("cfo-open"),
    },
    Milestone {
        id: "dj-play",
        device: "both",
        zh: "听琴 · 在 DJ 里放一首歌",
        en: "Hear the qin · play a song in DJ",
        how_zh: "点「去 DJ」，再点一首",
        how_en: "Tap Go to DJ, then a song",
        open: Open::App("dj", ""),
        ask: None,
        seen: Seen::Saw(Sighting::AppAudio { app: "dj" }),
        fact: Some("dj-play"),
    },
    Milestone {
        id: "health-open",
        device: "phone",
        zh: "望气 · 打开 Health 看一眼今天",
        en: "Read the qi · open Health and see today",
        how_zh: "点「去 Health」",
        how_en: "Tap Go to Health",
        open: Open::App("health", ""),
        ask: None,
        seen: Seen::Phone,
        fact: Some("health-open"),
    },
    Milestone {
        id: "linggen-notify",
        device: "phone",
        zh: "开灵讯 · 允许银月发通知",
        en: "Open the spirit line · let Yinyue notify you",
        how_zh: "点「允许」",
        how_en: "Tap Allow",
        open: Open::Page("/settings?tab=notify"),
        ask: None,
        seen: Seen::Phone,
        fact: Some("linggen-notify"),
    },
    Milestone {
        id: "shifu-scan-first",
        device: "mac",
        zh: "洒扫 · 让 Shifu 扫描一次",
        en: "Sweep the halls · let Shifu scan once",
        how_zh: "点「去 Shifu」，再点「扫描」",
        how_en: "Tap Go to Shifu, then Scan",
        open: Open::App("apple-shifu", "?tab=system"),
        ask: None,
        seen: Seen::Quest("apple-shifu", "shifu-scan"),
        fact: None,
    },
    Milestone {
        id: "linggen-image",
        device: "mac",
        zh: "灵眼 · 拖一张图给银月看",
        en: "The spirit eye · show Yinyue a picture",
        how_zh: "拖一张图进聊天框，@银月 发给她",
        how_en: "Drop a picture in the chat and @Yinyue",
        open: Open::Page("/"),
        ask: None,
        seen: Seen::Saw(Sighting::UserImage { agent: YINYUE }),
        fact: None,
    },
    Milestone {
        id: "linggen-pair",
        device: "pair",
        zh: "结缘 · 手机和 Mac 结契",
        en: "Bind the two · pair your phone and Mac",
        how_zh: "扫一下二维码",
        how_en: "Scan the QR code",
        open: Open::Page("/settings?tab=phone"),
        ask: None,
        seen: Seen::State(paired_device),
        fact: None,
    },
];

/// A recurring row: done again each `period`; `done_at` is the latest time
/// its agent heard a person's message, or its phone fact said so.
pub(super) struct Recurring {
    id: &'static str,
    kind: &'static str,
    period: &'static str,
    device: &'static str,
    zh: &'static str,
    en: &'static str,
    how_zh: &'static str,
    how_en: &'static str,
    open: &'static str,
    reward: u32,
    /// The agent a person's message to counts.
    agent: &'static str,
    fact: Option<&'static str>,
}

const RECURRING: &[Recurring] = &[Recurring {
    id: "linggen-greet",
    kind: "greet",
    period: "day",
    device: "both",
    zh: "问安 · 跟银月道声早或晚安",
    en: "Pay respects · a good morning or good night to Yinyue",
    how_zh: "在聊天里跟银月说句话",
    how_en: "Say a word to Yinyue in the chat",
    open: "/",
    reward: 15,
    agent: YINYUE,
    fact: Some("yinyue-chat"),
}];

/// agent → the latest time a person's message reached it (RFC 3339, UTC).
static LAST_HEARD: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

fn heard(agent: &str, at: &str) {
    let mut last = LAST_HEARD.lock().unwrap_or_else(|e| e.into_inner());
    if last.len() < 256 || last.contains_key(agent) {
        let next = later(last.get(agent).map(String::as_str), Some(at))
            .unwrap_or(at)
            .to_string();
        last.insert(agent.to_string(), next);
    }
}

fn last_heard(agent: &str) -> Option<String> {
    LAST_HEARD
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(agent)
        .cloned()
}

/// The later of two times; an unreadable one gives way.
fn later<'a>(a: Option<&'a str>, b: Option<&'a str>) -> Option<&'a str> {
    let t = |v: Option<&str>| v.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
    match (t(a), t(b)) {
        (Some(x), Some(y)) => Some(if y > x { b.unwrap() } else { a.unwrap() }),
        (Some(_), None) => a,
        (None, Some(_)) => b,
        (None, None) => None,
    }
}

// ---------------------------------------------------------------------------
// Moments the code that handles them reports
// ---------------------------------------------------------------------------

/// A moment no state keeps, reported where it happens. Each names only
/// general things — an agent, a tool, an app — never a milestone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Sighting<'a> {
    /// A person's own message reached `agent` (any device's chat).
    UserMessage { agent: &'a str },
    /// …and it carried a picture.
    UserImage { agent: &'a str },
    /// `agent` ran `tool` in a turn.
    ToolRan { agent: &'a str, tool: &'a str },
    /// An app's page (its `app.entry`) was served.
    AppPage { app: &'a str },
    /// An app served an audio file — a page playing sound.
    AppAudio { app: &'a str },
}

impl Sighting<'_> {
    fn key(&self) -> String {
        match self {
            Sighting::UserMessage { agent } => format!("msg:{agent}"),
            Sighting::UserImage { agent } => format!("img:{agent}"),
            Sighting::ToolRan { agent, tool } => format!("tool:{agent}:{tool}"),
            Sighting::AppPage { app } => format!("page:{app}"),
            Sighting::AppAudio { app } => format!("audio:{app}"),
        }
    }
}

static SEEN: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Note a moment; the next tick records it.
pub(crate) fn saw(s: Sighting<'_>) {
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen.len() < 4096 {
        seen.insert(s.key());
    }
}

fn was_seen(s: Sighting<'_>) -> bool {
    SEEN.lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&s.key())
}

/// The events every surface already gets: a person's message to an agent,
/// and an agent starting a tool. Seen here, so the chat and the tool loop
/// carry no milestone code.
fn sighting_of(ev: &ServerEvent) -> Vec<Sighting<'_>> {
    match ev {
        ServerEvent::Message {
            from, to, images, ..
        } if from == "user" => {
            let mut out = vec![Sighting::UserMessage { agent: to }];
            if !images.is_empty() {
                out.push(Sighting::UserImage { agent: to });
            }
            out
        }
        ServerEvent::ContentBlockStart {
            agent_id,
            block_type,
            tool: Some(tool),
            ..
        } if block_type == "tool_use" => vec![Sighting::ToolRan {
            agent: agent_id,
            tool,
        }],
        _ => Vec::new(),
    }
}

async fn watch_events(state: Arc<ServerState>) {
    let mut rx = state.events_tx.subscribe();
    loop {
        match rx.recv().await {
            Ok(ev) => {
                let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                for s in sighting_of(&ev) {
                    if let Sighting::UserMessage { agent } = s {
                        heard(agent, &now);
                    }
                    saw(s);
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(_) => return,
        }
    }
}

// ---------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------

fn probe(state: Arc<ServerState>, m: &'static Milestone) -> CheckFuture {
    match m.seen {
        Seen::State(check) => check(state),
        Seen::Saw(s) => {
            let v = was_seen(s);
            Box::pin(async move { Ok(v) })
        }
        Seen::Quest(app, id) => {
            Box::pin(async move { Ok(quest_ever_done(&quests_dir(), app, id)) })
        }
        Seen::Phone => Box::pin(async { Ok(false) }),
    }
}

fn paired_device(_: Arc<ServerState>) -> CheckFuture {
    Box::pin(async { Ok(!crate::state_fs::devices::load_devices().is_empty()) })
}

/// Her voice is on and a surface holds her — she is being heard.
fn voice_heard(state: Arc<ServerState>) -> CheckFuture {
    Box::pin(async move {
        let pet = state.manager.get_config_snapshot().await.pet;
        Ok(pet.enabled && !pet.muted && state.yinyue_holder().is_some())
    })
}

/// Whether `<app>.json` in `dir` holds row `id` with a `done_at`. Pure read.
fn quest_ever_done(dir: &Path, app: &str, id: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join(format!("{app}.json"))) else {
        return false;
    };
    let doc: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    entry(&doc, id).is_some_and(|q| q["done_at"].is_string())
}

/// Each app milestone's page, from the registry (`/apps/<app>/<entry>`).
async fn app_opens(state: &ServerState, table: &[Milestone]) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for m in table {
        let Open::App(app, query) = m.open else {
            continue;
        };
        let entry = state
            .skills
            .get_skill(app)
            .await
            .and_then(|s| s.app)
            .map(|a| a.entry);
        if let Some(entry) = entry.filter(|e| !e.contains("..")) {
            out.push((m.id, format!("/apps/{app}/{entry}{query}")));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

pub(super) async fn milestone_loop(state: Arc<ServerState>) {
    tokio::spawn(watch_events(state.clone()));
    tokio::time::sleep(FIRST_TICK).await;
    let path = quests_file();
    // Kept alive even with every milestone done: the table's words still
    // follow it, and a fact keeps arriving.
    loop {
        let opens = app_opens(&state, MILESTONES).await;
        tick(&path, MILESTONES, &opens, |m| probe(state.clone(), m)).await;
        let heard: Vec<(&str, String)> = RECURRING
            .iter()
            .filter_map(|r| last_heard(r.agent).map(|at| (r.id, at)))
            .collect();
        let stamps: Vec<(&str, &str)> = heard.iter().map(|(id, at)| (*id, at.as_str())).collect();
        if let Err(e) = recur(&path, RECURRING, &stamps) {
            tracing::warn!("[milestones] could not write {}: {e}", path.display());
        }
        tokio::time::sleep(TICK).await;
    }
}

fn quests_dir() -> PathBuf {
    crate::paths::quests_dir()
}

fn quests_file() -> PathBuf {
    quests_dir().join(format!("{APP}.json"))
}

/// One pass: run the checks still open, record what they saw, with `opens`
/// (id → the app page it is done on). Returns how many milestones remain
/// open (a file it can't read counts as all open).
async fn tick<F>(
    path: &Path,
    table: &'static [Milestone],
    opens: &[(&str, String)],
    probe: F,
) -> usize
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
    match update(path, table, &stamps, opens) {
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

/// Re-read the file, bring it up to the table with `stamps` (id → done_at)
/// and `opens`, and write it when anything changed. Returns what the file
/// now holds.
fn update(
    path: &Path,
    table: &[Milestone],
    stamps: &[(&str, &str)],
    opens: &[(&str, String)],
) -> std::io::Result<Value> {
    let _one = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut doc = load(path)?;
    if record(&mut doc, table, stamps, opens) {
        let bytes = serde_json::to_vec_pretty(&doc).unwrap_or_default();
        crate::state_fs::devices::write_atomic(path, &bytes)?;
        let ids: Vec<&str> = stamps.iter().map(|(id, _)| *id).collect();
        tracing::info!("[milestones] recorded {}", ids.join(", "));
    }
    Ok(doc)
}

/// The milestones and recurring rows a phone fact of `kind` does.
pub(crate) fn for_fact(kind: &str) -> Vec<&'static str> {
    let once = MILESTONES
        .iter()
        .filter(|m| m.fact == Some(kind))
        .map(|m| m.id);
    let again = RECURRING
        .iter()
        .filter(|r| r.fact == Some(kind))
        .map(|r| r.id);
    once.chain(again).collect()
}

/// A phone fact did milestone `id` at `at`: stamp it then — not at the tick —
/// when it is still open. A done milestone stays as it is (set once); a
/// recurring row takes the later time. Ok once the file holds a `done_at`.
pub(crate) fn stamp_fact(id: &str, at: &str) -> std::io::Result<()> {
    if RECURRING.iter().any(|r| r.id == id) {
        let doc = recur(&quests_file(), RECURRING, &[(id, at)])?;
        return if is_done(&doc, id) {
            Ok(())
        } else {
            Err(std::io::Error::other(format!("'{id}' not stamped")))
        };
    }
    stamp_fact_in(&quests_file(), MILESTONES, id, at)
}

/// Bring the recurring rows up to their table, each `done_at` the later of
/// the file's and `stamps` (id → time); under the file's one lock. Returns
/// what the file now holds.
fn recur(path: &Path, table: &[Recurring], stamps: &[(&str, &str)]) -> std::io::Result<Value> {
    let _one = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut doc = load(path)?;
    let mut changed = false;
    for r in table {
        let stamp = stamps.iter().find(|(id, _)| *id == r.id).map(|(_, at)| *at);
        let Some(quests) = doc["quests"].as_array_mut() else {
            break;
        };
        let fresh = recurring_words(r);
        let Some(q) = quests.iter_mut().find(|q| q["id"] == r.id) else {
            let mut q = fresh;
            q["id"] = json!(r.id);
            q["done_at"] = json!(later(None, stamp));
            quests.push(q);
            changed = true;
            continue;
        };
        for (key, value) in fresh.as_object().into_iter().flatten() {
            if q.get(key) != Some(value) {
                q[key.as_str()] = value.clone();
                changed = true;
            }
        }
        let held = q["done_at"].as_str().map(str::to_string);
        let next = later(held.as_deref(), stamp).map(str::to_string);
        if next != held {
            q["done_at"] = json!(next);
            changed = true;
        }
    }
    if changed {
        let bytes = serde_json::to_vec_pretty(&doc).unwrap_or_default();
        crate::state_fs::devices::write_atomic(path, &bytes)?;
    }
    Ok(doc)
}

/// What a recurring row says, beside its id and `done_at`.
fn recurring_words(r: &Recurring) -> Value {
    json!({
        "kind": r.kind,
        "period": r.period,
        "device": r.device,
        "due": true,
        "reward": r.reward,
        "open": r.open,
        "title": { "zh": r.zh, "en": r.en },
        "how": { "zh": r.how_zh, "en": r.how_en },
        "ask": { "zh": "", "en": "" },
    })
}

fn stamp_fact_in(path: &Path, table: &[Milestone], id: &str, at: &str) -> std::io::Result<()> {
    let doc = update(path, table, &[(id, at)], &[])?;
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
/// one (done at its stamp if just seen), stamp `done_at` on an open one just
/// seen, and set each entry's words to the table's. A `done_at` is never
/// otherwise touched, nor the pay or a field the table doesn't write, and
/// entries the table doesn't know are kept. True when anything changed.
fn record(
    doc: &mut Value,
    table: &[Milestone],
    stamps: &[(&str, &str)],
    opens: &[(&str, String)],
) -> bool {
    let mut changed = drop_retired(doc);
    for m in table {
        let stamp = stamps.iter().find(|(id, _)| *id == m.id).map(|(_, at)| *at);
        let open = opens
            .iter()
            .find(|(id, _)| *id == m.id)
            .map(|(_, o)| o.as_str());
        let Some(quests) = doc["quests"].as_array_mut() else {
            return changed;
        };
        let Some(q) = quests.iter_mut().find(|q| q["id"] == m.id) else {
            quests.push(new_entry(m, stamp, open));
            changed = true;
            continue;
        };
        if !q["done_at"].is_string() && stamp.is_some() {
            q["done_at"] = json!(stamp);
            changed = true;
        }
        changed |= refresh(q, m, open);
    }
    changed
}

/// The table's words on an entry (title, how, device, ask, and the page when
/// known). True when any differed.
fn refresh(q: &mut Value, m: &Milestone, open: Option<&str>) -> bool {
    let mut changed = false;
    let fresh = words(m, open);
    for (key, value) in fresh.as_object().into_iter().flatten() {
        if q.get(key) != Some(value) {
            q[key.as_str()] = value.clone();
            changed = true;
        }
    }
    if m.ask.is_none() && q.as_object_mut().and_then(|o| o.remove("ask")).is_some() {
        changed = true;
    }
    changed
}

/// What an entry says: its title, 怎么做, device, the line it asks, and its
/// page — an app's only once the registry named its entry.
fn words(m: &Milestone, open: Option<&str>) -> Value {
    let mut w = json!({
        "title": { "zh": m.zh, "en": m.en },
        "how": { "zh": m.how_zh, "en": m.how_en },
        "device": m.device,
    });
    if let Some((zh, en)) = m.ask {
        w["ask"] = json!({ "zh": zh, "en": en });
    }
    match (m.open, open) {
        (_, Some(o)) => w["open"] = json!(o),
        (Open::Page(p), None) => w["open"] = json!(p),
        (Open::App(..), None) => {}
    }
    w
}

fn drop_retired(doc: &mut Value) -> bool {
    let Some(quests) = doc["quests"].as_array_mut() else {
        return false;
    };
    let before = quests.len();
    quests.retain(|q| !RETIRED.iter().any(|id| q["id"] == *id));
    quests.len() != before
}

fn new_entry(m: &Milestone, done_at: Option<&str>, open: Option<&str>) -> Value {
    let mut q = json!({
        "id": m.id,
        "period": "once",
        "due": true,
        "done_at": done_at,
        "reward": REWARD,
    });
    for (k, v) in words(m, open).as_object().into_iter().flatten() {
        q[k.as_str()] = v.clone();
    }
    if q.get("open").is_none() {
        // An app the registry has not named yet: its own folder until it does.
        if let Open::App(app, _) = m.open {
            q["open"] = json!(format!("/apps/{app}/"));
        }
    }
    q
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

    fn opens() -> Vec<(&'static str, String)> {
        vec![("cfo-open", "/apps/cfo/scripts/index.html".to_string())]
    }

    #[test]
    fn table_ids_are_unique_and_devices_known() {
        let mut ids = table_ids();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), MILESTONES.len());
        for m in MILESTONES {
            assert!(
                ["mac", "phone", "both", "pair"].contains(&m.device),
                "{}",
                m.id
            );
            if let Open::Page(p) = m.open {
                assert!(p.starts_with('/'), "{}", m.id);
            }
            assert!(m.how_zh.chars().count() <= 20, "{}", m.id);
            assert!(m.how_en.split_whitespace().count() <= 20, "{}", m.id);
            assert!(!RETIRED.contains(&m.id), "{}", m.id);
        }
    }

    /// Mac-only 7, phone-only 6 (no voice on the phone yet), both 10.
    #[test]
    fn each_device_set_lists_its_own() {
        let fits = |mac: bool, phone: bool| {
            MILESTONES
                .iter()
                .filter(|m| match m.device {
                    "pair" => mac && phone,
                    "both" => true,
                    "mac" => mac,
                    _ => phone,
                })
                .count()
        };
        assert_eq!(fits(true, false), 7);
        assert_eq!(fits(false, true), 6);
        assert_eq!(fits(true, true), 10);
    }

    #[tokio::test]
    async fn fact_table_becomes_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quests").join("linggen.json");
        let open = tick(&path, MILESTONES, &opens(), |m| ok(m.id == "linggen-pair")).await;
        assert_eq!(open, MILESTONES.len() - 1);

        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc["app"], "linggen");
        let quests = doc["quests"].as_array().unwrap();
        assert_eq!(quests.len(), MILESTONES.len());
        let pair = entry(&doc, "linggen-pair").unwrap();
        assert!(pair["done_at"].is_string());
        assert_eq!(pair["period"], "once");
        assert_eq!(pair["device"], "pair");
        assert_eq!(pair["due"], true);
        assert_eq!(pair["reward"], REWARD);
        // No quest pays 体力 (Lingjing, 2026-11-02): the rows carry none.
        assert!(pair.get("stamina").is_none());
        assert_eq!(pair["title"]["zh"], "结缘 · 手机和 Mac 结契");
        assert_eq!(pair["how"]["zh"], "扫一下二维码");
        assert!(pair.get("ask").is_none());
        let news = entry(&doc, "linggen-ask-news").unwrap();
        assert_eq!(news["ask"]["zh"], "今天有什么大新闻？");
        assert!(news["done_at"].is_null());
        assert_eq!(
            entry(&doc, "cfo-open").unwrap()["open"],
            "/apps/cfo/scripts/index.html"
        );
        assert_eq!(
            entry(&doc, "dj-play").unwrap()["open"],
            "/apps/dj/",
            "until the registry names it"
        );
    }

    #[test]
    fn a_retired_milestone_leaves_the_file() {
        let mut doc = json!({ "app": "linggen", "quests": [
            { "id": "linggen-skill", "done_at": null },
            { "id": "linggen-mcp", "done_at": "2026-09-01T00:00:00.000Z" },
            { "id": "linggen-model", "done_at": "2026-09-01T00:00:00.000Z" },
            { "id": "linggen-phone-chat", "done_at": null },
            { "id": "somebody-else", "done_at": null }
        ]});
        assert!(record(&mut doc, &[], &[], &[]));
        for id in [
            "linggen-skill",
            "linggen-mcp",
            "linggen-model",
            "linggen-phone-chat",
        ] {
            assert!(entry(&doc, id).is_none(), "done or not, {id} leaves");
        }
        assert!(entry(&doc, "somebody-else").is_some());
    }

    /// done_at, pay and unknown fields stay; the words follow the table.
    #[tokio::test]
    async fn existing_entries_keep_their_record_and_take_the_tables_words() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("linggen.json");
        let old = json!({ "app": "linggen", "quests": [
            { "id": "linggen-pair", "done_at": "2026-01-01T00:00:00.000Z", "reward": 7, "custom": 1,
              "device": "both", "title": { "zh": "结缘 · 与手机结契" }, "open": "/settings?tab=phone" },
            { "id": "linggen-voice", "done_at": "2026-01-01T00:00:00.000Z", "open": "/settings?tab=general", "ask": { "zh": "x" } },
            { "id": "somebody-else", "done_at": null }
        ]});
        std::fs::write(&path, old.to_string()).unwrap();

        // Everything is true now — the old stamp must stay.
        tick(&path, MILESTONES, &opens(), |_| ok(true)).await;
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let pair = entry(&doc, "linggen-pair").unwrap();
        assert_eq!(pair["done_at"], "2026-01-01T00:00:00.000Z");
        assert_eq!(pair["reward"], 7);
        assert_eq!(pair["custom"], 1);
        assert_eq!(pair["device"], "pair");
        assert_eq!(pair["title"]["zh"], "结缘 · 手机和 Mac 结契");
        let voice = entry(&doc, "linggen-voice").unwrap();
        assert_eq!(voice["open"], "/settings?tab=agents");
        assert!(voice.get("ask").is_none());
        assert!(
            entry(&doc, "somebody-else").is_some(),
            "unknown entries are kept"
        );
        assert_eq!(
            doc["quests"].as_array().unwrap().len(),
            MILESTONES.len() + 1
        );

        // A fact that goes false later stays done.
        let stamped = entry(&doc, "linggen-chat").unwrap()["done_at"].clone();
        let open = tick(&path, MILESTONES, &opens(), |_| ok(false)).await;
        assert_eq!(open, 0);
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(entry(&doc, "linggen-chat").unwrap()["done_at"], stamped);
    }

    #[tokio::test]
    async fn open_entry_gets_stamped_once_seen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("linggen.json");
        tick(&path, MILESTONES, &[], |_| ok(false)).await;
        assert!(entry(&load(&path).unwrap(), "linggen-voice").unwrap()["done_at"].is_null());
        tick(&path, MILESTONES, &[], |m| ok(m.id == "linggen-voice")).await;
        assert!(entry(&load(&path).unwrap(), "linggen-voice").unwrap()["done_at"].is_string());
    }

    #[tokio::test]
    async fn a_check_that_errors_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("linggen.json");
        let open = tick(&path, MILESTONES, &[], |m| {
            if m.id == "linggen-voice" {
                Box::pin(async { anyhow::bail!("config unreadable") })
            } else {
                ok(true)
            }
        })
        .await;
        assert_eq!(open, 1);
        let doc = load(&path).unwrap();
        assert!(entry(&doc, "linggen-voice").unwrap()["done_at"].is_null());
        assert!(entry(&doc, "linggen-pair").unwrap()["done_at"].is_string());
    }

    #[tokio::test]
    async fn an_unreadable_file_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("linggen.json");
        std::fs::write(&path, "{ half a file").unwrap();
        tick(&path, MILESTONES, &[], |_| ok(true)).await;
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ half a file");
    }

    #[tokio::test]
    async fn write_is_atomic_and_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("linggen.json");
        tick(&path, MILESTONES, &[], |_| ok(false)).await;
        tick(&path, MILESTONES, &[], |_| ok(true)).await;
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["linggen.json".to_string()]);
        assert!(load(&path).is_ok());
    }

    /// A phone fact stamps its milestone at the fact's own time, not the
    /// tick's; a done milestone stays as it was.
    #[test]
    fn a_fact_stamps_its_milestone_at_the_facts_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("linggen.json");
        assert_eq!(
            for_fact("yinyue-chat"),
            vec!["linggen-chat", "linggen-greet"]
        );
        assert_eq!(for_fact("dj-play"), vec!["dj-play"]);
        assert_eq!(for_fact("linggen-notify"), vec!["linggen-notify"]);
        assert!(for_fact("zz-other").is_empty());

        stamp_fact_in(&path, MILESTONES, "linggen-chat", "2026-09-20T08:00:00Z").unwrap();
        let doc = load(&path).unwrap();
        assert_eq!(
            entry(&doc, "linggen-chat").unwrap()["done_at"],
            "2026-09-20T08:00:00Z"
        );
        assert!(entry(&doc, "linggen-pair").unwrap()["done_at"].is_null());
        assert_eq!(doc["quests"].as_array().unwrap().len(), MILESTONES.len());

        stamp_fact_in(&path, MILESTONES, "linggen-chat", "2026-09-24T08:00:00Z").unwrap();
        let doc = load(&path).unwrap();
        assert_eq!(
            entry(&doc, "linggen-chat").unwrap()["done_at"],
            "2026-09-20T08:00:00Z",
            "set once"
        );
        assert!(stamp_fact_in(&path, MILESTONES, "nope", "2026-09-24T08:00:00Z").is_err());
    }

    #[test]
    fn sightings_are_remembered_by_what_they_name() {
        saw(Sighting::AppPage { app: "zz-test-app" });
        assert!(was_seen(Sighting::AppPage { app: "zz-test-app" }));
        assert!(!was_seen(Sighting::AppAudio { app: "zz-test-app" }));
        assert!(!was_seen(Sighting::AppPage {
            app: "zz-other-app"
        }));
    }

    #[test]
    fn events_become_sightings() {
        let msg = |from: &str, images: Vec<String>| ServerEvent::Message {
            from: from.into(),
            to: "yinyue".into(),
            content: "hi".into(),
            session_id: None,
            run_id: None,
            parent_agent_id: None,
            client_id: None,
            via: None,
            via_note: None,
            images,
        };
        assert_eq!(
            sighting_of(&msg("user", vec![])),
            vec![Sighting::UserMessage { agent: "yinyue" }]
        );
        assert_eq!(
            sighting_of(&msg("user", vec!["/api/sessions/images?x".into()])),
            vec![
                Sighting::UserMessage { agent: "yinyue" },
                Sighting::UserImage { agent: "yinyue" }
            ]
        );
        assert!(
            sighting_of(&msg("ling", vec![])).is_empty(),
            "an agent's line is not the person's"
        );
        let tool = ServerEvent::ContentBlockStart {
            agent_id: "yinyue".into(),
            block_id: "b".into(),
            block_type: "tool_use".into(),
            tool: Some("WebSearch".into()),
            args: None,
            parent_id: None,
            session_id: None,
            run_id: None,
            parent_run_id: None,
        };
        assert_eq!(
            sighting_of(&tool),
            vec![Sighting::ToolRan {
                agent: "yinyue",
                tool: "WebSearch"
            }]
        );
    }

    #[test]
    fn another_apps_row_ever_done() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("apple-shifu.json"),
            json!({ "app": "apple-shifu", "quests": [
                { "id": "shifu-scan", "done_at": "2026-11-02T01:40:43Z" },
                { "id": "shifu-backup", "done_at": null }
            ]})
            .to_string(),
        )
        .unwrap();
        assert!(quest_ever_done(dir.path(), "apple-shifu", "shifu-scan"));
        assert!(!quest_ever_done(dir.path(), "apple-shifu", "shifu-backup"));
        assert!(!quest_ever_done(dir.path(), "dj", "dj-fetch"));
    }

    #[test]
    fn a_recurring_row_takes_the_later_time_and_keeps_the_once_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("linggen.json");
        stamp_fact_in(&path, MILESTONES, "linggen-chat", "2026-09-20T08:00:00Z").unwrap();
        let doc = recur(
            &path,
            RECURRING,
            &[("linggen-greet", "2026-11-02T08:00:00.000Z")],
        )
        .unwrap();
        let q = entry(&doc, "linggen-greet").unwrap();
        assert_eq!(q["done_at"], "2026-11-02T08:00:00.000Z");
        assert_eq!(q["period"], "day");
        assert_eq!(q["kind"], "greet");
        assert_eq!(q["due"], true);
        assert!(q["how"]["zh"].as_str().is_some_and(|h| !h.is_empty()));
        // older never moves it back; newer moves it on
        let doc = recur(
            &path,
            RECURRING,
            &[("linggen-greet", "2026-11-01T08:00:00Z")],
        )
        .unwrap();
        assert_eq!(
            entry(&doc, "linggen-greet").unwrap()["done_at"],
            "2026-11-02T08:00:00.000Z"
        );
        let doc = recur(
            &path,
            RECURRING,
            &[("linggen-greet", "2026-11-03T07:00:00Z")],
        )
        .unwrap();
        assert_eq!(
            entry(&doc, "linggen-greet").unwrap()["done_at"],
            "2026-11-03T07:00:00Z"
        );
        assert_eq!(
            entry(&doc, "linggen-chat").unwrap()["done_at"],
            "2026-09-20T08:00:00Z"
        );
        // the milestones' tick leaves it as it is
        let doc = update(&path, MILESTONES, &[], &[]).unwrap();
        assert_eq!(
            entry(&doc, "linggen-greet").unwrap()["done_at"],
            "2026-11-03T07:00:00Z"
        );
        // nothing heard yet: a row, not done
        let other = dir.path().join("fresh.json");
        let doc = recur(&other, RECURRING, &[]).unwrap();
        assert!(entry(&doc, "linggen-greet").unwrap()["done_at"].is_null());
    }

    #[test]
    fn the_chat_fact_does_both_the_milestone_and_the_greeting() {
        let ids = for_fact("yinyue-chat");
        assert!(
            ids.contains(&"linggen-chat") && ids.contains(&"linggen-greet"),
            "{ids:?}"
        );
        assert!(RECURRING
            .iter()
            .all(|r| !MILESTONES.iter().any(|m| m.id == r.id)));
    }

    #[test]
    fn heard_keeps_the_latest() {
        heard("test-agent", "2026-11-02T08:00:00Z");
        heard("test-agent", "2026-11-01T08:00:00Z");
        assert_eq!(
            last_heard("test-agent").as_deref(),
            Some("2026-11-02T08:00:00Z")
        );
        assert_eq!(
            later(Some("junk"), Some("2026-11-02T08:00:00Z")),
            Some("2026-11-02T08:00:00Z")
        );
    }
}
