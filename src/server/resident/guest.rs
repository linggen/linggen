//! A guest at another table: the user addressed her in an app's chat
//! (`@银月 …`), and she answers there — in that chat, labeled as her, and
//! aloud. The session's agent (Ling) is not woken; he reads the exchange at
//! the start of his next turn (`chat::side_lines`).

use super::*;
use crate::server::{AgentStatusKind, ServerEvent};

/// Whether `session_id` is one of her own rolling sessions.
pub(crate) fn is_own_session(session_id: &str) -> bool {
    session_id
        .strip_prefix(&yinyue_session_prefix())
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
}

/// Put the user's message on the table and answer it in the background.
pub(crate) async fn answer_as_guest(
    state: Arc<ServerState>,
    session_id: String,
    root: std::path::PathBuf,
    message: String,
) {
    crate::server::chat::helpers::persist_and_emit_to_store(
        &state.manager.global_sessions,
        &state.events_tx,
        YINYUE_AGENT,
        "user",
        YINYUE_AGENT,
        &message,
        Some(&session_id),
        false,
    )
    .await;
    crate::server::chat::side_lines::note(&session_id, YINYUE_AGENT, message.clone());
    tokio::spawn(async move {
        let sid = Some(session_id.clone());
        state
            .send_agent_status(
                YINYUE_AGENT.to_string(),
                AgentStatusKind::Thinking,
                Some("Thinking".to_string()),
                None,
                sid.clone(),
            )
            .await;
        let reply = run_guest_turn(&state, session_id.clone(), root, message).await;
        if let Some(reply) = reply {
            crate::server::chat::side_lines::note(
                &session_id,
                YINYUE_AGENT,
                format!(
                    "[{}]: {reply}",
                    crate::server::chat::sender_label(YINYUE_AGENT)
                ),
            );
            if let Some(line) = spoken_line(&reply) {
                crate::server::api::yinyue::emit_speak(&state, line, None);
            }
        }
        let _ = state.events_tx.send(ServerEvent::TurnComplete {
            agent_id: YINYUE_AGENT.to_string(),
            duration_ms: None,
            context_tokens: None,
            parent_id: None,
            session_id: sid.clone(),
            run_id: None,
            parent_run_id: None,
        });
        state
            .send_agent_status(
                YINYUE_AGENT.to_string(),
                AgentStatusKind::Idle,
                Some("Idle".to_string()),
                None,
                sid,
            )
            .await;
    });
}

#[cfg(test)]
mod tests {
    use super::is_own_session;

    /// A guest takes up nothing of the session's skill (`ChatRunCtx::guest`
    /// skips its activation), so what she can do there is exactly her own
    /// spec's list — which must hold no tool that changes anything and no
    /// question widget, and must answer to her alias.
    #[test]
    fn as_a_guest_she_brings_only_her_own_read_tools() {
        let (spec, _) = crate::extensions::agents::parse_agent_markdown(include_str!(
            "../../../agents/yinyue.md"
        ))
        .unwrap();
        assert!(spec.aliases.iter().any(|a| a == "银月"));
        for forbidden in ["Skill", "AskUser", "Write", "Edit", "Bash", "Task", "Exec"] {
            assert!(
                !spec.tools.iter().any(|t| t == forbidden),
                "{forbidden} is not hers to bring to a table"
            );
        }
    }

    #[test]
    fn her_rolling_sessions_are_hers_and_an_apps_chat_is_not() {
        assert!(is_own_session("sess-yinyue-2026-09-24"));
        assert!(is_own_session("sess-yinyue-2026-09-24-2"));
        assert!(!is_own_session("sess-lingjing-1758700000"));
        assert!(!is_own_session("sess-yinyuex-1"));
    }
}
