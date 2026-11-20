//! A Claude session from 2.5.1 and before: one the Claude Agent SDK hosted
//! (`SessionRuntime::ClaudeAgentSdk`). That runtime is gone, so the session
//! is a read-only record — it opens, renames, clears and deletes as any
//! session does, takes no turn, and offers one way on: a new Linggen session
//! in the same folder that starts with its conversation
//! (`doc/session-spec.md` § Old Claude sessions).
//!
//! This module holds the reason every door gives and the continue. The one
//! check every native turn passes is `refuse_foreign_runtime`
//! (`engine/agent/mod.rs`).

use crate::server::ServerState;
use crate::state_fs::sessions::{ChatMsg, SessionMeta, COMPACTION_SENDER};
use crate::state_fs::RowAuthor;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Deserialize;
use std::sync::Arc;

/// What a door that refuses an old Claude session says (the page says its own line).
pub(crate) const READ_ONLY: &str =
    "This Claude session is read-only. Continue it in a new Linggen session.";

/// How much of the old conversation the new session starts with: its newest
/// lines, whole, up to this many characters (about ten thousand tokens).
const CARRY_CHARS: usize = 40_000;
/// A single line longer than this is cut, so one pasted log cannot fill the carry.
const LINE_CHARS: usize = 4_000;

/// True for a session this engine keeps only as a read-only record.
pub(crate) fn is_read_only(meta: &SessionMeta) -> bool {
    !meta.runtime.is_linggen()
}

#[derive(Deserialize)]
pub(crate) struct ContinueRequest {
    session_id: String,
}

