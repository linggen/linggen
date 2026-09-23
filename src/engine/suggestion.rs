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

/// A turn's last model request that ended in words, and those words.
#[derive(Clone)]
pub struct LastCall {
    model_id: String,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<serde_json::Value>>,
    /// The turn's own effort: a different one is a different cache.
    effort: Option<String>,
    app: Option<String>,
    reply: String,
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
    /// A turn begins: the last turn's hint is stale. Stop its fork if it is
    /// still asking the model, and forget the call it would fork from.
    pub(crate) fn begin_suggestion_turn(&mut self) {
        if let Some(fork) = self.suggestion_fork.take() {
            fork.abort();
        }
        self.last_call = None;
    }

    /// Keep a model call as the fork's prefix, when this turn wants a
    /// suggestion. Only a call that ended in words can be the turn's last
    /// word, so only that one is copied; a call that ended on a tool clears
    /// it (a question to them, a plan — nothing to predict after). A skill
    /// that meters a cloud is never forked.
    pub(crate) fn remember_call(
        &mut self,
        model_id: &str,
        messages: &[ChatMessage],
        tools: Option<&Vec<serde_json::Value>>,
        result: &crate::engine::types::StreamResult,
    ) {
        let metered_skill = self
            .active_skill
            .as_ref()
            .is_some_and(|s| s.cloud.is_some());
        if !self.suggest_followups || metered_skill {
            return;
        }
        if !ended_in_words(result.tool_calls.is_empty(), &result.full_text) {
            self.last_call = None;
            return;
        }
        self.last_call = Some(LastCall {
            model_id: model_id.to_string(),
            messages: messages.to_vec(),
            tools: tools.cloned(),
            effort: self.reasoning_effort.clone(),
            app: self.app_product().map(str::to_string),
            reply: result.full_text.clone(),
        });
    }

    /// After a finished turn: predict the person's next message in the
    /// background and send it as a `followups` event of one item, stamped
    /// with the turn's run so a surface can tell a late one. Nothing when
    /// the turn didn't ask or didn't end in words. The next turn on this
    /// engine aborts the fork ([`Self::begin_suggestion_turn`]).
    pub(crate) fn spawn_next_suggestion(&mut self, call: Option<LastCall>, run_id: &str) {
        let Some(call) = call else {
            tracing::debug!("next-prompt suggestion: no call to fork");
            return;
        };
        let Some(manager) = self.tools.get_manager() else {
            return;
        };
        let models = self.model_manager.clone();
        let session_id = self.session_id.clone();
        let agent_id = self
            .agent_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let run_id = run_id.to_string();
        let fork = tokio::spawn(async move {
            match predict(&models, call).await {
                Ok(Some(text)) => {
                    let event = crate::engine::agent::AgentEvent::Followups {
                        agent_id,
                        items: vec![text],
                        run_id: Some(run_id),
                    };
                    manager.send_event(event, session_id).await;
                }
                Ok(None) => {}
                Err(e) => tracing::debug!("next-prompt suggestion skipped: {e:#}"),
            }
        });
        self.suggestion_fork = Some(fork.abort_handle());
    }
}

/// A reply worth predicting after. No cold-cache gate, unlike Claude Code's:
/// OpenAI-style caches are written by the turn's own call, so the fork hits
/// them (measured 2026-09-23 on gpt-6: 87–97% of the prompt cached) — as
/// long as the request carries its cache key (`provider/openai.rs`).
fn ended_in_words(no_tool_calls: bool, reply: &str) -> bool {
    no_tool_calls && !reply.trim().is_empty()
}

/// The fork: the turn's own request, its reply, and the ask. Same tools, so
/// the prefix matches the cached one; a tool call back means no suggestion.
async fn predict(models: &Arc<ModelManager>, call: LastCall) -> anyhow::Result<Option<String>> {
    let mut messages = call.messages;
    messages.push(message("assistant", &call.reply));
    messages.push(message("user", PROMPT));
    let (effort, app) = (call.effort.as_deref(), call.app.as_deref());
    let mut stream = match call.tools {
        Some(tools) => {
            models
                .chat_tool_stream(&call.model_id, &messages, tools, effort, app)
                .await?
        }
        None => {
            models
                .chat_text_stream(&call.model_id, &messages, effort, app)
                .await?
        }
    };
    let mut text = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk? {
            StreamChunk::Token(t) => text.push_str(&t),
            StreamChunk::ToolCall(_) => return Ok(None),
            StreamChunk::Usage(u) => tracing::debug!(
                "next-prompt suggestion: prompt {:?}, cached {:?}, output {:?}",
                u.prompt_tokens,
                u.cached_tokens,
                u.completion_tokens
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
    let own_voice = ["Let me", "I'll", "I will", "Here's", "Here is"]
        .iter()
        .any(|p| line.starts_with(p));
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
        assert_eq!(
            clean("  \"trim half of NVDA\"\n"),
            Some("trim half of NVDA".to_string())
        );
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

    #[test]
    fn forks_only_a_reply_that_ended_in_words() {
        assert!(ended_in_words(true, "NVDA is 21% of it."));
        assert!(
            !ended_in_words(false, ""),
            "a turn that ended on a tool asked them something"
        );
        assert!(
            !ended_in_words(false, "Let me check."),
            "words before a tool call are not the last word"
        );
        assert!(!ended_in_words(true, "  "));
    }
}
