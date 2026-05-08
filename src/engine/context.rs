use super::types::*;
use crate::message::ChatMessage;
use futures_util::StreamExt;

/// Maximum number of messages to retain in chat_history across turns.
const CHAT_HISTORY_MAX_MESSAGES: usize = 120;

// ---------------------------------------------------------------------------
// Adaptive context window thresholds
// ---------------------------------------------------------------------------

impl AgentEngine {
    /// Trim chat_history to the most recent `CHAT_HISTORY_MAX_MESSAGES` entries.
    pub(crate) fn truncate_chat_history(&mut self) {
        if self.chat_history.len() > CHAT_HISTORY_MAX_MESSAGES {
            let excess = self.chat_history.len() - CHAT_HISTORY_MAX_MESSAGES;
            self.chat_history.drain(..excess);
        }
    }

    pub(crate) fn context_soft_token_limit(&self) -> usize {
        self.context_window_tokens
            .map(|cw| (cw as f64 * 0.95) as usize)
            .unwrap_or(120_000)
    }

    pub(crate) fn context_soft_message_limit(&self) -> usize {
        // Message-count limit is a safety net, not the primary trigger.
        // Primary compaction should be token-driven (aligned with CC at 95%).
        self.context_window_tokens
            .map(|cw| (cw / 200).clamp(200, 800))
            .unwrap_or(200)
    }

    pub(crate) fn context_keep_tail_messages(&self) -> usize {
        self.context_window_tokens
            .map(|cw| ((cw as f64 * 0.15) as usize).max(28))
            .unwrap_or(28)
    }

    pub(crate) fn context_max_summary_passes(&self) -> usize {
        self.context_window_tokens
            .map(|cw| if cw > 100_000 { 5 } else if cw > 32_000 { 4 } else { 3 })
            .unwrap_or(3)
    }

    // Persistence + event helpers (writes to session files + emits UI events)

    pub async fn persist_observation(
        &self,
        tool: &str,
        rendered: &str,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        if let Some(manager) = self.tools.get_manager() {
            let aid = self
                .agent_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            manager
                .add_chat_message(
                    &self.cfg.ws_root,
                    session_id.unwrap_or("default"),
                    &crate::state_fs::sessions::ChatMsg {
                        agent_id: aid.clone(),
                        from_id: "system".to_string(),
                        to_id: aid,
                        content: format!("Tool {}: {}", tool, rendered),
                        timestamp: crate::util::now_ts_secs(),
                        is_observation: true,
                    },
                )
                .await;
        }
        Ok(())
    }

    pub async fn persist_assistant_message(
        &self,
        content: &str,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        if let Some(manager) = self.tools.get_manager() {
            let agent_id = self
                .agent_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let target = self.outbound_target();
            // Emit to UI immediately, so structured messages are visible
            // even when no outer chat handler emits an explicit Outcome event.
            manager
                .send_event(crate::agent_manager::AgentEvent::Message {
                    from: agent_id.clone(),
                    to: target.clone(),
                    content: content.to_string(),
                }, self.session_id.clone())
                .await;
            manager
                .add_chat_message(
                    &self.cfg.ws_root,
                    session_id.unwrap_or("default"),
                    &crate::state_fs::sessions::ChatMsg {
                        agent_id: agent_id.clone(),
                        from_id: agent_id.clone(),
                        to_id: target,
                        content: content.to_string(),
                        timestamp: crate::util::now_ts_secs(),
                        is_observation: false,
                    },
                )
                .await;

            // Nudge UI to refresh immediately.
            manager
                .send_event(crate::agent_manager::AgentEvent::StateUpdated, self.session_id.clone())
                .await;
        }
        Ok(())
    }

    // Token estimation

    pub(crate) fn estimate_tokens_for_text(text: &str) -> usize {
        let chars = text.chars().count();
        if chars == 0 {
            0
        } else {
            (chars + 3) / 4
        }
    }

    pub(crate) fn estimate_chars_for_messages(messages: &[ChatMessage]) -> usize {
        messages.iter().map(|m| m.content.chars().count()).sum()
    }

