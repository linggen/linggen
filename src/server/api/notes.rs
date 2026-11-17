//! GET /api/notes?agent= — an agent's open notes, read-only, for another
//! device (the phone shows them; `doc/one-ling-spec.md` § One session a
//! day). Notes are written by their agent's own tools alone; nothing here
//! changes them. A private app's turn writes none (`tools::exit_gate`),
//! so nothing of its talk is in them. They are the builder's: another
//! person is given none (the channel and the LAN gate name the person).

use axum::extract::Query;
use axum::Json;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct NotesQuery {
    agent: String,
    #[serde(default)]
    user_id: Option<String>,
}

/// `{agent, notes: [{id, text, written_at, device, look_again_at?}]}`;
/// an agent id that is not one, and a person other than the builder, get
/// no notes.
pub(crate) async fn list_notes(Query(q): Query<NotesQuery>) -> Json<serde_json::Value> {
    let agent = q.agent.trim();
    if !is_agent_id(agent) || !crate::server::day_session::is_builder(q.user_id.as_deref()) {
        return Json(serde_json::json!({ "agent": agent, "notes": [] }));
    }
    let book = crate::notes::read(agent);
    Json(serde_json::json!({ "agent": agent, "notes": book.notes() }))
}

/// An agent id as agents are named: lowercase letters, digits, `-`, `_`.
fn is_agent_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::is_agent_id;

    #[test]
    fn only_an_agent_id_is_read() {
        assert!(is_agent_id("yinyue") && is_agent_id("ling"));
        for bad in ["", "../x", "Yinyue", "a/b", "x y"] {
            assert!(!is_agent_id(bad), "{bad}");
        }
    }
}
