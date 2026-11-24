//! What this engine has to say about a call's effort, gathered into an
//! [`EffortAsk`]. The engine only gathers; the one decision of what is sent
//! is `provider::models::effort`. Gathering happens here and nowhere else
//! (lock: `effort::tests::only_the_resolver_and_the_asks_read_the_effort_fields`).

use super::AgentEngine;
use crate::engine::agent::AgentEvent;
use crate::provider::models::EffortAsk;

impl AgentEngine {
    /// The session's own choice, read from the stored session for each call
    /// so the picker's change holds from the next call of a turn running.
    pub(crate) fn session_effort(&self) -> Option<String> {
        let manager = self.tools.get_manager()?;
        manager.global_sessions.effort_of(self.session_id.as_deref()?)
    }

    /// Everything this engine says about the effort of a call in its own
    /// conversation. The chat default is asked for interactive chat only.
    pub(crate) fn effort_ask<'a>(&'a self, session: Option<&'a str>) -> EffortAsk<'a> {
        EffortAsk {
            forced: self.forced_effort,
            session,
            agent: self.reasoning_effort.as_deref(),
            chat_default: self.session_permissions.interactive.then_some(self.cfg.chat_effort.as_deref()).flatten(),
        }
    }

    /// The agent's own value alone: a side call (a summary) is not the
    /// conversation's, so the person's choices for the conversation do not
    /// reach it.
    pub(crate) fn agent_effort_ask(&self) -> EffortAsk<'_> {
        EffortAsk::agent(self.reasoning_effort.as_deref())
    }

    /// What a call to `model_id` sends, for a replay of it to send the same:
    /// the value, or `"auto"` for none.
    pub(crate) fn sent_effort(&self, model_id: &str) -> String {
        let session = self.session_effort();
        let resolved = self.model_manager.resolve_effort(model_id, &self.effort_ask(session.as_deref()));
        resolved.effort.wire().unwrap_or("auto").to_string()
    }

    /// Tell the conversation, once for each model and value, that a value the
    /// person chose is not one this model takes, so its calls go to the
    /// provider's default. The choice itself stays where it is stored.
    pub(crate) async fn tell_if_effort_held_back(&self, model_id: &str, ask: &EffortAsk<'_>) {
        let Some(value) = self.model_manager.resolve_effort(model_id, ask).held_back else { return };
        let key = (model_id.to_string(), value.clone());
        {
            let mut last = self.told_effort.lock().unwrap_or_else(|e| e.into_inner());
            if last.as_ref() == Some(&key) {
                return;
            }
            *last = Some(key);
        }
        if self.tools.builtins.delegation_depth() > 0 {
            return;
        }
        self.emit(AgentEvent::Message {
            from: "system".to_string(),
            to: "user".to_string(),
            content: format!("{model_id} has no \"{value}\" thinking effort; using its default for this call."),
            run_id: None,
            parent_id: None,
        })
        .await;
    }
}