    pub(crate) fn estimate_tokens_for_messages(messages: &[ChatMessage]) -> usize {
        messages
            .iter()
            .map(|m| Self::estimate_tokens_for_text(&m.content))
            .sum()
    }

    // Message tracking

    /// Push a message to the messages vec with importance tracking.
    /// Updates `message_importance` and `accumulated_token_estimate` in sync.
    pub(crate) fn push_tracked_message(
        &mut self,
        messages: &mut Vec<ChatMessage>,
        msg: ChatMessage,
        importance: MessageImportance,
    ) {
        self.accumulated_token_estimate += Self::estimate_tokens_for_text(&msg.content);
        self.message_importance.push(importance);
        messages.push(msg);
    }

    // Context records

    pub(crate) fn push_context_record(
        &mut self,
        context_type: ContextType,
        name: Option<String>,
        from: Option<String>,
        to: Option<String>,
        content: String,
        meta: serde_json::Value,
    ) {
        let rec = ContextRecord {
            id: self.next_context_id,
            ts: crate::util::now_ts_secs(),
            context_type,
            name,
            from,
            to,
            content,
            meta,
        };
        self.next_context_id = self.next_context_id.saturating_add(1);
        self.context_records.push(rec);
    }

    pub(crate) fn upsert_context_record_by_type_name(
        &mut self,
        context_type: ContextType,
        name: &str,
        from: Option<String>,
        to: Option<String>,
        content: String,
        meta: serde_json::Value,
    ) {
        self.context_records.retain(|existing| {
            if existing.context_type != context_type {
                return true;
            }
            if let Some(existing_name) = &existing.name {
                !existing_name.eq_ignore_ascii_case(name)
            } else {
                true
            }
        });
        self.push_context_record(
            context_type,
            Some(name.to_string()),
            from,
            to,
            content,
            meta,
        );
    }

    // Observations

    pub(crate) fn observation_text(&self, observation_type: &str, name: &str, content: &str) -> String {
        self.prompt_store.render_or_fallback(
            crate::prompts::keys::OBSERVATION_WRAPPER,
            &[("type", observation_type), ("name", name), ("content", content)],
        )
    }

    pub(crate) fn observation_for_model(&self, obs: &ObservationRecord) -> String {
        self.observation_text(&obs.observation_type, &obs.name, &obs.content)
    }

    pub(crate) fn upsert_observation(
        &mut self,
        observation_type: &str,
        name: &str,
        content: String,
    ) {
        let context_type = if observation_type.eq_ignore_ascii_case("tool") {
            ContextType::ToolResult
        } else if observation_type.eq_ignore_ascii_case("error") {
            ContextType::Error
        } else if observation_type.eq_ignore_ascii_case("status") {
            ContextType::Status
        } else if observation_type.eq_ignore_ascii_case("summary") {
            ContextType::Summary
        } else {
            ContextType::Observation
        };
        self.upsert_context_record_by_type_name(
            context_type,
            name,
            Some("system".to_string()),
            self.agent_id.clone(),
            content.clone(),
            serde_json::json!({ "observation_type": observation_type }),
        );
        self.observations.retain(|existing| {
            !(existing
                .observation_type
                .eq_ignore_ascii_case(observation_type)
                && existing.name.eq_ignore_ascii_case(name))
        });
        self.observations.push(ObservationRecord {
            observation_type: observation_type.to_string(),
            name: name.to_string(),
            content,
        });
    }

    // Context usage event

    pub(crate) async fn emit_context_usage_event(
        &self,
        stage: &str,
        messages: &[ChatMessage],
        summary_count: usize,
    ) {
        let Some(manager) = self.tools.get_manager() else {
            return;
        };
        let token_limit = self.context_window_tokens.or_else(|| {
            // Fallback: not cached yet (shouldn't happen after loop start).
            None
        });
        let (actual_prompt, actual_completion) = match &self.last_token_usage {
            Some(u) => (u.prompt_tokens, u.completion_tokens),
            None => (None, None),
        };
        let _ = manager
            .send_event(crate::agent_manager::AgentEvent::ContextUsage {
                agent_id: self
                    .agent_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                stage: stage.to_string(),
                message_count: messages.len(),
                char_count: Self::estimate_chars_for_messages(messages),
                estimated_tokens: Self::estimate_tokens_for_messages(messages),
                token_limit,
                actual_prompt_tokens: actual_prompt,
                actual_completion_tokens: actual_completion,
                compressed: summary_count > 0,
                summary_count,
            }, self.session_id.clone())
            .await;
    }

