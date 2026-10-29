//! The next-prompt suggestion: what the person would most likely type next,
//! shown as the chat input's grey hint (Tab takes it). Claude Code's way:
//! after a turn ends, fork its last model request — same system prompt,
//! tools and history, so the provider's prompt cache serves nearly all of it
//! — with one more user message asking for the prediction. The fork never
//! reaches the transcript and runs no tools. See `doc/chat-spec.md`
//! § Suggestions.

mod filter;

use crate::engine::types::{PlanStatus, StreamResult};
use crate::engine::AgentEngine;
use crate::message::ChatMessage;
use crate::provider::models::{ModelManager, StreamChunk, TokenUsage};
use filter::clean;
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

/// The cache gate, Claude Code's number: fork only when the last call's
/// prompt was mostly served from cache. Over this many fresh tokens (prompt
/// not read from cache — which includes what it wrote to cache — plus the
/// reply, which the fork must read fresh) the fork is skipped.
const MAX_UNCACHED_TOKENS: u64 = 10_000;

/// Claude Code's suggestion prompt (CLI v2.1.309), worded for a chat rather
/// than a coding CLI. Two deliberate differences: a follow-up question is a
/// fine next step here ("what about TSLA?"), and the length rule speaks of
/// characters for Chinese and Japanese, as `filter` measures them.
const PROMPT: &str = "[SUGGESTION MODE: Suggest what the person might naturally type next in this chat.]

FIRST: Look at their recent messages and what they came for.

Your job is to predict what THEY would type — not what you think they should do.

THE TEST: Would they think \"I was just about to type that\"?

