//! The place a day's running turn stands in, on the events it sends to a
//! page (`doc/one-ling-spec.md` § Linggen's daily session): a page at another
//! app then says whose work it shows. Told by `turn_place::told_running`;
//! outside a day no event names a place.

use crate::engine::events::UiEvent;

/// The kinds of event a running turn sends that a page reads its place
/// from: its status (the status line and Stop), and what it streams — its
/// text, blocks, tool output and messages, the person's that started it
/// included — so another place's turn folds while it streams; and the
/// question it asks, so a card at another page names whose it is.
const TOLD_KINDS: &[&str] = &[
    "ask_user",
    "activity",
    "run",
    "turn_complete",
    "token",
    "text_segment",
    "content_block",
    "tool_progress",
    "message",
];

/// `ui` with the place of the turn running in its session, when it is one
/// of [`TOLD_KINDS`] and a place is told there.
pub(crate) fn with_place(mut ui: UiEvent, manager: &crate::engine::agent::AgentManager) -> UiEvent {
    if !TOLD_KINDS.contains(&ui.kind.as_str()) {
        return ui;
    }
    let Some(sid) = ui.session_id.as_deref() else {
        return ui;
    };
    let places = &manager.global_sessions.running_places;
    ui.place = crate::engine::turn_place::told_running(places, sid, manager.daily_session());
    ui
}

/// One open question as a page restores it (`pending_ask_user` on the page
/// state, `GET /api/pending-ask-user`), with the place of the turn that asked
/// it when one is told there; no `place` key anywhere else.
pub(crate) fn pending_ask(
    question_id: &str,
    entry: &crate::engine::tools::PendingAskUser,
    manager: &crate::engine::agent::AgentManager,
) -> serde_json::Value {
    let mut item = serde_json::json!({
        "question_id": question_id,
        "agent_id": entry.agent_id,
        "questions": entry.questions,
        "session_id": entry.session_id,
    });
    let places = &manager.global_sessions.running_places;
    let day_on = manager.daily_session();
    let told = entry
        .session_id
        .as_deref()
        .and_then(|sid| crate::engine::turn_place::told_running(places, sid, day_on));
    if let Some(place) = told {
        item["place"] = serde_json::json!(place);
    }
    // Said aloud while the person was away: the card carries the mark.
    if let Some(told) = crate::server::resident::wait_told(question_id) {
        item["told_aloud_at"] = told.at.into();
        item["told_aloud_line"] = told.line.into();
    }
    item
}