    // Compaction

    /// Richer summary that extracts structured facts from dropped messages:
    /// file paths mentioned, error messages, tool outcomes.
    fn summarize_message_window_rich(&self, window: &[&ChatMessage]) -> String {
        let mut written_files: Vec<String> = Vec::new();
        let mut read_files: Vec<String> = Vec::new();
        let mut error_snippets: Vec<String> = Vec::new();
        let mut search_count = 0usize;

        for msg in window {
            let content = &msg.content;
            Self::extract_file_paths(content, &mut written_files, &mut read_files);
            Self::extract_error_snippet(content, &mut error_snippets);
            if content.contains("name: Grep") || content.contains("name: Glob") {
                search_count += 1;
            }
        }

        let mut summary = self.prompt_store.render_or_fallback(
            crate::prompts::keys::COMPACTION_SUMMARY,
            &[("count", &window.len().to_string())],
        );
        if !written_files.is_empty() {
            let files: Vec<&str> = written_files.iter().take(10).map(|s| s.as_str()).collect();
            summary.push_str(&format!("\nWrote: {}", files.join(", ")));
        }
        if !read_files.is_empty() {
            let files: Vec<&str> = read_files.iter().take(8).map(|s| s.as_str()).collect();
            summary.push_str(&format!("\nRead: {}", files.join(", ")));
        }
        if search_count > 0 {
            summary.push_str(&format!("\n{} search operations performed.", search_count));
        }
        if !error_snippets.is_empty() {
            summary.push_str(&format!("\n{} error(s):", error_snippets.len()));
            for e in &error_snippets {
                summary.push_str(&format!("\n- {}", e));
            }
        }
        summary
    }

    /// Extract written/edited and read file paths from a message's content.
    fn extract_file_paths(content: &str, written: &mut Vec<String>, read: &mut Vec<String>) {
        let has_writes = content.contains("File written:") || content.contains("Edited file:");
        let has_reads = content.contains("name: Read");
        if !has_writes && !has_reads {
            return;
        }
        for line in content.lines().map(str::trim) {
            if has_writes
                && (line.starts_with("File written:") || line.starts_with("Edited file:"))
            {
                if let Some(path) = line.split_whitespace().last() {
                    let s = path.to_string();
                    if !written.contains(&s) {
                        written.push(s);
                    }
                }
            }
            if has_reads && (line.starts_with("Read:") || line.starts_with("name: Read")) {
                if let Some(path_part) = line.split_whitespace().last() {
                    let path = path_part.trim_end_matches(')').trim_end_matches(',').to_string();
                    if !read.contains(&path) && read.len() < 8 {
                        read.push(path);
                    }
                }
            }
        }
    }

    /// Extract the first error line from content, truncated to 120 chars.
    fn extract_error_snippet(content: &str, snippets: &mut Vec<String>) {
        if snippets.len() >= 5 {
            return;
        }
        if !content.contains("tool_error:") && !content.contains("Error:") {
            return;
        }
        let Some(line) = content.lines().find(|l| l.contains("tool_error:") || l.contains("Error:")) else {
            return;
        };
        let mut short = line.trim().to_string();
        if short.chars().count() > 120 {
            short = short.chars().take(120).collect::<String>() + "...";
        }
        snippets.push(short);
    }

