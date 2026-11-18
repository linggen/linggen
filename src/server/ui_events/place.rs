//! The place a day's running turn stands in, on the events it sends to a
//! page (`doc/one-ling-spec.md` § Linggen's daily session): a page at another
//! app then says whose work it shows. Told by `turn_place::told_running`;
//! outside a day no event names a place.

use crate::engine::events::UiEvent;

/// The kinds of event a running turn sends that a page reads its place
/// from: its status (the status line and Stop), and what it streams — its
/// text, blocks, tool output and messages, the person's that started it
/// included — so another place's turn folds while it streams.
const TOLD_KINDS: &[&str] = &[
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
