//! What a run sends and saves carries the run: the one place an engine's
//! events and rows get their `run_id`, so the chat binds each to that run's
//! bubble instead of guessing by agent, words and time (chat-spec.md § Runs).

use super::AgentEngine;
use crate::engine::agent::AgentEvent;
use crate::engine::types::{StreamedToken, ThinkingEvent};
use crate::state_fs::sessions::ChatMsg;

impl AgentEngine {
    /// Send `event` as this run's: stamped with its run (and parent run).
    pub(crate) async fn emit(&self, mut event: AgentEvent) {
        let Some(manager) = self.tools.get_manager() else {
            return;
        };
        event.stamp_run(self.run_id.as_deref(), self.parent_run_id.as_deref());
        manager.send_event(event, self.session_id.clone()).await;
    }

    /// Stream a token of this run to whoever listens (`thinking_tx`).
    pub(crate) fn stream(&self, event: ThinkingEvent) {
        if let Some(tx) = &self.thinking_tx {
            let run_id = self.run_id.clone();
            let _ = tx.send(StreamedToken { run_id, event });
        }
    }

    /// Save `row` in the session's log as this run's. A person's row (a
    /// kickoff's next item) opens a turn and never carries a run.
    pub(crate) async fn save_row(&self, session_id: Option<&str>, mut row: ChatMsg) {
        let Some(manager) = self.tools.get_manager() else {
            return;
        };
        if row.from_id != "user" {
            row.run_id = self.run_id.clone();
        }
        manager
            .add_chat_message(&self.cfg.ws_root, session_id.unwrap_or("default"), &row)
            .await;
    }
}