    /// Ensure tool_use (assistant with tool_calls) and tool_result (role="tool")
    /// messages are always kept or dropped together. OpenAI requires every
    /// tool_call to have a matching tool output in the next message.
    fn preserve_tool_pairs(
        window_msgs: &[ChatMessage],
        keep_indices: &mut Vec<usize>,
        drop_indices: &mut Vec<usize>,
    ) {
        use std::collections::HashSet;

        let keep_set: HashSet<usize> = keep_indices.iter().copied().collect();
        let mut promote: HashSet<usize> = HashSet::new();

        // For each kept assistant message with tool_calls, find the tool_result
        // messages that follow it and promote them to keep.
        for &ki in keep_indices.iter() {
            let msg = &window_msgs[ki];
            if msg.role == "assistant" && !msg.tool_calls.is_empty() {
                let tc_ids: HashSet<&str> = msg
                    .tool_calls
                    .iter()
                    .map(|tc| tc.id.as_str())
                    .collect();
                // Scan forward for matching tool results
                for j in (ki + 1)..window_msgs.len() {
                    if let Some(ref tc_id) = window_msgs[j].tool_call_id {
                        if tc_ids.contains(tc_id.as_str()) && !keep_set.contains(&j) {
                            promote.insert(j);
                        }
                    }
                    // Stop once we hit another assistant message
                    if window_msgs[j].role == "assistant" {
                        break;
                    }
                }
            }
        }

        // For each kept tool_result, ensure its parent assistant message is also kept.
        for &ki in keep_indices.iter() {
            let msg = &window_msgs[ki];
            if let Some(ref tc_id) = msg.tool_call_id {
                // Scan backward for the assistant message with this tool_call_id
                for j in (0..ki).rev() {
                    if window_msgs[j].role == "assistant"
                        && window_msgs[j]
                            .tool_calls
                            .iter()
                            .any(|tc| tc.id == *tc_id)
                        && !keep_set.contains(&j)
                    {
                        promote.insert(j);
                        // Also promote all other tool_results for this assistant
                        let all_tc_ids: HashSet<&str> = window_msgs[j]
                            .tool_calls
                            .iter()
                            .map(|tc| tc.id.as_str())
                            .collect();
                        for k in (j + 1)..window_msgs.len() {
                            if let Some(ref tid) = window_msgs[k].tool_call_id {
                                if all_tc_ids.contains(tid.as_str()) && !keep_set.contains(&k) {
                                    promote.insert(k);
                                }
                            }
                            if window_msgs[k].role == "assistant" {
                                break;
                            }
                        }
                        break;
                    }
                }
            }
        }

        if !promote.is_empty() {
            drop_indices.retain(|i| !promote.contains(i));
            keep_indices.extend(promote);
            keep_indices.sort_unstable();
        }
    }

