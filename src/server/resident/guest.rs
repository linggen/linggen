//! A guest at another table: the user addressed her in an app's chat
//! (`@银月 …`), and she answers there — in that chat, labeled as her, and
//! aloud. The session's agent (Ling) is not woken; she reads the exchange at
//! the start of his next turn (`chat::side_lines`).

use super::*;
use crate::server::{AgentStatusKind, ServerEvent};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use tokio::sync::oneshot;

/// Whether `session_id` is one of her own rolling sessions.
pub(crate) fn is_own_session(session_id: &str) -> bool {
    session_id
        .strip_prefix(&yinyue_session_prefix())
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
}

/// Her guest turns at each table, in line: the newest one's ticket and the
/// signal it gives when done. Two quick `@银月` messages each get a turn of
/// their own, in the order sent — never two at once, whose run, interrupt
/// channel (keyed by session and agent) and status would clear each other's.
static LINE: LazyLock<Mutex<HashMap<String, (u64, oneshot::Receiver<()>)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static TICKETS: AtomicU64 = AtomicU64::new(0);

/// A place in the line at one table.
struct Turn {
    session_id: String,
    ticket: u64,
    /// The turn before this one, done when it signals (or is dropped).
    before: Option<oneshot::Receiver<()>>,
    done: oneshot::Sender<()>,
}

impl Turn {
    /// Join the end of the line at `session_id` — at once, in call order.
    fn join(session_id: &str) -> Self {
        let (done, next) = oneshot::channel();
        let ticket = TICKETS.fetch_add(1, Ordering::Relaxed);
        let before = LINE
            .lock_ok()
            .insert(session_id.to_string(), (ticket, next))
            .map(|(_, rx)| rx);
        Turn {
            session_id: session_id.to_string(),
            ticket,
            before,
            done,
        }
    }

    /// Wait until the turn before this one is over.
    async fn wait(&mut self) {
        if let Some(before) = self.before.take() {
            let _ = before.await;
        }
    }

    /// Hand the table to the next turn. True when nobody is waiting — the
    /// last in line, whose end is the table's end of her turns.
    fn finish(self) -> bool {
        let mut line = LINE.lock_ok();
        let last = line
            .get(&self.session_id)
            .is_some_and(|(t, _)| *t == self.ticket);
        if last {
            line.remove(&self.session_id);
        }
        let _ = self.done.send(());
        last
    }
}

/// Put the user's message on the table and answer it in the background.
pub(crate) async fn answer_as_guest(state: Arc<ServerState>, session_id: String, message: String) {
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
    let mut turn = Turn::join(&session_id);
    tokio::spawn(async move {
        turn.wait().await;
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
        let reply = run_guest_turn(&state, session_id.clone(), message).await;
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
                tracing::info!("[yinyue] answered as a guest in {session_id}");
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
        // The next message's turn keeps the table busy: only the last in
        // line says she is idle there.
        if !turn.finish() {
            return;
        }
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
    use super::{is_own_session, Turn};
    use crate::util::LockExt;

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

    /// One message, one run. The turn core begins and finishes the run it
    /// executes (`run_loop_with_tracking`); a caller that also began one
    /// wrapped it in a second "running" row (yinyue01 + yinyue02 for one
    /// `@银月 你好`, 2026-09-24), and the chat's stop could cancel the outer
    /// one, which the engine never checks.
    #[test]
    fn her_turns_leave_the_run_to_the_turn_core() {
        for (file, src) in [
            ("turn.rs", include_str!("turn.rs")),
            ("triggers.rs", include_str!("triggers.rs")),
            ("guest.rs", include_str!("guest.rs")),
        ] {
            let calls = src.matches(concat!(".begin_agent_run", "(")).count();
            assert_eq!(calls, 0, "{file} begins a run around the turn core");
        }
    }

    /// Two quick messages at one table: the second waits for the first,
    /// and only the last in line ends her turns there. Another table has a
    /// line of its own.
    #[tokio::test]
    async fn her_guest_turns_at_one_table_run_one_at_a_time_in_order() {
        let first = Turn::join("sess-line-test");
        let mut second = Turn::join("sess-line-test");
        let mut elsewhere = Turn::join("sess-line-other");
        elsewhere.wait().await;
        assert!(elsewhere.finish(), "its own line, done at once");

        let waiting = tokio::spawn(async move {
            second.wait().await;
            second
        });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished(), "the second waits for the first");
        assert!(!first.finish(), "someone is waiting: not the end");
        let second = waiting.await.unwrap();
        assert!(second.finish(), "the last in line ends her turns here");
        assert!(!super::LINE.lock_ok().contains_key("sess-line-test"));
    }

    #[test]
    fn her_rolling_sessions_are_hers_and_an_apps_chat_is_not() {
        assert!(is_own_session("sess-yinyue-2026-09-24"));
        assert!(is_own_session("sess-yinyue-2026-09-24-2"));
        assert!(!is_own_session("sess-lingjing-1758700000"));
        assert!(!is_own_session("sess-yinyuex-1"));
    }
}
