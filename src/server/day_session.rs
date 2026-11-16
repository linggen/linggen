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
use crate::state_fs::day_session;
use crate::state_fs::sessions::SessionMeta;

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
    if crate::state_fs::day_session::is_day_session(session_id) {
        crate::server::chat::members::seat(manager, session_id, COMPANION_AGENT_ID).await;
    }
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
}