    pub(crate) async fn maybe_compact_model_messages(
        &mut self,
        messages: &mut Vec<ChatMessage>,
        stage: &str,
    ) -> usize {
        let mut summary_count = 0usize;
        let soft_token_limit = self.context_soft_token_limit();
        let soft_message_limit = self.context_soft_message_limit();
        let max_summary_passes = self.context_max_summary_passes();
        let keep_tail = self.context_keep_tail_messages();

        // Sync importance vector length (safety net for untagged pushes).
        while self.message_importance.len() < messages.len() {
            self.message_importance.push(MessageImportance::Normal);
        }

        // Use accumulated token estimate if available, fall back to full scan.
        let mut token_est = if self.accumulated_token_estimate > 0 {
            self.accumulated_token_estimate
        } else {
            Self::estimate_tokens_for_messages(messages)
        };

        tracing::debug!(
            "[compact] stage={stage} msgs={} token_est={token_est} soft_token_limit={soft_token_limit} \
             soft_msg_limit={soft_message_limit} context_window={:?}",
            messages.len(), self.context_window_tokens,
        );

        loop {
            let over_tokens = token_est > soft_token_limit;
            let over_messages = messages.len() > soft_message_limit;
            if (!over_tokens && !over_messages) || summary_count >= max_summary_passes {
                break;
            }
            tracing::info!(
                "[compact] triggered: over_tokens={over_tokens} over_messages={over_messages} \
                 tokens={token_est}/{soft_token_limit} msgs={}/{soft_message_limit}",
                messages.len(),
            );

            if messages.len() <= keep_tail + 2 {
                break;
            }

            let start = 1usize; // Keep the leading system prompt.
            let end = messages.len().saturating_sub(keep_tail);
            if end <= start {
                break;
            }

            // Importance-aware compaction: partition the middle window into
            // messages to summarize (Low+Normal) and messages to keep (High+Critical).
            let window_msgs = &messages[start..end];
            let window_imp = &self.message_importance[start..end];

            let mut keep_indices: Vec<usize> = Vec::new();
            let mut drop_indices: Vec<usize> = Vec::new();
            for (i, imp) in window_imp.iter().enumerate() {
                if *imp >= MessageImportance::High {
                    keep_indices.push(i);
                } else {
                    drop_indices.push(i);
                }
            }

            // Preserve tool_use ↔ tool_result pairing: if an assistant message
            // with tool_calls is kept, its tool_result messages must also be kept
            // (and vice versa). Otherwise OpenAI errors with "No tool output found".
            Self::preserve_tool_pairs(window_msgs, &mut keep_indices, &mut drop_indices);

            // If nothing to drop, fall back to dropping everything in window
            // (original behavior) — the model needs space.
            if drop_indices.is_empty() {
                drop_indices = (0..window_msgs.len()).collect();
                keep_indices.clear();
            }

            let dropped_messages = drop_indices.len();
            let dropped_tokens: usize = drop_indices
                .iter()
                .map(|&i| Self::estimate_tokens_for_text(&window_msgs[i].content))
                .sum();
            let dropped_chars: usize = drop_indices
                .iter()
                .map(|&i| window_msgs[i].content.chars().count())
                .sum();

            // Summarize dropped messages using the model (CC-aligned).
            // Falls back to deterministic summary if model call fails.
            let drop_refs: Vec<&ChatMessage> = drop_indices.iter().map(|&i| &window_msgs[i]).collect();
            let summary = self.summarize_with_model(&drop_refs, None).await;

            // Collect kept high-importance messages.
            let kept_msgs: Vec<ChatMessage> = keep_indices
                .iter()
                .map(|&i| window_msgs[i].clone())
                .collect();
            let kept_imp: Vec<MessageImportance> = keep_indices
                .iter()
                .map(|&i| window_imp[i])
                .collect();

            // Remove the entire window, replace with summary + kept messages.
            messages.drain(start..end);
            self.message_importance.drain(start..end);

            // Insert summary first, then kept messages (in original order).
            messages.insert(start, ChatMessage::new("user", summary.clone()));
            self.message_importance.insert(start, MessageImportance::Normal);

            for (offset, (msg, imp)) in kept_msgs.into_iter().zip(kept_imp).enumerate() {
                messages.insert(start + 1 + offset, msg);
                self.message_importance.insert(start + 1 + offset, imp);
            }

            // Recompute token estimate after compaction.
            token_est = Self::estimate_tokens_for_messages(messages);

            summary_count += 1;
            self.push_context_record(
                ContextType::Summary,
                Some(format!("{}_summary_{}", stage, summary_count)),
                Some("system".to_string()),
                self.agent_id.clone(),
                summary,
                serde_json::json!({
                    "stage": stage,
                    "dropped_messages": dropped_messages,
                    "dropped_chars": dropped_chars,
                    "dropped_estimated_tokens": dropped_tokens,
                    "kept_high_importance": keep_indices.len()
                }),
            );
        }

        // Reset accumulated estimate after compaction.
        self.accumulated_token_estimate = token_est;

        summary_count
    }

