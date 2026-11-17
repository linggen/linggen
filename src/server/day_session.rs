//! The day's session, as the server resolves it (`doc/one-ling-spec.md`
//! § One session a day). With `agent.one_session_a_day` on, the chats the
//! person is in — an app's chat, Yinyue's own and unprompted turns, a
//! hand-off from the phone — are one session a day; each turn stands in the
//! place of the page it came from. Missions, delegated runs (`agent_run`)
//! and sessions an app makes for a one-off job keep sessions of their own.
//!
//! The id is the day's (`state_fs::day_session`), so it is found again after
//! a restart; it is made on first use.

use crate::engine::agent::{AgentManager, COMPANION_AGENT_ID};
use crate::engine::turn_place::sight::Sight;
use crate::state_fs::day_session;
use crate::state_fs::sessions::{ChatMsg, SessionMeta};

/// The agent that leads the day's session and answers what is not
/// addressed to Yinyue.
const LEAD: &str = "ling";

/// Whether the setting is on.
pub(crate) async fn enabled(manager: &AgentManager) -> bool {
    manager.get_config_snapshot().await.agent.one_session_a_day
}

/// The day's session a chat from `app` (`None`: home — her own turns, the
/// main page, the Mac's own chat) of the person `user_id` names joins, made
/// if it is not there yet: their own day, the builder's when it names the
/// builder or no one. `None` while the setting is off, and for an app that
/// keeps a session of its own ([`takes`]). Every way into the day asks
/// here; nothing else makes or names a day's id (the lock test below).
pub(crate) async fn day_for(manager: &AgentManager, user_id: Option<&str>, app: Option<&str>) -> Option<String> {
    if !enabled(manager).await || !takes(manager, app).await {
        return None;
    }
    Some(current(manager, person_of(user_id).as_deref()))
}

/// Whether a chat from `app` may be the day's: home's is; an app's only
/// when it resolves now and does not declare `private-talk: true`. A private
/// app keeps a session of its own, as a mission does; an app that does not
/// resolve (removed, its SKILL.md unreadable) stays apart too — when it is
/// not known to be shared, it is not.
pub(crate) async fn takes(manager: &AgentManager, app: Option<&str>) -> bool {
    let Some(name) = app else {
        return true;
    };
    manager.skills.get_skill(name).await.is_some_and(|s| !s.private_talk)
}

/// The app's own chat to stand in when a request names a day its app does
/// not join: its newest session that is not one of its one-off jobs, else
/// none — a session is then made for it.
pub(crate) fn own_chat(manager: &AgentManager, app: &str) -> Option<String> {
    let mut sessions: Vec<SessionMeta> = manager.global_sessions.list_sessions().ok()?;
    sessions.retain(|m| m.skill.as_deref() == Some(app) && !kept_apart(m) && !is_day(&m.id));
    sessions.into_iter().max_by_key(|m| m.created_at).map(|m| m.id)
}

/// Whether the person `user_id` names may read and write session
/// `session_id`: a day is its own person's alone; any other session is
/// left as it is.
pub(crate) fn may_enter(session_id: &str, user_id: Option<&str>) -> bool {
    let Some(day) = day_session::day_in(session_id) else {
        return true;
    };
    day_session::id_for(day, person_of(user_id).as_deref()) == session_id
}

/// Whose day a request's `user_id` is: `None` for the builder — no id, this
/// machine's own, a paired device not yet named, or the builder's account.
fn person_of(user_id: Option<&str>) -> Option<String> {
    let id = user_id.map(str::trim).filter(|id| !id.is_empty())?;
    if matches!(id, "__local__" | "__paired__") {
        return None;
    }
    let builders = crate::account::load_account().and_then(|a| a.user_id);
    (builders.as_deref() != Some(id)).then(|| id.to_string())
}

