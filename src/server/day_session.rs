//! The day's session, as the server resolves it (`doc/one-ling-spec.md`
//! § Linggen's daily session). With `agent.daily_session` on, the chats the
//! person is in — an app's chat, Yinyue's own and unprompted turns, a
//! hand-off from the phone — are the daily session; each turn stands in the
//! place of the page it came from. Missions, delegated runs (`agent_run`)
//! and sessions an app makes for a one-off job keep sessions of their own.
//!
//! The id is the day's (`state_fs::day_session`), so it is found again after
//! a restart; it is made on first use.

use crate::engine::agent::{AgentManager, COMPANION_AGENT_ID};
use crate::server::person::Person;
use crate::state_fs::day_session;
use crate::state_fs::sessions::{ChatMsg, SessionMeta};

/// The agent that leads the day's session and answers what is not
/// addressed to Yinyue.
const LEAD: &str = "ling";

/// Whether the setting is on.
pub(crate) async fn enabled(manager: &AgentManager) -> bool {
    manager.get_config_snapshot().await.agent.daily_session
}

/// The day's session a chat from `app` (`None`: home — her own turns, the
/// main page, the Mac's own chat) of the person `user_id` names joins, made
/// if it is not there yet: their own day, the builder's when it names the
/// builder or no one. `None` while the setting is off, for an app that
/// keeps a session of its own ([`takes`]), and for a caller who is no one
/// yet. Every way into the day asks here; nothing else makes or names a
/// day's id (the lock test below).
pub(crate) async fn day_for(manager: &AgentManager, user_id: Option<&str>, app: Option<&str>) -> Option<String> {
    if !enabled(manager).await || !takes(manager, app).await {
        return None;
    }
    let person = Person::from_wire(user_id).day_key()?;
    Some(current(manager, person.as_deref()))
}

/// Whether a chat from `app` may be the day's: home's is; an app's only
/// when it resolves now and does not declare `private-talk: true`. A private
/// app keeps a session of its own, as a mission does; an app that does not
/// resolve (removed, its SKILL.md unreadable) stays apart too — when it is
/// not known to be shared, it is not.
pub(crate) async fn takes(manager: &AgentManager, app: Option<&str>) -> bool {
    let place = crate::engine::turn_place::TurnPlace::of_request(app, None);
    place.private_app(&*manager.skills, true).await.is_none()
}

/// The private app a person's words take up by name — `/name …`, or a
/// trigger prefix the app claims — with the setting on; `None` otherwise.
/// `words`: the message without the `@name` that addressed it, read as the
/// turn's dispatch reads it. In a day it is carried to that app's own chat.
pub(crate) async fn private_app_named(manager: &AgentManager, words: &str) -> Option<String> {
    if !enabled(manager).await {
        return None;
    }
    let name = match words.trim_start().starts_with('/') {
        true => crate::server::chat::slash_command(words).0.to_string(),
        false => manager.skills.match_trigger(words).await?.0,
    };
    let skill = manager.skills.get_skill(&name).await?;
    skill.private_talk.then_some(name)
}

/// The app's own chat to stand in when the person `user_id` names a day
/// their app does not join: the newest of their sessions of it that is not
/// one of its one-off jobs, else none — a session is then made for it.
pub(crate) fn own_chat(manager: &AgentManager, app: &str, user_id: Option<&str>) -> Option<String> {
    let asker = Person::from_wire(user_id);
    let mut sessions: Vec<SessionMeta> = manager.global_sessions.list_sessions().ok()?;
    sessions.retain(|m| m.skill.as_deref() == Some(app) && !kept_apart(m) && !is_day(&m.id));
    sessions.retain(|m| !asker.is_no_one() && Person::from_wire(m.user_id.as_deref()) == asker);
    sessions.into_iter().max_by_key(|m| m.created_at).map(|m| m.id)
}

/// Whether the person `user_id` names may read and write session
/// `session_id` — chat into it, read its state or images, steer, cancel or
/// clear its runs, answer its questions. The builder enters every session.
/// Anyone else: a day is its own person's alone, and any other session is
/// the person it was made for (its meta's `user_id`, [`Person::from_wire`]); a
/// session not kept yet is theirs to make. No one (a peer that named no
/// device) enters no kept session. The one rule every check point asks.
pub(crate) fn may_enter(session_id: &str, user_id: Option<&str>) -> bool {
    let store = crate::state_fs::SessionStore::with_sessions_dir(crate::paths::global_sessions_dir());
    may_enter_in(&store, session_id, user_id)
}

/// [`may_enter`] against the sessions kept in `store`.
fn may_enter_in(store: &crate::state_fs::SessionStore, session_id: &str, user_id: Option<&str>) -> bool {
    let asker = Person::from_wire(user_id);
    if let Some(day) = day_session::day_in(session_id) {
        let Some(person) = asker.day_key() else {
            return false;
        };
        return day_session::id_for(day, person.as_deref()) == session_id;
    }
    if asker.is_owner() {
        return true;
    }
    match kept_for(store, session_id) {
        Kept::No => true,
        Kept::For(whose) => !asker.is_no_one() && whose == asker,
        Kept::Unreadable => false,
    }
}

/// Whether a session is kept, and for whom.
enum Kept {
    No,
    For(Person),
    /// Named in a way no session is, or kept but not readable: no one's but
    /// the builder's.
    Unreadable,
}