/// POST /api/sessions/continue — a new Linggen session in the old Claude
/// session's folder, with the lead's model the old session used (a Claude
/// model, run natively), whose first row carries the old conversation
/// ([`carried`]). The old session is left as it was. Returns `{id, cwd}`.
pub(crate) async fn continue_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ContinueRequest>,
) -> impl IntoResponse {
    let store = &state.manager.global_sessions;
    let Ok(Some(old)) = store.get_session_meta(&req.session_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !is_read_only(&old) {
        return (
            StatusCode::BAD_REQUEST,
            "Only an old Claude session is continued this way.",
        )
            .into_response();
    }
    let rows = store.get_chat_history(&old.id).unwrap_or_default();
    // The model the old session pinned (a Claude model) carries over when
    // this engine runs it; else the new session takes the default chain.
    let pinned = old.agents.first().and_then(|m| m.model.clone());
    let known = match pinned {
        Some(id)
            if crate::engine::model_choice::is_here(&*state.manager.models.read().await, &id) =>
        {
            Some(id)
        }
        _ => None,
    };
    let meta = continued_meta(&old, known);
    let seed = seed_row(&meta, carried(&old.title, &rows));
    if store.add_session(&meta).is_err() || store.add_chat_message(&meta.id, &seed).is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    Json(serde_json::json!({ "id": meta.id, "cwd": meta.cwd })).into_response()
}

/// The new session: the old one's folder and person, an ordinary chat's
/// lead (`members::default_members`) on `model` (none: the default chain).
/// Built like `api::sessions::create_session` builds a plain chat.
fn continued_meta(old: &SessionMeta, model: Option<String>) -> SessionMeta {
    let mut agents = crate::server::chat::members::default_members(None);
    if let Some(lead) = agents.first_mut() {
        lead.model = model;
    }
    let mut meta = SessionMeta {
        id: format!(
            "sess-{}-{}",
            crate::util::now_ts_secs(),
            &uuid::Uuid::new_v4().to_string()[..8]
        ),
        title: format!("{} (continued)", old.title),
        created_at: crate::util::now_ts_secs(),
        creator: "user".into(),
        agents,
        user_id: old.user_id.clone(),
        title_locked: true,
        ..Default::default()
    };
    meta.set_cwd(&old.cwd);
    meta.settle_cwd();
    // It goes on with coding work: a Coding session, as `work: code` makes one.
    let _ = crate::engine::session_spec::ask_work(&mut meta, "code");
    meta
}

/// The carried conversation as one system note every member reads (a
/// compaction row: the thread starts there, `doc/shared-session-spec.md`).
fn seed_row(meta: &SessionMeta, content: String) -> ChatMsg {
    ChatMsg {
        agent_id: meta
            .agents
            .first()
            .map(|m| m.id.clone())
            .unwrap_or_default(),
        from_id: COMPACTION_SENDER.to_string(),
        to_id: "user".to_string(),
        content,
        timestamp: meta.created_at,
        is_observation: false,
        client_id: None,
        failed: false,
        via: None,
        via_note: None,
        images: Vec::new(),
        run_id: None,
        place: None,
    }
}

/// What the new session starts with: the person's messages and Claude's
/// replies, newest kept, within [`CARRY_CHARS`]. Tool calls, their results
/// and the engine's own rows are left out: the files they touched are still
/// in the folder.
pub(crate) fn carried(title: &str, rows: &[ChatMsg]) -> String {
    let lines: Vec<String> = rows
        .iter()
        .filter(|row| !row.is_observation && !row.content.trim().is_empty())
        .filter_map(|row| {
            let who = match row.author() {
                RowAuthor::Person => "Person",
                RowAuthor::Agent(_) => "Claude",
                _ => return None,
            };
            Some(format!("{who}: {}", cut(row.content.trim())))
        })
        .collect();
    let mut kept = Vec::new();
    let mut used = 0;
    for line in lines.iter().rev() {
        if used + line.len() > CARRY_CHARS && !kept.is_empty() {
            break;
        }
        used += line.len();
        kept.push(line.as_str());
    }
    kept.reverse();
    let mut text = format!(
        "[Continued from the Claude session \"{title}\" — {} of its {} messages carried]\n\n\
         This session goes on from a Claude session that is now read-only. \
         Its conversation, as it was:\n\n",
        kept.len(),
        lines.len()
    );
    text.push_str(&kept.join("\n\n"));
    text
}

fn cut(text: &str) -> String {
    if text.chars().count() <= LINE_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(LINE_CHARS).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(from: &str, content: &str, observation: bool) -> ChatMsg {
        ChatMsg {
            agent_id: "claude".into(),
            from_id: from.into(),
            to_id: "user".into(),
            content: content.into(),
            timestamp: 1,
            is_observation: observation,
            client_id: None,
            failed: false,
            via: None,
            via_note: None,
            images: Vec::new(),
            run_id: None,
            place: None,
        }
    }

    /// The carry keeps the conversation and leaves out tool rows.
    #[test]
    fn an_old_claude_session_carries_its_conversation_not_its_tools() {
        let rows = [
            row("user", "Rename the ferry module", false),
            row("claude", r#"{"type":"tool","tool":"Edit"}"#, true),
            row("system", "Tool Edit: done", true),
            row("claude", "Renamed it to boats.rs.", false),
        ];
        let text = carried("Ferry", &rows);
        assert!(text.contains("Person: Rename the ferry module"), "{text}");
        assert!(text.contains("Claude: Renamed it to boats.rs."), "{text}");
        assert!(!text.contains("Tool Edit"), "{text}");
        assert!(text.contains("2 of its 2 messages carried"), "{text}");
    }

    /// A long conversation keeps its newest lines within the budget.
    #[test]
    fn a_long_old_conversation_keeps_its_newest_lines() {
        let rows: Vec<ChatMsg> = (0..40)
            .map(|i| row("user", &format!("line {i} {}", "x".repeat(3000)), false))
            .collect();
        let text = carried("Long", &rows);
        assert!(text.contains("line 39"));
        assert!(!text.contains("line 0 "));
        assert!(text.len() < CARRY_CHARS + 1000);
    }
}