/// Today's session id for `person` (`state_fs::day_session::today`), made
/// if missing. It is made once: a kept day is never written over, even when
/// it cannot be read now or another request made it a moment ago.
fn current(manager: &AgentManager, person: Option<&str>) -> String {
    let day = day_session::today();
    let sid = day_session::id_for(day, person);
    let cwd = crate::util::resolve_path(&crate::paths::ensure_default_workspace());
    let meta = day_session::new_meta(&sid, day, &cwd.to_string_lossy(), (LEAD, person));
    if let Err(e) = manager.global_sessions.add_session_if_new(&meta) {
        tracing::warn!("[day] could not make {sid}: {e}");
    }
    sid
}

/// Whether a request for a new session of `skill` is an app's chat, which
/// the day's session takes: not a session bound to its own values (a test
/// save), not one made with tools withheld (an app's one-off job — an NPC's
/// talk, a prefetch), not a consumer's or another runtime's.
pub(crate) fn takes_new_chat(
    skill: Option<&str>,
    bound: bool,
    withheld: bool,
    owner_linggen: bool,
) -> bool {
    skill.is_some() && !bound && !withheld && owner_linggen
}

/// Whether `meta` is a session an app keeps apart from the day's — bound
/// to its own values, or made with tools withheld.
pub(crate) fn kept_apart(meta: &SessionMeta) -> bool {
    !meta.params.is_empty() || !meta.withheld_tools.is_empty()
}

/// Seat Yinyue in the day's session: she is seated when she first speaks
/// there.
pub(crate) async fn seat_her(manager: &AgentManager, session_id: &str) {
    if is_day(session_id) {
        crate::server::chat::members::seat(manager, session_id, COMPANION_AGENT_ID).await;
    }
}

/// The rows of session `session_id` a page of `page` (an app's, or `None`
/// for the main page and the phone) may be shown, and — in a day's session
/// — the place each was written in, for the page to fold other apps' talk.
/// A private app's rows are shown only on its own pages
/// (`turn_place::sight`).
/// `asker`: the person the page's channel is (`None`: the builder) — a day
/// is shown only to its own person.
pub(crate) async fn rows_for_page(
    manager: &AgentManager,
    session_id: &str,
    (page, asker): (Option<&str>, Option<&str>),
) -> (Vec<ChatMsg>, Option<SessionMeta>) {
    if !may_enter(session_id, asker) {
        return (Vec::new(), None);
    }
    let store = &manager.global_sessions;
    let reader = page.unwrap_or(crate::engine::turn_place::HOME_KEY);
    let rows = store.rows_seen_by(session_id, &Sight::page(reader)).unwrap_or_default();
    let Ok(Some(meta)) = store.get_session_meta(session_id) else {
        return (rows, None);
    };
    let shown = is_day(session_id).then_some(meta);
    (rows, shown)
}

/// A history row's meta with the place it was written in, for a day's
/// session (`meta` is that session's; `None` elsewhere: no place is shown).
pub(crate) fn with_place(
    mut row_meta: serde_json::Value,
    row: &ChatMsg,
    meta: Option<&SessionMeta>,
) -> serde_json::Value {
    if let Some(meta) = meta {
        row_meta["place"] = serde_json::Value::from(row.place_in(meta));
    }
    row_meta
}

/// Whether `session_id` is a day's session.
pub(crate) fn is_day(session_id: &str) -> bool {
    day_session::is_day_session(session_id)
}

/// Who asks for a day by URL: the person on the page's channel
/// (`rtc::peer::control` sets it); none is the builder.
#[derive(serde::Deserialize)]
pub(crate) struct Asker {
    #[serde(default)]
    pub(crate) user_id: Option<String>,
    /// The app the hand-off is for: an app that keeps a session of its own
    /// is given no day.
    #[serde(default)]
    pub(crate) skill: Option<String>,
}