fn kept_for(store: &crate::state_fs::SessionStore, session_id: &str) -> Kept {
    let plain = !session_id.is_empty()
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !plain {
        return Kept::Unreadable;
    }
    match store.get_session_meta(session_id) {
        Ok(None) => Kept::No,
        Ok(Some(meta)) => Kept::For(Person::from_wire(meta.user_id.as_deref())),
        Err(_) => Kept::Unreadable,
    }
}

/// Whether the person `user_id` names is the builder (no id is).
pub(crate) fn is_builder(user_id: Option<&str>) -> bool {
    Person::from_wire(user_id).is_owner()
}

/// Whether the caller `user_id` names is given what is the builder's alone —
/// an agent's notes, her own thread: with the setting on, the builder only
/// (a phone signed out is its own person, as memory reads it); with it off,
/// anyone, as before days.
pub(crate) async fn shares_builders(manager: &AgentManager, user_id: Option<&str>) -> bool {
    !enabled(manager).await || is_builder(user_id)
}

/// Whether session `session_id` is the builder's: their day, or — with the
/// setting on — any session not made for another person. With the setting
/// off every session is, as before days.
pub(crate) async fn is_builders(manager: &AgentManager, session_id: Option<&str>) -> bool {
    let Some(sid) = session_id else {
        return true;
    };
    if day_session::day_in(sid).is_some() {
        return may_enter(sid, None);
    }
    if !enabled(manager).await {
        return true;
    }
    let meta = manager.global_sessions.get_session_meta(sid).ok().flatten();
    Person::from_wire(meta.and_then(|m| m.user_id).as_deref()).is_owner()
}

/// Whose session `session_id` is, as its meta keeps it: a day its own
/// person's, any other session the person it was made for. `None` (her own
/// thread, what the engine starts itself) and a session not kept are the
/// builder's.
pub(crate) fn whose_session(manager: &AgentManager, session_id: Option<&str>) -> Person {
    let meta = session_id.and_then(|sid| manager.global_sessions.get_session_meta(sid).ok().flatten());
    Person::from_wire(meta.and_then(|m| m.user_id).as_deref())
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
    crate::engine::session_spec::is_kept_apart(meta)
}

/// Seat Yinyue in the day's session: she is seated when she first speaks
/// there.
pub(crate) async fn seat_her(manager: &AgentManager, session_id: &str) {
    if is_day(session_id) {
        crate::server::chat::members::seat(manager, session_id, COMPANION_AGENT_ID).await;
    }
}

/// The rows of session `session_id` a page may be shown, and — in a day's
/// session — its meta, for the page to show the place each row was written
/// in and fold other apps' talk. `asker`: the person the page's channel is
/// (`None`: the builder) — a day is shown only to its own person.
pub(crate) fn rows_for_page(
    manager: &AgentManager,
    session_id: &str,
    asker: Option<&str>,
) -> (Vec<ChatMsg>, Option<SessionMeta>) {
    if !may_enter(session_id, asker) {
        return (Vec::new(), None);
    }
    let store = &manager.global_sessions;
    let rows = store.get_chat_history(session_id).unwrap_or_default();
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

    /// A session is its person's: another person may not name it — nor no
    /// one, nor by an id no session has; a session not kept yet is theirs
    /// to make; the builder enters every one.
    #[test]
    fn a_session_is_its_persons() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::state_fs::SessionStore::with_sessions_dir(dir.path().into());
        let made_for = |id: &str, user: Option<&str>| {
            let mut meta = crate::server::claude::new_meta("t".into(), "/p".into(), None);
            meta.id = id.into();
            meta.user_id = user.map(str::to_string);
            store.add_session(&meta).unwrap();
        };
        made_for("sess-1-builder", None);
        made_for("sess-2-friend", Some("u-friend"));
        made_for("sess-3-local", Some("__local__"));
        let enters = |sid: &str, who: Option<&str>| may_enter_in(&store, sid, who);
        assert!(!enters("sess-1-builder", Some("u-friend")));
        assert!(!enters("sess-3-local", Some("u-friend")));
        assert!(enters("sess-2-friend", Some("u-friend")));
        assert!(!enters("sess-2-friend", Some("u-other")));
        assert!(!enters("sess-2-friend", Some(crate::server::rtc::NO_ONE)));
        assert!(enters("sess-9-new", Some("u-friend")), "a session not kept yet");
        assert!(!enters("../sess-2-friend", Some("u-friend")));
        assert!(!enters("", Some("u-friend")), "no session named: a run of no session");
        for sid in ["sess-1-builder", "sess-2-friend", "sess-3-local", ""] {
            assert!(enters(sid, None), "the builder: {sid}");
            assert!(enters(sid, Some("__local__")), "this machine: {sid}");
        }
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

    /// The only files outside this module that make a day's meta.
    const MAKES_A_DAY: &[(&str, usize, &str)] = &[(
        "crates/linggen-engine/src/engine/session_spec/tests.rs",
        2,
        "fixtures: the daily preset's row in the spec's table test",
    )];

    #[test]
    fn every_way_into_the_day_asks_the_one_rule() {
        check(&[concat!("day_", "for(")], WAYS_IN, "comes into the day");
        let own_ids = [
            concat!("day_session::", "id_for("),
            concat!("day_session::", "new_meta("),
            concat!("day_session::", "today("),
        ];
        check(&own_ids, MAKES_A_DAY, "names or makes a day's id itself");
    }
}
