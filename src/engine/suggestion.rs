//! The next-prompt suggestion: what the person would most likely type next,
//! shown as the chat input's grey hint (Tab takes it). Claude Code's way:
//! after a turn ends, fork its last model request — same system prompt,
//! tools and history, so the provider's prompt cache serves nearly all of it
//! — with one more user message asking for the prediction. The fork never
//! reaches the transcript and runs no tools. See `doc/chat-spec.md`
//! § Suggestions.

use crate::engine::AgentEngine;
use crate::message::ChatMessage;
use crate::provider::models::{ModelManager, StreamChunk};
use futures_util::StreamExt;
use std::sync::Arc;

/// A turn's latest model request and its answer.
#[derive(Clone)]
pub struct LastCall {
    model_id: String,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<serde_json::Value>>,
    /// The turn's own effort: a different one is a different cache.
    effort: Option<String>,
    app: Option<String>,
    reply: String,
    ended_on_tool: bool,
}

/// A suggestion longer than this is not something a person types.
const MAX_WORDS: usize = 12;

const PROMPT: &str = "[SUGGESTION MODE: Suggest what the person might naturally type next in this chat.]

Look at their recent messages and what they came for. Predict what THEY would type — not what you think they should do. The test: would they think \"I was just about to type that\"?

- You offered choices → the one they would most likely pick.
- You asked whether to go on → \"yes\" or \"go ahead\".
- An answer that invites the obvious next step → that step, in their words (\"what about TSLA?\", \"show me the numbers\").
- After an error or a misunderstanding → nothing; let them correct it.

Be specific: \"trim half of NVDA\" beats \"continue\". Never suggest thanks or praise, your own voice (\"Let me…\", \"I'll…\"), or something new they didn't ask about. Match their language and style, 2–12 words, one line.

If the next step isn't obvious, or a suggestion could be unsafe or sensitive, reply with exactly: (none)

Reply with the suggestion only — no quotes, no explanation. Do not call any tool.";

impl AgentEngine {
    /// Keep a model call as the fork's prefix, when this turn wants a
    /// suggestion.
    pub(crate) fn remember_call(
        &mut self,
        model_id: &str,
        messages: &[ChatMessage],
        tools: Option<&Vec<serde_json::Value>>,
        result: &crate::engine::types::StreamResult,
    ) {
        if !self.suggest_followups {
            return;
        }
        self.last_call = Some(LastCall {
            model_id: model_id.to_string(),
            messages: messages.to_vec(),
            tools: tools.cloned(),
            effort: self.reasoning_effort.clone(),
            app: self.app_product().map(str::to_string),
            reply: result.full_text.clone(),
            ended_on_tool: !result.tool_calls.is_empty(),
        });
    }

    /// After a finished turn: predict the person's next message in the
    /// background and send it as a `followups` event of one item. Nothing
    /// when the turn didn't ask, ended on a tool (a question to them, a plan),
    /// its cache is cold, or its skill meters a cloud.
    pub(crate) fn spawn_next_suggestion(&mut self) {
        let Some(call) = self.last_call.take() else {
            tracing::debug!("next-prompt suggestion: no call to fork (asked {})", self.suggest_followups);
            return;
        };
        let metered_skill = self.active_skill.as_ref().is_some_and(|s| s.cloud.is_some());
        if !self.suggest_followups || metered_skill || !worth_forking(&call) {
            tracing::debug!(
                "next-prompt suggestion: not forked (asked {}, metered skill {metered_skill}, ended on tool {})",
                self.suggest_followups,
                call.ended_on_tool
            );
            return;
        }
        let Some(manager) = self.tools.get_manager() else {
            return;
        };
        let models = self.model_manager.clone();
        let session_id = self.session_id.clone();
        let agent_id = self.agent_id.clone().unwrap_or_else(|| "unknown".to_string());
        tokio::spawn(async move {
            match predict(&models, call).await {
                Ok(Some(text)) => {
                    manager
                        .send_event(
                            crate::engine::agent::AgentEvent::Followups { agent_id, items: vec![text] },
                            session_id,
                        )
                        .await;
                }
                Ok(None) => {}
                Err(e) => tracing::debug!("next-prompt suggestion skipped: {e:#}"),
            }
        });
    }
}

/// A reply worth predicting after. No cold-cache gate, unlike Claude Code's:
/// OpenAI-style caches are written by the turn's own call, so the fork hits
/// them (measured 2026-09-23 on gpt-6: 87–97% of the prompt cached) — as
/// long as the request carries its cache key (`provider/openai.rs`).
fn worth_forking(call: &LastCall) -> bool {
    !call.ended_on_tool && !call.reply.trim().is_empty()
}

/// The fork: the turn's own request, its reply, and the ask. Same tools, so
/// the prefix matches the cached one; a tool call back means no suggestion.
async fn predict(models: &Arc<ModelManager>, call: LastCall) -> anyhow::Result<Option<String>> {
    let mut messages = call.messages;
    messages.push(message("assistant", &call.reply));
    messages.push(message("user", PROMPT));
    let (effort, app) = (call.effort.as_deref(), call.app.as_deref());
    let mut stream = match call.tools {
        Some(tools) => models.chat_tool_stream(&call.model_id, &messages, tools, effort, app).await?,
        None => models.chat_text_stream(&call.model_id, &messages, effort, app).await?,
    };
    let mut text = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk? {
            StreamChunk::Token(t) => text.push_str(&t),
            StreamChunk::ToolCall(_) => return Ok(None),
            StreamChunk::Usage(u) => tracing::debug!(
                "next-prompt suggestion: prompt {:?}, cached {:?}, output {:?}",
                u.prompt_tokens, u.cached_tokens, u.completion_tokens
            ),
        }
    }
    let suggestion = clean(&crate::engine::streaming::strip_think_tags(&text));
    tracing::debug!("next-prompt suggestion: {text:?} → {suggestion:?}");
    Ok(suggestion)
}

