//! Clear in the day's session takes the rows of the place it is pressed in —
//! an app's page its app's, the main page home's — and every other place's
//! stay (`doc/one-ling-spec.md` § Linggen's daily session). Anywhere else Clear
//! empties the session, as it always did.

use super::types::ClearChatRequest;
use crate::server::{ServerEvent, ServerState};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;

/// POST /api/chat/clear.
pub(crate) async fn clear_chat_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ClearChatRequest>,
) -> axum::response::Response {
    let session_id = req
        .session_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    match app_part(&session_id, req.skill_name.as_deref()) {
        Some(app) => clear_place(&state, &session_id, &app).await,
        None => super::admin::clear_chat_history_api(State(state), Json(req))
            .await
            .into_response(),
    }
}

/// The place whose part of the day a page of `skill` clears: its app's, or
/// home's for a page of none, in a day's session. `None`: the whole
/// session is cleared.
fn app_part(session_id: &str, skill: Option<&str>) -> Option<String> {
    if !crate::server::day_session::is_day(session_id) {
        return None;
    }
    let skill = skill.map(str::trim).filter(|s| !s.is_empty());
    Some(skill.unwrap_or(crate::engine::turn_place::HOME_KEY).to_string())
}

/// Remove the rows written in `app`'s place from the day's session; every
/// engine of the session reads its thread again before its next turn.
async fn clear_place(
    state: &Arc<ServerState>,
    session_id: &str,
    app: &str,
) -> axum::response::Response {
    let store = &state.manager.global_sessions;
    let Ok(Some(meta)) = store.get_session_meta(session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let mut removed = 0;
    let edited = store.edit_chat_history(session_id, |rows| {
        let before = rows.len();
        let kept = without_place(rows, &meta, app);
        removed = before - kept.len();
        kept
    });
    if let Err(e) = edited {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    for engine in state.manager.session_engines_of(session_id).await {
        super::thread::reload_next(&mut *engine.lock().await);
    }
    let _ = state.events_tx.send(ServerEvent::StateUpdated);
    Json(serde_json::json!({ "removed": removed })).into_response()
}

/// `rows` without those written in `app`'s place of session `meta`; the
/// compaction summary is the day's and stays.
fn without_place(
    rows: Vec<crate::state_fs::sessions::ChatMsg>,
    meta: &crate::state_fs::sessions::SessionMeta,
    app: &str,
) -> Vec<crate::state_fs::sessions::ChatMsg> {
    rows.into_iter()
        .filter(|r| r.place_of(meta).as_deref() != Some(app))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_fs::sessions::{ChatMsg, SessionMeta};

    fn row(place: Option<&str>) -> ChatMsg {
        ChatMsg {
            agent_id: "ling".into(),
            from_id: "user".into(),
            to_id: "ling".into(),
            content: "x".into(),
            timestamp: 1,
            is_observation: false,
            client_id: None,
            failed: false,
            via: None,
            via_note: None,
            images: Vec::new(),
            run_id: None,
            place: place.map(str::to_string),
            heard: false,
        }
    }

    /// Clear in the day takes the rows of the place it is pressed in; in any
    /// other session the whole session goes.
    #[test]
    fn clear_on_an_apps_page_takes_only_its_part_of_the_day() {
        let day = SessionMeta {
            id: "sess-day-2026-11-16".into(),
            ..Default::default()
        };
        let rows = vec![
            row(Some("zz-ledger")),
            row(Some("zz-dice")),
            row(None),
            row(Some(":home")),
        ];
        let left = without_place(rows, &day, "zz-ledger");
        let places: Vec<_> = left.iter().map(|r| r.place_in(&day)).collect();
        assert_eq!(places, ["zz-dice", ":home", ":home"]);
        assert_eq!(
            app_part("sess-day-2026-11-16", Some("zz-ledger")).as_deref(),
            Some("zz-ledger")
        );
        assert_eq!(app_part("sess-day-2026-11-16", None).as_deref(), Some(":home"));
        let home_left = without_place(vec![row(Some("zz-ledger")), row(None), row(Some(":home"))], &day, ":home");
        assert_eq!(home_left.len(), 1, "home's Clear leaves the apps' rows");
        assert_eq!(app_part("sess-1-zz-ledger", Some("zz-ledger")), None);
    }
}