EXAMPLES:
They asked \"fix the bug and run the tests\", the bug is fixed → \"run the tests\"
You offered choices → the one they would most likely pick, given the conversation
You asked whether to go on → \"yes\" or \"go ahead\"
An answer that invites the obvious next step → that step, in their words (\"what about TSLA?\", \"show me the numbers\")
After an error or a misunderstanding → silence (let them assess and correct)

Be specific: \"trim half of NVDA\" beats \"continue\".

NEVER SUGGEST:
- Evaluative (\"looks good\", \"thanks\")
- Your own voice (\"Let me…\", \"I'll…\", \"Here's…\")
- New ideas they didn't ask about
- More than one sentence

Stay silent if the next step isn't obvious from what they said.

Stay silent if a suggestion could be unsafe or inappropriate — including any sensitive topic (security incidents, credentials, harm, private data).

Format: match their language and style, one line — 2–12 words, or 2–40 characters in Chinese or Japanese. To stay silent, reply with exactly: (none)

Reply with ONLY the suggestion — no quotes, no explanation. Do not call any tool.";

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
    /// suggestion and the call passes [`skip_reason`]. Each call replaces
    /// the last, so the turn's final call decides. A skill that meters a
    /// cloud is never forked.
    pub(crate) fn remember_call(
        &mut self,
        model_id: &str,
        messages: &[ChatMessage],
        tools: Option<&Vec<serde_json::Value>>,
        result: &StreamResult,
    ) {
        let metered_skill = self
            .active_skill
            .as_ref()
            .is_some_and(|s| s.cloud.is_some());
        if !self.suggest_followups || metered_skill {
            return;
        }
        self.last_call = None;
        if let Some(reason) = skip_reason(messages, result) {
            tracing::debug!("next-prompt suggestion: skip ({reason})");
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

    /// Why the turn's end is no moment to suggest: a plan is being made or
    /// waits for approval, or the model is rate-limited.
    fn blocked_reason(&self, call: &LastCall) -> Option<&'static str> {
        let plan_pending = self
            .plan
            .as_ref()
            .is_some_and(|p| p.status == PlanStatus::Planned);
        if self.plan_mode || plan_pending {
            return Some("plan_mode");
        }
        if crate::provider::models::in_cooldown(&call.model_id) {
            return Some("rate_limit");
        }
        None
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
        if let Some(reason) = self.blocked_reason(&call) {
            tracing::debug!("next-prompt suggestion: skip ({reason})");
            return;
        }
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

/// Claude Code's gates on the call itself; a failed turn never gets here.
/// - `ended_on_tool`: words before a tool call are not the last word, and a
///   turn that ends on a tool asked them something (or submitted a plan).
/// - `early_conversation`: fewer than two assistant replies, this one counted.
/// - `uncached`: too much of the prefix would be read fresh ([`uncached_tokens`]).
fn skip_reason(messages: &[ChatMessage], result: &StreamResult) -> Option<&'static str> {
    if !ended_in_words(result.tool_calls.is_empty(), &result.full_text) {
        return Some("ended_on_tool");
    }
    if assistant_replies(messages) + 1 < 2 {
        return Some("early_conversation");
    }
    if uncached_tokens(result.token_usage.as_ref()) > MAX_UNCACHED_TOKENS {
        return Some("uncached");
    }
    None
}

fn ended_in_words(no_tool_calls: bool, reply: &str) -> bool {
    no_tool_calls && !reply.trim().is_empty()
}

fn assistant_replies(messages: &[ChatMessage]) -> usize {
    messages.iter().filter(|m| m.role == "assistant").count()
}

/// The call's fresh tokens: prompt not read from cache (Anthropic counts
/// cache writes in it) plus the reply. A provider that reports no usage
/// counts as zero and is forked: there is nothing to judge by, and on a
/// local model the fork costs no money. A provider that reports a prompt
/// but no cache counts it all as fresh, so a long conversation is skipped.
fn uncached_tokens(usage: Option<&TokenUsage>) -> u64 {
    usage.map_or(0, TokenUsage::metered)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::types::{ParsedToolCall, Plan};
    use crate::engine::{AgentRole, EngineConfig, InterfaceMode};

    fn engine() -> AgentEngine {
        let cfg = EngineConfig::from_app_config(
            &crate::config::Config::default(),
            std::env::temp_dir(),
            InterfaceMode::Web,
        );
        let models = Arc::new(ModelManager::new_with_credentials(
            Vec::new(),
            &crate::credentials::Credentials::default(),
        ));
        let mut engine = AgentEngine::new(cfg, models, "m".to_string(), AgentRole::Lead).unwrap();
        engine.suggest_followups = true;
        engine
    }

    /// A second turn: one earlier reply, so this one makes two.
    fn history() -> Vec<ChatMessage> {
        ["user", "assistant", "user"]
            .iter()
            .map(|role| message(role, "hi"))
            .collect()
    }

    fn reply(text: &str, prompt: usize, cached: usize, output: usize) -> StreamResult {
        StreamResult {
            full_text: text.to_string(),
            token_usage: Some(TokenUsage {
                prompt_tokens: Some(prompt),
                completion_tokens: Some(output),
                total_tokens: Some(prompt + output),
                cached_tokens: Some(cached),
            }),
            first_action: None,
            tool_calls: Vec::new(),
        }
    }

    fn warm() -> StreamResult {
        reply("NVDA is 21% of it.", 50_000, 48_000, 300)
    }

    #[test]
    fn forks_a_warm_reply_that_ended_in_words() {
        assert_eq!(skip_reason(&history(), &warm()), None);
    }

    #[test]
    fn skips_a_turn_that_ended_on_a_tool() {
        let mut result = warm();
        result.tool_calls.push(ParsedToolCall {
            id: "1".into(),
            name: "AskUser".into(),
            arguments: serde_json::json!({}),
            thought_signature: None,
        });
        assert_eq!(skip_reason(&history(), &result), Some("ended_on_tool"));
        let blank = reply("  ", 100, 0, 1);
        assert_eq!(skip_reason(&history(), &blank), Some("ended_on_tool"));
    }

    #[test]
    fn skips_the_first_reply_of_a_conversation() {
        let first = vec![message("user", "hi")];
        assert_eq!(skip_reason(&first, &warm()), Some("early_conversation"));
    }

    #[test]
    fn skips_when_the_prefix_was_mostly_fresh() {
        // 12k of the prompt read fresh, or written to cache (Anthropic counts
        // writes in the prompt, not in `cached`).
        let cold = reply("ok, done.", 50_000, 38_000, 300);
        assert_eq!(skip_reason(&history(), &cold), Some("uncached"));
        // A long reply is read fresh by the fork too.
        let long = reply("a long answer", 20_000, 15_000, 6_000);
        assert_eq!(skip_reason(&history(), &long), Some("uncached"));
        let edge = reply("ok", 20_000, 10_000, 0);
        assert_eq!(skip_reason(&history(), &edge), None, "10,000 is still warm");
    }

    #[test]
    fn forks_when_the_provider_reports_no_usage() {
        let mut result = warm();
        result.token_usage = None;
        assert_eq!(skip_reason(&history(), &result), None);
    }

    #[test]
    fn remembers_only_a_call_that_passes_the_gates() {
        let mut engine = engine();
        engine.remember_call("m", &history(), None, &warm());
        assert!(engine.last_call.is_some());
        let first = vec![message("user", "hi")];
        engine.remember_call("m", &first, None, &warm());
        assert!(engine.last_call.is_none(), "a later call replaces it");
        engine.suggest_followups = false;
        engine.remember_call("m", &history(), None, &warm());
        assert!(engine.last_call.is_none(), "the turn didn't ask");
    }

    #[test]
    fn blocks_in_plan_mode_and_while_a_plan_waits() {
        let mut engine = engine();
        engine.remember_call("m", &history(), None, &warm());
        let call = engine.last_call.clone().unwrap();
        assert_eq!(engine.blocked_reason(&call), None);
        engine.plan_mode = true;
        assert_eq!(engine.blocked_reason(&call), Some("plan_mode"));
        engine.plan_mode = false;
        engine.plan = Some(Plan {
            summary: String::new(),
            status: PlanStatus::Planned,
            plan_text: String::new(),
            items: Vec::new(),
        });
        assert_eq!(engine.blocked_reason(&call), Some("plan_mode"));
    }

    #[test]
    fn blocks_while_the_model_is_rate_limited() {
        let mut engine = engine();
        let model = "suggestion-test-rate-limited";
        engine.remember_call(model, &history(), None, &warm());
        let call = engine.last_call.clone().unwrap();
        let err = anyhow::anyhow!("429 Too Many Requests: rate limit exceeded");
        crate::provider::models::note_unavailable(model, &err);
        assert_eq!(engine.blocked_reason(&call), Some("rate_limit"));
        crate::provider::models::note_healthy(model);
    }
}