fn message(role: &str, content: &str) -> ChatMessage {
    ChatMessage {
        role: role.to_string(),
        content: content.to_string(),
        thinking: None,
        images: Vec::new(),
        cache_control: None,
        tool_calls: Vec::new(),
        tool_call_id: None,
        name: None,
    }
}

/// The model's answer as a suggestion, or None when it declined or wrote
/// something no person would type.
pub(crate) fn clean(raw: &str) -> Option<String> {
    let line = raw.trim().trim_matches(['"', '“', '”', '\'', '`']).trim();
    let declined = line.is_empty()
        || line.contains('\n')
        || line.to_lowercase().trim_matches(['(', ')', '.', '[', ']']) == "none";
    let own_voice = ["Let me", "I'll", "I will", "Here's", "Here is"].iter().any(|p| line.starts_with(p));
    let words = line.split_whitespace().count();
    let cjk_long = line.chars().count() > 40;
    if declined || own_voice || words > MAX_WORDS || (words == 1 && cjk_long) {
        return None;
    }
    Some(line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_a_short_line_in_their_voice() {
        assert_eq!(clean("  \"trim half of NVDA\"\n"), Some("trim half of NVDA".to_string()));
        assert_eq!(clean("yes"), Some("yes".to_string()));
        assert_eq!(clean("那特斯拉呢？"), Some("那特斯拉呢？".to_string()));
    }

    #[test]
    fn drops_a_decline_its_own_voice_or_a_paragraph() {
        assert_eq!(clean("(none)"), None);
        assert_eq!(clean("None."), None);
        assert_eq!(clean(""), None);
        assert_eq!(clean("Let me check the numbers"), None);
        assert_eq!(clean("one\ntwo"), None);
        assert_eq!(clean("a b c d e f g h i j k l m n"), None);
    }

    fn call(reply: &str, ended_on_tool: bool) -> LastCall {
        LastCall {
            model_id: "m".into(),
            messages: Vec::new(),
            tools: None,
            effort: None,
            app: None,
            reply: reply.into(),
            ended_on_tool,
        }
    }

    #[test]
    fn forks_only_a_reply_that_ended_in_words() {
        assert!(worth_forking(&call("NVDA is 21% of it.", false)));
        assert!(!worth_forking(&call("", true)), "a turn that ended on a tool asked them something");
        assert!(!worth_forking(&call("  ", false)));
    }
}
