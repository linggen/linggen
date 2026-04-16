use super::types::*;
use crate::engine::actions;
use crate::engine::render::normalize_tool_path_arg;
use crate::ollama::ChatMessage;
use anyhow::Result;
use std::collections::HashSet;
use std::path::Path;
use tokio_stream::StreamExt as TokioStreamExt;
use tracing::{debug, info, warn};

/// Unescape common JSON string escape sequences so streamed plan text
/// renders as readable markdown in the UI.
/// Unescape JSON string escape sequences in a streaming delta.
/// Processes character by character to handle `\\n` (literal backslash + n)
/// vs `\n` (newline) correctly, unlike chained `.replace()`.
fn unescape_json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some(other) => { out.push('\\'); out.push(other); }
                None => out.push('\\'), // trailing backslash
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Think-tag stripping
// ---------------------------------------------------------------------------

/// Strip `<think>...</think>` blocks that many local models (Qwen, DeepSeek)
/// emit for chain-of-thought reasoning. These should not appear in user-facing
/// text responses. Also handles unclosed `<think>` (strips to end of string).
pub(crate) fn strip_think_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(start) = remaining.find("<think>") {
        result.push_str(&remaining[..start]);
        remaining = &remaining[start + 7..]; // skip "<think>"
        if let Some(end) = remaining.find("</think>") {
            remaining = &remaining[end + 8..]; // skip "</think>"
        } else {
            // Unclosed <think> — strip everything after it
            remaining = "";
        }
    }
    result.push_str(remaining);
    result
}

// ---------------------------------------------------------------------------
// JSON recovery helpers
// ---------------------------------------------------------------------------

