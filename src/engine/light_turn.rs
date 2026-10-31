//! A light turn: one line from the agent, or SILENT — no tools, no loop.
//!
//! Some kickoffs want a single line and nothing else (the one exchange after
//! the companion speaks in an app's chat). A full turn sends the whole
//! system prompt, the skill's body and every tool schema for it; this sends
//! the agent's soul, a short place, the chat's recent tail and the ask —
//! one model call through the engine's own routing and fallback. A line is
//! kept like any reply of the agent's; SILENT keeps nothing.

use crate::engine::AgentEngine;
use crate::message::ChatMessage;

/// How much of each tail message the light turn reads.
pub(crate) const TAIL_CHARS: usize = 280;

/// The messages a light turn sends: its persona, the tail, then the ask.
pub(crate) fn light_messages(
    persona: String,
    tail: Vec<ChatMessage>,
    ask: &str,
) -> Vec<ChatMessage> {
    let mut msgs = Vec::with_capacity(tail.len() + 2);
    msgs.push(ChatMessage::new("system", persona));
    msgs.extend(tail.into_iter().map(|mut m| {
        m.content = cut(&m.content, TAIL_CHARS);
        m
    }));
    msgs.push(ChatMessage::new("user", ask.to_string()));
    msgs
}

/// The line a reply leaves, if any: trimmed, `None` for silence or nothing.
pub(crate) fn kept_line(reply: &str) -> Option<String> {
    let line = reply.trim();
    (!line.is_empty() && !crate::engine::context::is_silence(line)).then(|| line.to_string())
}

fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

impl AgentEngine {
    /// One light turn in `session_id`: the agent answers `ask` after `tail`
    /// in one line, or is silent. The line is kept and shown as the agent's
    /// reply; silence leaves nothing. No tool schemas are sent, and the
    /// engine's cached thread is left as it is.
    pub(crate) async fn light_reply(
        &mut self,
        tail: Vec<ChatMessage>,
        ask: &str,
        session_id: &str,
    ) -> Option<String> {
        self.session_id = Some(session_id.to_string());
        let msgs = light_messages(self.light_system_prompt(), tail, ask);
        // No run of its own: a stale one would read as stopped, and nothing
        // streams to a forwarder — a SILENT must never flash in the chat.
        self.set_run_id(None);
        self.thinking_tx = None;
        let effort = self.reasoning_effort.replace("low".to_string());
        let result = self.stream_with_fallback(&msgs, None).await;
        self.reasoning_effort = effort;
        let reply = match result {
            Ok(r) => r.full_text,
            Err(e) => {
                tracing::warn!("light turn in {session_id} failed: {e:#}");
                return None;
            }
        };
        let line = kept_line(&reply)?;
        let _ = self
            .persist_assistant_message(&line, Some(session_id))
            .await;
        Some(line)
    }
}

#[cfg(test)]
mod tests;