/// GET /api/sessions/day[?skill=] — the day's session for a surface that
/// joins it (the phone's hand-off): `{on, id, cwd}`; `id` is null while the
/// setting is off, and for an app that keeps a session of its own.
pub(crate) async fn day_session_api(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::server::ServerState>>,
    axum::extract::Query(asker): axum::extract::Query<Asker>,
) -> axum::Json<serde_json::Value> {
    let on = enabled(&state.manager).await;
    let Some(id) = day_for(&state.manager, asker.user_id.as_deref(), asker.skill.as_deref()).await else {
        return axum::Json(serde_json::json!({ "on": on, "id": null }));
    };
    let cwd = state
        .manager
        .global_sessions
        .get_session_meta(&id)
        .ok()
        .flatten()
        .map(|m| m.cwd);
    axum::Json(serde_json::json!({ "on": true, "id": id, "cwd": cwd }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_apps_chat_joins_and_its_one_off_jobs_stay_apart() {
        assert!(takes_new_chat(Some("cfo"), false, false, true));
        assert!(
            !takes_new_chat(None, false, false, true),
            "a New chat stays apart"
        );
        assert!(
            !takes_new_chat(Some("lingjing"), true, false, true),
            "a test save"
        );
        assert!(
            !takes_new_chat(Some("lingjing"), false, true, true),
            "an NPC talk"
        );
        assert!(
            !takes_new_chat(Some("cfo"), false, false, false),
            "a consumer"
        );
    }

    #[test]
    fn a_session_bound_or_withholding_is_kept_apart() {
        let mut meta = SessionMeta::default();
        assert!(!kept_apart(&meta));
        meta.withheld_tools = vec!["mcp__memory".into()];
        assert!(kept_apart(&meta));
        let mut bound = SessionMeta::default();
        bound.params.insert("save".into(), "test".into());
        assert!(kept_apart(&bound));
    }

    /// Every place that reads a session's rows outside `rows_seen_by`, and
    /// why it may: each one that hands rows to a model, to Yinyue or to a
    /// page reads them through `rows_seen_by` (`turn_place::sight`). A new
    /// reader fails here until it is looked at and listed.
    const ROW_READERS: &[(&str, usize, &str)] = &[
        ("src/server/chat/thread.rs", 2, "how far the thread is read: counts and senders, no words"),
        ("src/server/chat/compact_rows.rs", 1, "the summary's input: filtered by the compacting place's Sight"),
        ("crates/linggen-engine/src/engine/tools/mod.rs", 1, "a skill command's user words when no place is kept"),
        ("src/server/resident/session_roll.rs", 1, "only whether her thread is empty"),
        ("src/server/mcp_agent.rs", 1, "a delegated run's reply; refuses the day's session"),
        ("src/server/mcp.rs", 1, "a dream mission run's last line"),
        ("src/server/api/missions.rs", 1, "a mission run's own session"),
        ("crates/linggen-engine/src/extensions/missions/scheduler.rs", 1, "a mission run's report"),
        ("src/server/chat/plan_flow.rs", 2, "the plan the page approves; the plan's own stamp"),
        ("src/server/restart.rs", 1, "whether a continue turn was told: a client id"),
        ("crates/linggen-engine/src/engine/agent/met.rs", 1, "whether an agent ever spoke: no words"),
        ("crates/linggen-engine/src/state_fs/sessions.rs", 0, "the store itself (counted apart)"),
        ("src/server/chat/helpers.rs", 2, "tests"),
        ("crates/linggen-engine/src/state_fs/row_place_tests.rs", 2, "tests"),
        ("crates/linggen-engine/src/engine/light_turn/tests.rs", 1, "tests"),
    ];

    /// Every place that rewrites rows already kept, and why each row keeps
    /// its stamp: a rewrite never drops a row's place or `kept_to`.
    const ROW_REWRITERS: &[(&str, usize, &str)] = &[
        ("crates/linggen-engine/src/engine/context.rs", 1, "in-loop compaction: rows kept whole, the summary stamped"),
        ("src/server/chat/compact_rows.rs", 1, "compaction: rows kept whole, the summary stamped"),
        ("src/server/chat/clear_place.rs", 1, "a clear of one app's part: rows dropped, never changed"),
        ("crates/linggen-engine/src/state_fs/row_place_tests.rs", 1, "tests"),
    ];

    /// Every listener of the event bus (`EventBus::listen`, its one way in),
    /// and what it does with a kept event.
    const BUS_LISTENERS: &[(&str, usize, &str)] = &[
        ("src/server/rtc/peer/mod.rs", 1, "the pages: forward judges the stamp"),
        ("src/server/rtc/proxy_client.rs", 1, "room chat only: no session's words"),
        ("src/server/resident/triggers.rs", 1, "her heralds: a kept question is told only as waiting"),
        ("src/server/tasks.rs", 1, "the task list: entries carry their own stamp"),
        ("src/server/push/mod.rs", 1, "a mission's end: no session's words"),
        ("src/server/milestones.rs", 1, "kinds and times, no words"),
        ("src/server/event_log.rs", 1, "the engine's own local log"),
        ("src/server/mcp_agent.rs", 2, "a delegated run's end: refuses the day's session"),
        ("src/server/quests_watch.rs", 1, "tests"),
        ("src/server/chat/helpers.rs", 2, "tests"),
        ("crates/linggen-engine/src/engine/event_bus.rs", 2, "the bus's own tests"),
    ];

    /// Every event sent with a stamp chosen by hand, and why it is right.
    const HAND_STAMPS: &[(&str, usize, &str)] = &[
        ("src/server/api/yinyue.rs", 1, "her line: the place it was said in"),
        ("src/server/chat/plan_flow.rs", 2, "a plan acted on later: the plan row's stamp"),
    ];

    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// Each file's count of `patterns`, against the list it must match.
    fn check(patterns: &[&str], listed: &[(&str, usize, &str)], what: &str) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        for dir in ["src", "crates/linggen-engine/src"] {
            rust_files(&root.join(dir), &mut files);
        }
        for file in files {
            let rel = file.strip_prefix(root).unwrap().to_string_lossy().to_string();
            if rel == "src/server/day_session.rs" || rel == "crates/linggen-engine/src/state_fs/sessions.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            let found: usize = patterns.iter().map(|p| text.matches(p).count()).sum();
            let want = listed.iter().find(|(f, _, _)| *f == rel).map_or(0, |e| e.1);
            assert_eq!(found, want, "{rel} {what}: look at it, then list it");
        }
    }

    /// Every way into the day, each asking the one rule (`day_for`, which
    /// asks [`takes`]): a new way in fails here until it is looked at and
    /// listed, and nothing outside this module names or makes a day's id.
    const WAYS_IN: &[(&str, usize, &str)] = &[
        ("src/server/chat/handler.rs", 2, "a chat with no session yet, or naming another person's day"),
        ("src/server/api/sessions.rs", 2, "POST /api/sessions, and an app's list"),
        ("src/server/yinyue_moments.rs", 1, "her moments, in the app they came from"),
        ("src/server/resident/triggers.rs", 1, "a message delivered to an agent's chat"),
        ("src/server/resident/session_roll.rs", 1, "her own thread: home"),
    ];

    #[test]
    fn every_way_into_the_day_asks_the_one_rule() {
        check(&[concat!("day_", "for(")], WAYS_IN, "comes into the day");
        let own_ids = [
            concat!("day_session::", "id_for("),
            concat!("day_session::", "new_meta("),
            concat!("day_session::", "today("),
        ];
        check(&own_ids, &[], "names or makes a day's id itself");
    }

    #[test]
    fn every_reader_of_a_sessions_rows_is_known() {
        let readers = [concat!("get_chat_history", "("), concat!("rows_since", "(")];
        check(&readers, ROW_READERS, "reads a session's rows");
    }

    #[test]
    fn every_rewrite_of_kept_rows_is_known() {
        let writers = [
            concat!("edit_chat_history", "("),
            concat!("rewrite_chat_history", "("),
        ];
        check(&writers, ROW_REWRITERS, "rewrites kept rows");
    }

    /// The bus is the only way an event goes out (`engine::event_bus`):
    /// every listener is listed, and no event is stamped by hand unlisted.
    #[test]
    fn every_listener_and_hand_stamp_of_the_bus_is_known() {
        check(&[concat!(".lis", "ten()")], BUS_LISTENERS, "listens on the bus");
        check(&[concat!(".send", "_kept(")], HAND_STAMPS, "stamps an event by hand");
        check(&[concat!("Sender<", "ServerEvent>")], &[], "sends events past the bus");
    }
}
