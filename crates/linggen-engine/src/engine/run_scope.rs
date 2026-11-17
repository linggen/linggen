//! What a run sends and saves carries the run: the one place an engine's
//! events and rows get their `run_id`, so the chat binds each to that run's
//! bubble instead of guessing by agent, words and time (chat-spec.md § Runs).

use super::AgentEngine;
use crate::engine::agent::AgentEvent;
use crate::engine::tool_render::tool_call_status;
use crate::engine::types::{StreamedToken, ThinkingEvent};
use crate::state_fs::sessions::ChatMsg;
use std::sync::Mutex;

/// What a tool block still open when its run ends says.
const ENDED_OPEN: &str = "Stopped: the turn ended before it finished";

/// The tool blocks a run opened in the chat (`ContentBlockStart`) and has
/// not ended (`ContentBlockUpdate` done or failed), by block id, with the
/// agent each belongs to. Every block the run sends passes `emit`, so this
/// sees them all, whatever path the call took.
#[derive(Debug, Default)]
pub(crate) struct OpenCalls(Mutex<Vec<(String, String)>>);

impl OpenCalls {
    /// Keep track of `event`: a tool block opens or ends.
    pub(crate) fn note(&self, event: &AgentEvent) {
        let Ok(mut open) = self.0.lock() else { return };
        match event {
            AgentEvent::ContentBlockStart {
                agent_id,
                block_id,
                block_type,
                ..
            } if block_type == "tool_use" => open.push((block_id.clone(), agent_id.clone())),
            AgentEvent::ContentBlockUpdate {
                block_id,
                status: Some(status),
                ..
            } if status != "running" => open.retain(|(id, _)| id != block_id),
            _ => {}
        }
    }

    /// The blocks still open, emptied.
    pub(crate) fn take(&self) -> Vec<(String, String)> {
        self.0
            .lock()
            .map(|mut open| std::mem::take(&mut *open))
            .unwrap_or_default()
    }
}

impl AgentEngine {
    /// Send `event` as this run's: stamped with its run (and parent run).
    pub(crate) async fn emit(&self, mut event: AgentEvent) {
        let Some(manager) = self.tools.get_manager() else {
            return;
        };
        event.stamp_run(self.run_id.as_deref(), self.parent_run_id.as_deref());
        self.open_calls.note(&event);
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

    /// End the run's tool blocks still open: failed, as a call whose result
    /// never came back is (chat-spec § Tool calls on reload). However the
    /// run ended — done, error, timeout, cancel — a block it opened ends
    /// with it, so the chat never shows a call running after its turn.
    pub(crate) async fn settle_open_calls(&self) {
        for (block_id, agent_id) in self.open_calls.take() {
            self.emit(AgentEvent::ContentBlockUpdate {
                agent_id,
                block_id,
                status: Some(tool_call_status(true).to_string()),
                summary: Some(ENDED_OPEN.to_string()),
                is_error: Some(true),
                parent_id: self.parent_agent_id.clone(),
                extra: None,
                run_id: self.run_id.clone(),
                parent_run_id: self.parent_run_id.clone(),
            })
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OpenCalls;
    use crate::engine::agent::AgentEvent;

    fn start(id: &str, block_type: &str) -> AgentEvent {
        AgentEvent::ContentBlockStart {
            agent_id: "ling".into(),
            block_id: id.into(),
            block_type: block_type.into(),
            tool: Some("Bash".into()),
            args: None,
            parent_id: None,
            run_id: None,
            parent_run_id: None,
        }
    }

    fn update(id: &str, status: &str) -> AgentEvent {
        AgentEvent::ContentBlockUpdate {
            agent_id: "ling".into(),
            block_id: id.into(),
            status: Some(status.into()),
            summary: None,
            is_error: None,
            parent_id: None,
            extra: None,
            run_id: None,
            parent_run_id: None,
        }
    }

    /// A tool block stays open until its update ends it; what is open when
    /// the run ends is what the run settles — once.
    #[test]
    fn a_tool_block_left_open_is_what_the_run_end_settles() {
        let open = OpenCalls::default();
        for e in [
            start("a", "tool_use"),
            start("b", "tool_use"),
            start("c", "tool_use"),
            start("t", "text"),
            update("a", "done"),
            update("b", "running"),
            update("c", "failed"),
        ] {
            open.note(&e);
        }
        assert_eq!(open.take(), vec![("b".to_string(), "ling".to_string())]);
        assert!(open.take().is_empty());
    }
}
