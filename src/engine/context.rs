use super::types::*;
use crate::ollama::ChatMessage;

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
            .map(|cw| (cw as f64 * 0.75) as usize)
            .unwrap_or(8_000)
    }

    pub(crate) fn context_soft_message_limit(&self) -> usize {
        self.context_window_tokens
            .map(|cw| (cw / 500).clamp(72, 200))
            .unwrap_or(72)
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

    // Persistence + event helpers (writes to session files + emits SSE events)

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
            // Emit to UI immediately (SSE), so structured terminal messages are visible
            // even when no outer chat handler emits an explicit Outcome event.
            manager
                .send_event(crate::agent_manager::AgentEvent::Message {
                    from: agent_id.clone(),
                    to: target.clone(),
                    content: content.to_string(),
                })
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
                .send_event(crate::agent_manager::AgentEvent::StateUpdated)
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
            })
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

    pub(crate) fn maybe_compact_model_messages(
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

        loop {
            let over_budget =
                token_est > soft_token_limit || messages.len() > soft_message_limit;
            if !over_budget || summary_count >= max_summary_passes {
                break;
            }

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

            // Build richer summary from dropped messages.
            let drop_refs: Vec<&ChatMessage> = drop_indices.iter().map(|&i| &window_msgs[i]).collect();
            let summary = self.summarize_message_window_rich(&drop_refs);

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
}
