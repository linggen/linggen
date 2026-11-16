//! The day's session, as the server resolves it (`doc/one-ling-spec.md`
//! § One session a day). With `agent.one_session_a_day` on, the chats the
//! person is in — an app's chat, Yinyue's own and unprompted turns, a
//! hand-off from the phone — are one session a day; each turn stands in the
//! place of the page it came from. Missions, delegated runs (`agent_run`)
//! and sessions an app makes for a one-off job keep sessions of their own.
//!
//! The id is the day's (`state_fs::day_session`), so it is found again after
//! a restart; it is made on first use.

use crate::engine::agent::AgentManager;
use crate::state_fs::day_session;

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
