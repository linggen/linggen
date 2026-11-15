//! Token accounting and the chunks a streaming chat call yields.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Token usage tracking from API responses
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: Option<usize>,
    pub completion_tokens: Option<usize>,
    pub total_tokens: Option<usize>,
    /// Prompt tokens the provider served from its cache — a part of
    /// `prompt_tokens`, not an addition to it.
    #[serde(default)]
    pub cached_tokens: Option<usize>,
}

impl TokenUsage {
    /// A later report fills what an earlier one left out: Anthropic sends
    /// the prompt at `message_start` and the output at `message_delta`.
    pub fn merged(self, later: TokenUsage) -> TokenUsage {
        let prompt_tokens = later.prompt_tokens.or(self.prompt_tokens);
        let completion_tokens = later.completion_tokens.or(self.completion_tokens);
        TokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: later
                .total_tokens
                .or(self.total_tokens)
                .or(prompt_tokens.zip(completion_tokens).map(|(p, c)| p + c)),
            cached_tokens: later.cached_tokens.or(self.cached_tokens),
        }
    }

    /// What the call cost the caller: the prompt not served from cache, plus
    /// the output. Falls back to `total_tokens` when the parts are unknown.
    /// A pace should count what the player did, not the cached world.
    pub fn metered(&self) -> u64 {
        match (self.prompt_tokens, self.completion_tokens) {
            (Some(p), Some(c)) => (p.saturating_sub(self.cached_tokens.unwrap_or(0)) + c) as u64,
            _ => self.total_tokens.unwrap_or(0) as u64,
        }
    }
}

/// What a run spent, summed over its model calls. A mission's run history
/// keeps it, so a skill can see what a night of its mission costs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunUsage {
    pub calls: usize,
    pub prompt: usize,
    /// A part of `prompt`, not an addition to it.
    pub cached: usize,
    pub output: usize,
    /// Calls whose provider reported no usage — the sums miss those.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unreported: usize,
    /// The models that answered, in order of first use. More than one means
    /// a fallback.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl RunUsage {
    pub fn add(&mut self, model: &str, usage: Option<&TokenUsage>) {
        self.calls += 1;
        if !self.models.iter().any(|m| m == model) {
            self.models.push(model.to_string());
        }
        let Some(u) = usage.filter(|u| u.prompt_tokens.is_some() || u.completion_tokens.is_some())
        else {
            self.unreported += 1;
            return;
        };
        self.prompt += u.prompt_tokens.unwrap_or(0);
        self.cached += u.cached_tokens.unwrap_or(0);
        self.output += u.completion_tokens.unwrap_or(0);
    }

    pub fn is_empty(&self) -> bool {
        self.calls == 0
    }
}

/// Items yielded by the streaming chat API.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// A text token (content or thinking).
    Token(String),
    /// Final usage stats from the API (emitted at end of stream).
    Usage(TokenUsage),
    /// A tool call chunk from native function calling (incremental or complete).
    ToolCall(ToolCallChunk),
}

/// An incremental tool call chunk from the streaming API.
/// For OpenAI: arguments arrive in pieces, keyed by `index`.
/// For Ollama: tool_calls arrive as complete objects per chunk.
#[derive(Debug, Clone)]
pub struct ToolCallChunk {
    /// Index within the tool_calls array (for accumulation).
    pub index: usize,
    /// Tool call ID (usually only present in the first chunk for this index).
    pub id: Option<String>,
    /// Function name (usually only present in the first chunk for this index).
    pub name: Option<String>,
    /// Incremental arguments JSON string fragment.
    pub arguments_delta: Option<String>,
    /// Gemini thought signature (opaque token that must be echoed back).
    pub thought_signature: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_merges_split_reports_and_meters_what_was_not_cached() {
        // Anthropic: prompt at message_start, output at message_delta.
        let start = TokenUsage {
            prompt_tokens: Some(19_000),
            cached_tokens: Some(17_500),
            ..Default::default()
        };
        let delta = TokenUsage {
            completion_tokens: Some(400),
            ..Default::default()
        };
        let u = start.merged(delta);
        assert_eq!(u.total_tokens, Some(19_400));
        assert_eq!(u.metered(), 1_900); // 19,000 − 17,500 + 400
                                        // No cache accounting: the whole prompt counts.
        let plain = TokenUsage {
            prompt_tokens: Some(100),
            completion_tokens: Some(50),
            ..Default::default()
        };
        assert_eq!(plain.metered(), 150);
        // Only a total known: it is what we have.
        let total = TokenUsage {
            total_tokens: Some(77),
            ..Default::default()
        };
        assert_eq!(total.metered(), 77);
    }

    #[test]
    fn run_usage_sums_calls_and_names_each_model_once() {
        let mut run = RunUsage::default();
        let call = TokenUsage {
            prompt_tokens: Some(9_000),
            cached_tokens: Some(6_000),
            completion_tokens: Some(200),
            ..Default::default()
        };
        let total_only = TokenUsage {
            total_tokens: Some(5),
            ..Default::default()
        };
        run.add("flash", Some(&call));
        run.add("flash", Some(&call));
        run.add("fallback", None);
        run.add("fallback", Some(&total_only));
        let sums = (run.calls, run.prompt, run.cached, run.output);
        assert_eq!(sums, (4, 18_000, 12_000, 400));
        assert_eq!(run.unreported, 2);
        assert_eq!(run.models, vec!["flash", "fallback"]);
        assert!(RunUsage::default().is_empty());
    }
}