/// Extract the first valid JSON object from a string that may contain multiple
/// concatenated JSON objects (e.g. `{"a":1}{"b":2}`). Some models produce this
/// when they try to emit multiple tool calls under a single delta index.
fn extract_first_json_object(s: &str) -> Option<serde_json::Value> {
    let trimmed = s.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    // Use serde_json's streaming deserializer to read just the first object
    let mut de = serde_json::Deserializer::from_str(trimmed).into_iter::<serde_json::Value>();
    if let Some(Ok(val)) = de.next() {
        if val.is_object() {
            return Some(val);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Free functions for parallel tool execution
// ---------------------------------------------------------------------------

/// Returns true for tools that are safe to execute in parallel.
/// Includes read-only tools and write tools (Write/Edit) which are
/// parallelizable when targeting different files (checked separately).
pub(crate) fn can_parallel_tool(tool: &str) -> bool {
    matches!(
        tool,
        "Read" | "Glob" | "Grep" | "WebSearch" | "WebFetch"
            | "capture_screenshot" | "Skill" | "Write" | "Edit"
    )
}

/// Check if a batch of parallelizable actions has write-path conflicts
/// (multiple Write/Edit targeting the same file). Returns true if conflicts
/// exist, meaning the batch must fall back to sequential execution.
pub(crate) fn has_write_path_conflicts(
    actions: &[(&str, &serde_json::Value)],
    cwd: &Path,
) -> bool {
    let mut write_paths: HashSet<String> = HashSet::new();
    for (tool, args) in actions {
        if matches!(*tool, "Write" | "Edit") {
            match normalize_tool_path_arg(cwd, args) {
                Some(path) => {
                    if !write_paths.insert(path) {
                        return true; // duplicate path
                    }
                }
                None => return true, // can't determine path — be safe
            }
        }
    }
    false
}

/// Check if context files (CLAUDE.md, ~/.linggen/memory/*.md, etc.) have changed
/// by comparing a content hash against the previous hash.
/// Runs on a background thread during tool execution.
pub(crate) fn check_context_staleness(
    prev_hash: Option<u64>,
    ws_root: &Path,
) -> bool {
    let Some(prev_hash) = prev_hash else { return false };
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    for name in &["CLAUDE.md", "AGENTS.md", ".cursorrules"] {
        if let Ok(content) = std::fs::read_to_string(ws_root.join(name)) {
            content.hash(&mut hasher);
        }
    }
    // Hash global memory file frontmatters for staleness detection
    let mem_dir = crate::paths::global_memory_dir();
    if let Ok(entries) = std::fs::read_dir(&mem_dir) {
        let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        paths.sort(); // stable ordering
        for path in paths {
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    content.hash(&mut hasher);
                }
            }
        }
    }
    hasher.finish() != prev_hash
}

// ---------------------------------------------------------------------------
// AgentEngine model streaming methods
// ---------------------------------------------------------------------------

impl AgentEngine {
    /// Stream model output with thinking-token forwarding.
    ///
    /// Uses `chat_text_stream` (no format constraint) instead of `chat_json`
    /// so the model can emit prose "thinking" tokens before the JSON action.
    /// Thinking tokens are forwarded via `self.thinking_tx` and the full
    /// accumulated text is returned for action parsing.
    pub(crate) async fn stream_with_thinking_model(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
    ) -> Result<StreamResult> {
        use crate::agent_manager::models::StreamChunk;
        let mut stream = self
            .model_manager
            .chat_text_stream(model_id, messages)
            .await?;
        let mut accumulated = String::new();
        let mut thinking_ended = false;
        let mut token_usage = None;
        let mut first_action: Option<(actions::ModelAction, usize)> = None;

        loop {
            let chunk_result = match TokioStreamExt::next(&mut stream).await {
                Some(result) => result,
                None => break, // stream ended
            };
            let chunk = chunk_result?;
            match chunk {
                StreamChunk::Token(token) => {
                    accumulated.push_str(&token);
                    if !thinking_ended {
                        if Self::looks_like_json_action_start(&accumulated) {
                            thinking_ended = true;
                            if let Some(tx) = &self.thinking_tx {
                                let _ = tx.send(ThinkingEvent::Done);
                            }
                        } else if let Some(tx) = &self.thinking_tx {
                            let _ = tx.send(ThinkingEvent::Token(token));
                        }
                    }
                    // After thinking ended, try to parse the first complete action
                    // from the accumulated buffer. This avoids double-parsing later.
                    if thinking_ended && first_action.is_none() {
                        if let Some(parsed) = actions::try_parse_first_action(&accumulated) {
                            first_action = Some(parsed);
                        }
                    }
                }
                StreamChunk::Usage(usage) => {
                    token_usage = Some(usage);
                }
                StreamChunk::ToolCall(_) => {
                    // Tool call chunks are not expected in legacy streaming mode;
                    // they are handled by stream_with_tool_calling().
                }
            }
        }

        // If thinking never ended (entire response was prose), signal done.
        if !thinking_ended {
            if let Some(tx) = &self.thinking_tx {
                let _ = tx.send(ThinkingEvent::Done);
            }
        }

        // Strip <think>...</think> blocks from the accumulated text.
        let accumulated = strip_think_tags(&accumulated);

        Ok(StreamResult {
            full_text: accumulated,
            token_usage,
            first_action,
            tool_calls: Vec::new(),
        })
    }

    /// Stream model output with native tool calling support.
    ///
    /// Sends tool definitions via the `tools` parameter and accumulates
    /// `StreamChunk::ToolCall` deltas into complete `ParsedToolCall` objects.
    /// Text content is forwarded via `thinking_tx` for the UI.
    pub(crate) async fn stream_with_tool_calling(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
        tools: Vec<serde_json::Value>,
    ) -> Result<StreamResult> {
        use crate::agent_manager::models::StreamChunk;

        let mut stream = self
            .model_manager
            .chat_tool_stream(model_id, messages, tools)
            .await?;

        let mut accumulated_text = String::new();
        let mut token_usage = None;
        // Track whether we're inside a <think> block to suppress streaming
        let mut in_think_block = false;
        // Accumulate tool call deltas keyed by index
        let mut tc_ids: Vec<Option<String>> = Vec::new();
        let mut tc_names: Vec<Option<String>> = Vec::new();
        let mut tc_args: Vec<String> = Vec::new();
        let mut tc_thought_sigs: Vec<Option<String>> = Vec::new();
        // Track whether any regular content tokens were streamed. If so, skip
        // streaming ExitPlanMode tool args to avoid showing plan text twice
        // (models like DeepSeek/Gemini output plan as text AND tool arg).
        let mut had_content_tokens = false;

        loop {
            let chunk_result = match TokioStreamExt::next(&mut stream).await {
                Some(result) => result,
                None => break,
            };
            let chunk = chunk_result?;
            match chunk {
                StreamChunk::Token(token) => {
                    accumulated_text.push_str(&token);
                    // Detect <think> / </think> boundaries for streaming suppression.
                    if !in_think_block && accumulated_text.contains("<think>") {
                        in_think_block = true;
                    }
                    if in_think_block && accumulated_text.contains("</think>") {
                        in_think_block = false;
                    }
                    // Stream content tokens so the UI shows progress in real time.
                    // Skip tokens inside <think> blocks.
                    // Plan mode tokens stream normally — the UI shows them as
                    // a generating message. When PlanUpdate arrives the frontend
                    // replaces the streaming text with the PlanBlock.
                    if !in_think_block {
                        if let Some(tx) = &self.thinking_tx {
                            let _ = tx.send(ThinkingEvent::ContentToken(token));
                        }
                        had_content_tokens = true;
                    }
                }
                StreamChunk::Usage(usage) => {
                    token_usage = Some(usage);
                }
                StreamChunk::ToolCall(tc) => {
                    let idx = tc.index;
                    // Grow accumulators if needed
                    while tc_ids.len() <= idx {
                        tc_ids.push(None);
                        tc_names.push(None);
                        tc_args.push(String::new());
                        tc_thought_sigs.push(None);
                    }
                    if let Some(id) = tc.id {
                        tc_ids[idx] = Some(id);
                    }
                    if let Some(ref name) = tc.name {
                        tc_names[idx] = Some(name.clone());
                    }
                    if let Some(args_delta) = tc.arguments_delta {
                        // Plan text streaming removed — the PlanBlock from
                        // finalize_plan_mode renders the full plan cleanly.
                        // Streaming caused: wrong position, \n formatting bugs,
                        // content disappearing due to finalizeMessage race.
                        tc_args[idx].push_str(&args_delta);
                    }
                    if tc.thought_signature.is_some() {
                        tc_thought_sigs[idx] = tc.thought_signature;
                    }
                }
            }
        }

        // Signal content stream done (not thinking done — avoids re-enabling
        // the UI thinking indicator after content tokens have been streamed).
        if let Some(tx) = &self.thinking_tx {
            let _ = tx.send(ThinkingEvent::ContentDone);
        }

        // Build ParsedToolCall objects from accumulated deltas.
        // Skip phantom entries (empty name + empty args) caused by Responses API
        // output_index gaps (text blocks occupy indices before function calls).
        let mut tool_calls = Vec::new();
        for i in 0..tc_ids.len() {
            let name = tc_names[i].clone().unwrap_or_default();
            let args_str = &tc_args[i];
            if name.is_empty() && args_str.is_empty() {
                tracing::debug!("Skipping phantom tool call at index {} (empty name and args)", i);
                continue;
            }
            let id = tc_ids[i].clone().unwrap_or_else(|| format!("fc_fallback_{}", i));
            let arguments = if args_str.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(args_str).unwrap_or_else(|_| {
                    // Some models (e.g. Qwen) concatenate multiple JSON objects
                    // into a single tool call delta: {"a":1}{"b":2}{"c":3}
                    // Try to extract the first valid JSON object.
                    extract_first_json_object(args_str).unwrap_or_else(|| {
                        warn!("Failed to parse tool call arguments as JSON: {}", args_str);
                        serde_json::json!({})
                    })
                })
            };
            let thought_signature = tc_thought_sigs.get(i).and_then(|s| s.clone());
            tool_calls.push(ParsedToolCall {
                id,
                name,
                arguments,
                thought_signature,
            });
        }

        // Strip <think>...</think> blocks from the accumulated text.
        let accumulated_text = strip_think_tags(&accumulated_text);

        if accumulated_text.is_empty() && tool_calls.is_empty() {
            warn!(
                "Model '{}' returned empty response (no text, no tool calls). Raw accumulated len={}, tool call deltas={}",
                model_id, accumulated_text.len(), tc_ids.len()
            );
        }

        Ok(StreamResult {
            full_text: accumulated_text,
            token_usage,
            first_action: None,
            tool_calls,
        })
    }

    /// Call the LLM using the configured model (no fallback).
    ///
    /// When `tools` is `Some`, uses native function calling via `stream_with_tool_calling()`.
    /// When `tools` is `None`, uses legacy JSON action format via `stream_with_thinking_model()`.
    pub(crate) async fn stream_with_fallback(
        &mut self,
        messages: &[ChatMessage],
        tools: Option<Vec<serde_json::Value>>,
    ) -> Result<StreamResult> {
        let model_id = self.model_id.clone();

        let result = if let Some(ref tool_defs) = tools {
            self.stream_with_tool_calling(&model_id, messages, tool_defs.clone())
                .await
        } else {
            self.stream_with_thinking_model(&model_id, messages).await
        };

        match result {
            Ok(result) => {
                self.last_token_usage = result.token_usage.clone();
                Ok(result)
            }
            Err(e) => Err(e),
        }
    }

    /// Emit a ModelFallback event via the agent manager.
    pub(crate) async fn emit_model_fallback_event(&self, preferred: &str, actual: &str, reason: &str) {
        let Some(manager) = self.tools.get_manager() else { return };
        let agent_id = self.agent_id.clone().unwrap_or_else(|| "unknown".to_string());
        manager
            .send_event(crate::agent_manager::AgentEvent::ModelFallback {
                agent_id,
                preferred_model: preferred.to_string(),
                actual_model: actual.to_string(),
                reason: reason.to_string(),
            }, self.session_id.clone())
            .await;
    }

    /// Drain any pending tool progress lines from the channel and forward
    /// them as AgentEvent::ToolProgress to the manager.
    pub(crate) async fn drain_tool_progress(
        &self,
        progress_rx: &mut tokio::sync::mpsc::UnboundedReceiver<(String, String, String)>,
    ) {
        let Some(manager) = self.tools.get_manager() else { return };
        let agent_id = self.agent_id.clone().unwrap_or_else(|| "unknown".to_string());
        while let Ok((tool, stream, line)) = progress_rx.try_recv() {
            manager
                .send_event(crate::agent_manager::AgentEvent::ToolProgress {
                    agent_id: agent_id.clone(),
                    tool,
                    line,
                    stream,
                }, self.session_id.clone())
                .await;
        }
    }

    pub(crate) fn looks_like_json_action_start(text: &str) -> bool {
        if let Some(brace_idx) = text.rfind('{') {
            text[brace_idx..].contains("\"type\"")
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_think_tags_basic() {
        assert_eq!(
            strip_think_tags("<think>reasoning here</think>Hello!"),
            "Hello!"
        );
    }

    #[test]
    fn strip_think_tags_no_tags() {
        assert_eq!(strip_think_tags("Hello world"), "Hello world");
    }

    #[test]
    fn strip_think_tags_unclosed() {
        assert_eq!(
            strip_think_tags("Before<think>reasoning without end"),
            "Before"
        );
    }

    #[test]
    fn strip_think_tags_multiple() {
        assert_eq!(
            strip_think_tags("<think>first</think>A<think>second</think>B"),
            "AB"
        );
    }

    #[test]
    fn strip_think_tags_with_newlines() {
        let input = "<think>\nThe user said hi.\nI should respond.\n</think>\nHi! How can I help?";
        assert_eq!(strip_think_tags(input), "\nHi! How can I help?");
    }
}
