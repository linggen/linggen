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

/// The day's session id when the setting is on — made if it is not there
/// yet; `None` when it is off.
pub(crate) async fn current_if_on(manager: &AgentManager) -> Option<String> {
    if !enabled(manager).await {
        return None;
    }
    Some(current(manager))
}

/// Today's session id (`state_fs::day_session::today`), made if missing.
pub(crate) fn current(manager: &AgentManager) -> String {
    let day = day_session::today();
    let sid = day_session::id_of(day);
    if let Ok(Some(_)) = manager.global_sessions.get_session_meta(&sid) {
        return sid;
    }
    let cwd = crate::util::resolve_path(&crate::paths::ensure_default_workspace());
    let meta = day_session::new_meta(&sid, day, &cwd.to_string_lossy(), LEAD);
    if let Err(e) = manager.global_sessions.add_session(&meta) {
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
pub(crate) async fn rows_for_page(
    manager: &AgentManager,
    session_id: &str,
    page: Option<&str>,
) -> (Vec<ChatMsg>, Option<SessionMeta>) {
    let store = &manager.global_sessions;
    let rows = store.get_chat_history(session_id).unwrap_or_default();
    let Ok(Some(meta)) = store.get_session_meta(session_id) else {
        return (rows, None);
    };
    let reader = page.unwrap_or(crate::engine::turn_place::HOME_KEY);
    let private = crate::engine::turn_place::sight::private_apps(&*manager.skills).await;
    let rows = Sight::new(reader, private).visible(rows, &meta);
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

/// GET /api/sessions/day — the day's session for a surface that joins it
/// (the phone's hand-off): `{on, id, cwd}`; `id` is null while the setting
/// is off.
pub(crate) async fn day_session_api(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::server::ServerState>>,
) -> axum::Json<serde_json::Value> {
    let Some(id) = current_if_on(&state.manager).await else {
        return axum::Json(serde_json::json!({ "on": false, "id": null }));
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

    /// Every place that reads a session's rows, and why it may (each one
    /// that hands rows to a model, to Yinyue or to a page goes through
    /// `turn_place::sight`). A new reader fails here until it is looked at
    /// and listed: a private app's talk must not reach another place by it.
    const ROW_READERS: &[(&str, usize, &str)] = &[
        (
            "src/server/chat/thread.rs",
            4,
            "a member's thread and the light turn's tail: seen_rows",
        ),
        (
            "src/server/chat/compact_rows.rs",
            1,
            "the summary's input: Sight of the compacting place",
        ),
        (
            "src/server/day_session.rs",
            1,
            "rows_for_page: history views, her herald quote and seed",
        ),
        (
            "crates/linggen-engine/src/engine/tools/mod.rs",
            1,
            "a skill command's user words: turn_sight",
        ),
        (
            "src/server/resident/session_roll.rs",
            1,
            "only whether her thread is empty",
        ),
        (
            "src/server/mcp_agent.rs",
            1,
            "a delegated run's reply; refuses the day's session",
        ),
        ("src/server/mcp.rs", 1, "a dream mission run's last line"),
        (
            "src/server/api/missions.rs",
            1,
            "a mission run's own session",
        ),
        (
            "crates/linggen-engine/src/extensions/missions/scheduler.rs",
            1,
            "a mission run's report",
        ),
        (
            "src/server/chat/plan_flow.rs",
            2,
            "the plan the page approves; the place it was made in, where it runs",
        ),
        (
            "src/server/restart.rs",
            1,
            "whether a continue turn was told: a client id",
        ),
        (
            "crates/linggen-engine/src/engine/agent/met.rs",
            1,
            "whether an agent ever spoke: no words",
        ),
        (
            "crates/linggen-engine/src/state_fs/sessions.rs",
            9,
            "the store itself",
        ),
        ("src/server/chat/helpers.rs", 2, "tests"),
        (
            "crates/linggen-engine/src/state_fs/row_place_tests.rs",
            2,
            "tests",
        ),
        (
            "crates/linggen-engine/src/engine/light_turn/tests.rs",
            1,
            "tests",
        ),
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

    #[test]
    fn every_reader_of_a_sessions_rows_is_known() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        for dir in ["src", "crates/linggen-engine/src"] {
            rust_files(&root.join(dir), &mut files);
        }
        for file in files {
            let rel = file
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .to_string();
            let text = std::fs::read_to_string(&file).unwrap();
            let found = text.matches(concat!("get_chat_history", "(")).count()
                + text.matches(concat!("rows_since", "(")).count();
            let listed = ROW_READERS
                .iter()
                .find(|(f, _, _)| *f == rel)
                .map_or(0, |e| e.1);
            assert_eq!(
                found, listed,
                "{rel} reads a session's rows: look at it, then list it"
            );
        }
    }
}
