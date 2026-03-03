use super::types::*;
use crate::engine::actions;
use crate::engine::render::normalize_tool_path_arg;
use crate::ollama::ChatMessage;
use anyhow::Result;
use std::collections::HashSet;
use std::path::Path;
use tokio::time::{timeout, Duration};
use tokio_stream::StreamExt as TokioStreamExt;
use tracing::{debug, info, warn};

const FIRST_CHUNK_TIMEOUT: Duration = Duration::from_secs(120);
const INTER_CHUNK_TIMEOUT: Duration = Duration::from_secs(60);

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
    ws_root: &Path,
) -> bool {
    let mut write_paths: HashSet<String> = HashSet::new();
    for (tool, args) in actions {
        if matches!(*tool, "Write" | "Edit") {
            match normalize_tool_path_arg(ws_root, args) {
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

/// Check if context files (CLAUDE.md, MEMORY.md, etc.) have changed
/// by comparing a content hash against the previous hash.
/// Runs on a background thread during tool execution.
pub(crate) fn check_context_staleness(
    prev_hash: Option<u64>,
    ws_root: &Path,
    memory_dir: Option<&Path>,
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
    if let Some(mem_dir) = memory_dir {
        if let Ok(content) = std::fs::read_to_string(mem_dir.join("MEMORY.md")) {
            content.hash(&mut hasher);
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
            let chunk_timeout = if accumulated.is_empty() {
                FIRST_CHUNK_TIMEOUT
            } else {
                INTER_CHUNK_TIMEOUT
            };
            let chunk_result = match timeout(chunk_timeout, TokioStreamExt::next(&mut stream)).await
            {
                Ok(Some(result)) => result,
                Ok(None) => break, // stream ended
                Err(_) => {
                    let secs = chunk_timeout.as_secs();
                    anyhow::bail!(
                        "Model streaming timed out after {}s (no data received)",
                        secs
                    );
                }
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
        // Accumulate tool call deltas keyed by index
        let mut tc_ids: Vec<Option<String>> = Vec::new();
        let mut tc_names: Vec<Option<String>> = Vec::new();
        let mut tc_args: Vec<String> = Vec::new();

        loop {
            let chunk_timeout = if accumulated_text.is_empty() && tc_ids.is_empty() {
                FIRST_CHUNK_TIMEOUT
            } else {
                INTER_CHUNK_TIMEOUT
            };
            let chunk_result = match timeout(chunk_timeout, TokioStreamExt::next(&mut stream)).await
            {
                Ok(Some(result)) => result,
                Ok(None) => break,
                Err(_) => {
                    anyhow::bail!(
                        "Model streaming timed out after {}s (no data received)",
                        chunk_timeout.as_secs()
                    );
                }
            };
            let chunk = chunk_result?;
            match chunk {
                StreamChunk::Token(token) => {
                    accumulated_text.push_str(&token);
                    // Forward text tokens to the UI for streaming display
                    if let Some(tx) = &self.thinking_tx {
                        let _ = tx.send(ThinkingEvent::Token(token));
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
                    }
                    if let Some(id) = tc.id {
                        tc_ids[idx] = Some(id);
                    }
                    if let Some(name) = tc.name {
                        tc_names[idx] = Some(name);
                    }
                    if let Some(args_delta) = tc.arguments_delta {
                        tc_args[idx].push_str(&args_delta);
                    }
                }
            }
        }

        // Signal thinking done
        if let Some(tx) = &self.thinking_tx {
            let _ = tx.send(ThinkingEvent::Done);
        }

        // Build ParsedToolCall objects from accumulated deltas
        let mut tool_calls = Vec::new();
        for i in 0..tc_ids.len() {
            let id = tc_ids[i].clone().unwrap_or_else(|| format!("call_{}", i));
            let name = tc_names[i].clone().unwrap_or_default();
            let args_str = &tc_args[i];
            let arguments = if args_str.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(args_str).unwrap_or_else(|_| {
                    warn!("Failed to parse tool call arguments as JSON: {}", args_str);
                    serde_json::json!({})
                })
            };
            tool_calls.push(ParsedToolCall {
                id,
                name,
                arguments,
            });
        }

        Ok(StreamResult {
            full_text: accumulated_text,
            token_usage,
            first_action: None,
            tool_calls,
        })
    }

    /// Build an ordered model chain for fallback attempts.
    ///
    /// Order: primary model → `routing.default_models` → remaining configured models.
    /// Filters out models marked unavailable by the health tracker (keeps at least one).
    fn build_model_chain(&self) -> Vec<String> {
        let primary = self.model_id.clone();
        let all_ids = self.model_manager.model_ids();

        // Start with the primary, then default_models from config, then remaining.
        let mut chain = vec![primary.clone()];
        for dm in &self.default_models {
            if !chain.contains(dm) && all_ids.contains(dm) {
                chain.push(dm.clone());
            }
        }
        for id in &all_ids {
            if !chain.contains(id) {
                chain.push(id.clone());
            }
        }
        chain
    }

    /// Call the LLM with automatic fallback to other configured models
    /// when the primary model hits a rate limit (429) or context limit (400).
    /// Uses the health tracker to skip models known to be down/quota-exhausted.
    ///
    /// When `tools` is `Some`, uses native function calling via `stream_with_tool_calling()`.
    /// When `tools` is `None`, uses legacy JSON action format via `stream_with_thinking_model()`.
    pub(crate) async fn stream_with_fallback(
        &mut self,
        messages: &[ChatMessage],
        tools: Option<Vec<serde_json::Value>>,
    ) -> Result<StreamResult> {
        use crate::agent_manager::models;

        let chain = self.build_model_chain();
        let primary = self.model_id.clone();
        let health = self.model_manager.health.clone();
        let mut last_err: Option<anyhow::Error> = None;

        for model_id in &chain {
            // Skip models known to be unavailable (but always try at least the primary).
            if model_id != &primary && !health.is_available(model_id).await {
                debug!("Skipping model '{}': unavailable", model_id);
                continue;
            }

            let result = if let Some(ref tool_defs) = tools {
                self.stream_with_tool_calling(model_id, messages, tool_defs.clone())
                    .await
            } else {
                self.stream_with_thinking_model(model_id, messages).await
            };

            match result {
                Ok(result) => {
                    health.mark_healthy(model_id).await;
                    self.last_token_usage = result.token_usage.clone();
                    if model_id != &primary {
                        let reason = last_err
                            .as_ref()
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "unavailable".to_string());
                        info!("Model fallback: {} → {} ({})", primary, model_id, reason);
                        self.model_id = model_id.clone();
                        self.emit_model_fallback_event(&primary, model_id, &reason).await;
                    }
                    return Ok(result);
                }
                Err(e) if models::is_fallback_worthy_error(&e) => {
                    warn!("Model '{}' error (will fallback): {}", model_id, e);
                    health.mark_error(model_id, &e.to_string()).await;
                    last_err = Some(e);
                    continue;
                }
                Err(e) => {
                    // Non-fallback-worthy error (e.g. network down, bad config) — don't try others.
                    health.mark_error(model_id, &e.to_string()).await;
                    return Err(e);
                }
            }
        }

        // All models exhausted — return the last error.
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("No models available")))
    }

    /// Emit a ModelFallback event via the agent manager.
    async fn emit_model_fallback_event(&self, preferred: &str, actual: &str, reason: &str) {
        let Some(manager) = self.tools.get_manager() else { return };
        let agent_id = self.agent_id.clone().unwrap_or_else(|| "unknown".to_string());
        manager
            .send_event(crate::agent_manager::AgentEvent::ModelFallback {
                agent_id,
                preferred_model: preferred.to_string(),
                actual_model: actual.to_string(),
                reason: reason.to_string(),
            })
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
                })
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