    /// Force-compact the context, regardless of whether we're over budget.
    /// Used by the `/compact` command. Returns the summary text, or None if
    /// there's nothing to compact.
    pub(crate) async fn force_compact(
        &mut self,
        messages: &mut Vec<ChatMessage>,
        focus: Option<&str>,
    ) -> Option<String> {
        let keep_tail = self.context_keep_tail_messages().min(messages.len().saturating_sub(2));

        // Sync importance vector.
        while self.message_importance.len() < messages.len() {
            self.message_importance.push(MessageImportance::Normal);
        }

        let start = 1usize; // Keep system prompt.
        let end = messages.len().saturating_sub(keep_tail);
        if end <= start {
            return None; // Nothing to compact.
        }

        let window_msgs = &messages[start..end];
        let window_imp = &self.message_importance[start..end];

        let mut keep_indices: Vec<usize> = Vec::new();
        let mut drop_indices: Vec<usize> = Vec::new();
        for (i, imp) in window_imp.iter().enumerate() {
            if *imp >= MessageImportance::High {
                keep_indices.push(i);
            } else {
                drop_indices.push(i);
            }
        }
        Self::preserve_tool_pairs(window_msgs, &mut keep_indices, &mut drop_indices);

        if drop_indices.is_empty() {
            drop_indices = (0..window_msgs.len()).collect();
            keep_indices.clear();
        }

        let drop_refs: Vec<&ChatMessage> = drop_indices.iter().map(|&i| &window_msgs[i]).collect();
        let summary = self.summarize_with_model(&drop_refs, focus).await;

        let kept_msgs: Vec<ChatMessage> = keep_indices.iter().map(|&i| window_msgs[i].clone()).collect();
        let kept_imp: Vec<MessageImportance> = keep_indices.iter().map(|&i| window_imp[i]).collect();

        messages.drain(start..end);
        self.message_importance.drain(start..end);

        messages.insert(start, ChatMessage::new("user", summary.clone()));
        self.message_importance.insert(start, MessageImportance::Normal);

        for (offset, (msg, imp)) in kept_msgs.into_iter().zip(kept_imp).enumerate() {
            messages.insert(start + 1 + offset, msg);
            self.message_importance.insert(start + 1 + offset, imp);
        }

        self.accumulated_token_estimate = Self::estimate_tokens_for_messages(messages);

        Some(summary)
    }

    /// Summarize dropped messages by sending them to the model.
    /// Falls back to `summarize_message_window_rich` if the model call fails.
    async fn summarize_with_model(&self, dropped: &[&ChatMessage], focus: Option<&str>) -> String {
        // Build a compact transcript of the dropped messages for the model.
        let mut transcript = String::new();
        for msg in dropped {
            let role = &msg.role;
            let content = &msg.content;
            // Truncate very long messages to avoid blowing up the summarization context.
            let truncated = if content.len() > 2000 {
                format!("{}...[truncated]", &content[..2000])
            } else {
                content.clone()
            };
            transcript.push_str(&format!("[{}] {}\n", role, truncated));
        }

        let focus_instruction = match focus {
            Some(f) if !f.is_empty() => format!("\n\nIMPORTANT: Focus especially on: {}\n", f),
            _ => String::new(),
        };

        let prompt = format!(
            "You are summarizing a conversation between a user and a coding assistant. \
             The following {} messages are being compacted to free up context space.\n\n\
             Summarize them into a concise working state that preserves:\n\
             - What task the user asked for and current progress\n\
             - Key decisions made and why\n\
             - Files created, modified, or read (with paths)\n\
             - Errors encountered and how they were resolved\n\
             - Any pending work or next steps\n\n\
             Be concise but complete. Use bullet points. Do not lose important details \
             like file paths, function names, or error messages.{}\n\n\
             --- MESSAGES TO SUMMARIZE ---\n{}",
            dropped.len(),
            focus_instruction,
            transcript
        );

        let summarize_msgs = vec![
            ChatMessage::new("system", "You are a context compaction assistant. Produce a concise summary."),
            ChatMessage::new("user", prompt),
        ];

        // Try model-based summarization.
        match self.model_manager.chat_text_stream(&self.model_id, &summarize_msgs).await {
            Ok(mut stream) => {
                let mut result = String::new();
                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(crate::agent_manager::models::StreamChunk::Token(t)) => result.push_str(&t),
                        Ok(_) => {} // Usage stats, tool calls — ignore
                        Err(e) => {
                            tracing::warn!("Compaction stream error, falling back to deterministic: {e}");
                            return self.summarize_message_window_rich(dropped);
                        }
                    }
                }
                if result.trim().is_empty() {
                    tracing::warn!("Compaction model returned empty response, falling back");
                    return self.summarize_message_window_rich(dropped);
                }
                format!("[Context compacted — {} messages summarized by model]\n\n{}", dropped.len(), result.trim())
            }
            Err(e) => {
                tracing::warn!("Compaction model call failed, falling back to deterministic: {e}");
                self.summarize_message_window_rich(dropped)
            }
        }
    }
}
