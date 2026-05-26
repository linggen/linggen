use super::types::*;
use crate::message::ChatMessage;
use futures_util::StreamExt;

// ---------------------------------------------------------------------------
// Adaptive context window thresholds
// ---------------------------------------------------------------------------
//
// chat_history is intentionally unbounded in memory — messages.jsonl is the
// durable source of truth and context is bounded by compaction, not by a
// blunt summary-less cap. See doc/compaction-spec.md.

impl AgentEngine {

    pub(crate) fn context_soft_token_limit(&self) -> usize {
        // Three layers, highest to lowest:
        //   1. Per-session override (POST /api/chat/compact_config)
        //   2. Global default from app config (`agent.compact_threshold` in linggen.toml)
        //   3. Hardcoded engine default (0.95)
        // Range clamp keeps wild values from breaking the trigger.
        let frac = self.compact_threshold
            .or(self.cfg.compact_threshold_default)
            .map(|t| t.clamp(0.1, 0.99) as f64)
            .unwrap_or(0.95);
        self.context_window_tokens
            .map(|cw| (cw as f64 * frac) as usize)
            .unwrap_or(120_000)
    }

    pub(crate) fn context_soft_message_limit(&self) -> usize {
        // Message-count limit is a safety net, not the primary trigger.
        // Primary compaction should be token-driven (aligned with CC at 95%).
        self.context_window_tokens
            .map(|cw| (cw / 200).clamp(200, 800))
            .unwrap_or(200)
    }

    /// Token budget for the verbatim recent tail kept after a summary.
    /// A token budget — not a message count. The previous impl compared a
    /// token-derived value against `messages.len()`, so the window guard
    /// never opened; see doc/compaction-spec.md.
    pub(crate) fn context_tail_token_budget(&self) -> usize {
        self.context_window_tokens
            .map(|cw| (cw as f64 * 0.15) as usize)
            .unwrap_or(16_000)
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
            // Subagents share the parent's `session_id`, so a raw write
            // would dump every subagent's tool observations into the
            // parent's messages.jsonl — those then leak back into the
            // chat on replay. The subagent's activity is already surfaced
            // through the SubagentSpawned / Activity / SubagentResult
            // event stream (rendered inside the subagent tree widget),
            // so skipping the persist here is non-lossy for the UI.
            if self.tools.builtins.delegation_depth() > 0 {
                return Ok(());
            }
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
            // Include run_id + parent_id so the UI can route subagent
            // messages into SubagentPane instead of leaking them into the
            // parent's main chat — these fields are None for top-level
            // (depth-0) messages and routing falls back to main chat as
            // before.
            let is_subagent = self.tools.builtins.delegation_depth() > 0;
            let run_id = if is_subagent { self.run_id.clone() } else { None };
            let parent_id = if is_subagent { self.parent_agent_id.clone() } else { None };
            manager
                .send_event(crate::engine::agent::AgentEvent::Message {
                    from: agent_id.clone(),
                    to: target.clone(),
                    content: content.to_string(),
                    run_id,
                    parent_id,
                }, self.session_id.clone())
                .await;
            // Subagents inherit the parent's session_id. Persisting their
            // terminal message into the parent's messages.jsonl was making
            // contract status lines (e.g. the encoder's `ENCODED
            // encoded=0`) show up as their own chat bubbles on replay —
            // visible noise in the parent's transcript. The Message event
            // above already carries the text to the UI's subagent-tree
            // widget; SubagentResult also re-carries it as `outcome` for
            // event-ordering safety. So the on-disk persist is the only
            // step that needs to skip for delegated runs.
            let is_subagent = self.tools.builtins.delegation_depth() > 0;
            if !is_subagent {
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
                    .send_event(crate::engine::agent::AgentEvent::StateUpdated, self.session_id.clone())
                    .await;
            }
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

    /// Push a message to the messages vec, keeping `accumulated_token_estimate`
    /// in sync so the compaction trigger needn't re-scan every iteration.
    pub(crate) fn push_tracked_message(
        &mut self,
        messages: &mut Vec<ChatMessage>,
        msg: ChatMessage,
    ) {
        self.accumulated_token_estimate += Self::estimate_tokens_for_text(&msg.content);
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
            .send_event(crate::engine::agent::AgentEvent::ContextUsage {
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
    //
    // Two tiers, aligned with Claude Code (see doc/compaction-spec.md):
    //   Tier 1 — evict stale tool_result bodies in place (cheap, no model call)
    //   Tier 2 — one structured-summary pass over the middle of the transcript
    // No per-message importance, no tool-pair reconciliation, no multi-pass loop.

    const TOOL_EVICTED_PLACEHOLDER: &'static str =
        "[tool output evicted to free context — re-run the tool if its result is needed]";

    /// Number of leading messages that are never compacted: 1 if a system
    /// prompt sits at index 0 (the auto path, `[system] + chat_history`),
    /// else 0 (the `/compact` path passes raw `chat_history`).
    fn head_len(messages: &[ChatMessage]) -> usize {
        match messages.first() {
            Some(m) if m.role == "system" => 1,
            _ => 0,
        }
    }

    /// Tier 1: replace the body of older `tool` (tool_result) messages with a
    /// short placeholder. Only `content` changes — message structure and
    /// tool_use↔tool_result pairing are untouched, so this is always safe.
    /// The protected head and everything from `protect_from` onward (the
    /// verbatim recent tail) are left intact. Returns tokens reclaimed.
    fn evict_old_tool_results(messages: &mut [ChatMessage], head: usize, protect_from: usize) -> usize {
        let placeholder_tokens = Self::estimate_tokens_for_text(Self::TOOL_EVICTED_PLACEHOLDER);
        let upper = protect_from.min(messages.len());
        let mut reclaimed = 0usize;
        for msg in messages.iter_mut().take(upper).skip(head) {
            if msg.role != "tool" || msg.content == Self::TOOL_EVICTED_PLACEHOLDER {
                continue;
            }
            let before = Self::estimate_tokens_for_text(&msg.content);
            if before <= placeholder_tokens {
                continue;
            }
            reclaimed += before - placeholder_tokens;
            msg.content = Self::TOOL_EVICTED_PLACEHOLDER.to_string();
        }
        reclaimed
    }

    /// Index where the verbatim recent tail begins. Walk back from the end
    /// accumulating token estimates until `tail_budget` is exceeded, never
    /// crossing the protected head. Then:
    ///  - pull the boundary back to the last assistant message so the
    ///    freshest exchange (the result the model just asked for) is always
    ///    kept verbatim, even if it alone exceeds `tail_budget`; otherwise a
    ///    single huge tool result would be evicted before the model sees it;
    ///  - advance past any leading `tool` messages so the tail never starts
    ///    with an orphaned tool_result whose tool_use was summarized away —
    ///    this is why no separate tool-pair reconciliation is needed.
    fn tail_start_index(messages: &[ChatMessage], head: usize, tail_budget: usize) -> usize {
        let mut acc = 0usize;
        let mut i = messages.len();
        while i > head {
            let t = Self::estimate_tokens_for_text(&messages[i - 1].content);
            if acc + t > tail_budget {
                break;
            }
            acc += t;
            i -= 1;
        }
        let mut start = i.max(head);
        if let Some(la) = messages.iter().rposition(|m| m.role == "assistant") {
            start = start.min(la).max(head);
        }
        while start < messages.len() && messages[start].role == "tool" {
            start += 1;
        }
        start
    }

    /// Auto-compaction, run once per loop iteration. Token-driven trigger
    /// (see `context_soft_token_limit`). Tier 1 first (cheap, in place); only
    /// if still over budget does Tier 2 (one structured summary) run. Returns
    /// the number of summaries produced (0 or 1).
    pub(crate) async fn maybe_compact_model_messages(
        &mut self,
        messages: &mut Vec<ChatMessage>,
        stage: &str,
    ) -> usize {
        let soft_token_limit = self.context_soft_token_limit();
        let soft_message_limit = self.context_soft_message_limit();

        let mut token_est = if self.accumulated_token_estimate > 0 {
            self.accumulated_token_estimate
        } else {
            Self::estimate_tokens_for_messages(messages)
        };

        let over = |t: usize, n: usize| t > soft_token_limit || n > soft_message_limit;
        if !over(token_est, messages.len()) {
            return 0;
        }

        tracing::info!(
            "[compact] stage={stage} triggered: tokens={token_est}/{soft_token_limit} \
             msgs={}/{soft_message_limit}",
            messages.len(),
        );

        // Tier 1: evict stale tool_result bodies before the verbatim tail.
        let head = Self::head_len(messages);
        let tail_start = Self::tail_start_index(messages, head, self.context_tail_token_budget());
        let reclaimed = Self::evict_old_tool_results(messages, head, tail_start);
        if reclaimed > 0 {
            token_est = Self::estimate_tokens_for_messages(messages);
            tracing::info!("[compact] tier1 reclaimed ~{reclaimed} tokens, now {token_est}");
        }

        // Tier 2: if still over budget, one structured-summary pass.
        let mut summary_count = 0usize;
        if over(token_est, messages.len()) {
            let focus = self.compact_focus.clone();
            if self.compact_once(messages, stage, focus).await.is_some() {
                summary_count = 1;
                token_est = Self::estimate_tokens_for_messages(messages);
                tracing::info!(
                    "[compact] tier2 summarized; now tokens={token_est} msgs={}",
                    messages.len(),
                );
            }
        }

        self.accumulated_token_estimate = token_est;
        summary_count
    }

    /// Tier 2 core: replace everything between the system prompt and the
    /// verbatim recent tail with one structured summary. `focus` (the
    /// per-session, persisted `compact_focus` for the auto path, or the
    /// caller's override for `/compact`) is fed to the summary prompt.
    /// Returns the summary text, or None when there is nothing to summarize
    /// or the model summary failed — in which case the caller keeps the
    /// uncompacted messages rather than silently degrading.
    async fn compact_once(
        &mut self,
        messages: &mut Vec<ChatMessage>,
        stage: &str,
        focus: Option<String>,
    ) -> Option<String> {
        // Keep the protected head (system prompt on the auto path; nothing on
        // the `/compact` path, which passes raw chat_history).
        let head = Self::head_len(messages);
        let tail_start = Self::tail_start_index(messages, head, self.context_tail_token_budget());
        if tail_start <= head {
            return None;
        }

        let dropped: Vec<&ChatMessage> = messages[head..tail_start].iter().collect();
        let dropped_messages = dropped.len();
        let dropped_chars: usize = dropped.iter().map(|m| m.content.chars().count()).sum();
        let dropped_tokens: usize = dropped
            .iter()
            .map(|m| Self::estimate_tokens_for_text(&m.content))
            .sum();

        let summary = self.summarize_span(&dropped, focus.as_deref()).await?;

        messages.drain(head..tail_start);
        messages.insert(head, ChatMessage::new("user", summary.clone()));

        self.push_context_record(
            ContextType::Summary,
            Some(format!("{}_summary", stage)),
            Some("system".to_string()),
            self.agent_id.clone(),
            summary.clone(),
            serde_json::json!({
                "stage": stage,
                "dropped_messages": dropped_messages,
                "dropped_chars": dropped_chars,
                "dropped_estimated_tokens": dropped_tokens,
            }),
        );
        Some(summary)
    }

    /// Force-compact regardless of token budget. Backs the `/compact`
    /// command. Thin wrapper over the Tier-2 path; `focus` overrides the
    /// per-session `compact_focus` for this one call only.
    pub(crate) async fn force_compact(
        &mut self,
        messages: &mut Vec<ChatMessage>,
        focus: Option<&str>,
    ) -> Option<String> {
        let effective_focus = focus
            .filter(|f| !f.is_empty())
            .map(str::to_string)
            .or_else(|| self.compact_focus.clone());
        let result = self.compact_once(messages, "force", effective_focus).await;
        if result.is_some() {
            self.accumulated_token_estimate = Self::estimate_tokens_for_messages(messages);
        }
        result
    }

    /// Summarize a span of messages into a structured working-state summary
    /// via the model. Returns None if the model call fails or yields nothing
    /// — the caller then leaves the transcript uncompacted rather than
    /// silently degrading to a low-fidelity extract.
    async fn summarize_span(
        &self,
        dropped: &[&ChatMessage],
        focus: Option<&str>,
    ) -> Option<String> {
        if dropped.is_empty() {
            return None;
        }
        let mut transcript = String::new();
        for msg in dropped {
            let content = if msg.content.chars().count() > 2000 {
                let head: String = msg.content.chars().take(2000).collect();
                format!("{head}...[truncated]")
            } else {
                msg.content.clone()
            };
            transcript.push_str(&format!("[{}] {}\n", msg.role, content));
        }

        let focus_instruction = match focus {
            Some(f) if !f.is_empty() => format!("\n\nIMPORTANT: Focus especially on: {}\n", f),
            _ => String::new(),
        };

        let prompt = format!(
            "You are summarizing a conversation between a user and a coding assistant. \
             The following {} messages are being compacted to free up context space.\n\n\
             Summarize them into a concise working state that preserves:\n\
             - What the user asked for and current progress\n\
             - Key decisions made and why\n\
             - Files created, modified, or read (with paths)\n\
             - Errors encountered and how they were resolved\n\
             - Pending work and the next step\n\n\
             Be concise but complete. Use bullet points. Do not lose file paths, \
             function names, or error messages.{}\n\n\
             --- MESSAGES TO SUMMARIZE ---\n{}",
            dropped.len(),
            focus_instruction,
            transcript
        );

        let summarize_msgs = vec![
            ChatMessage::new(
                "system",
                "You are a context compaction assistant. Produce a concise summary.",
            ),
            ChatMessage::new("user", prompt),
        ];

        match self
            .model_manager
            .chat_text_stream(&self.model_id, &summarize_msgs)
            .await
        {
            Ok(mut stream) => {
                let mut result = String::new();
                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(crate::provider::models::StreamChunk::Token(t)) => {
                            result.push_str(&t)
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!(
                                "[compact] summary stream error, skipping compaction: {e}"
                            );
                            return None;
                        }
                    }
                }
                let result = result.trim();
                if result.is_empty() {
                    tracing::warn!("[compact] summary model returned empty, skipping compaction");
                    return None;
                }
                Some(format!(
                    "[Context compacted — {} messages summarized]\n\n{}",
                    dropped.len(),
                    result
                ))
            }
            Err(e) => {
                tracing::warn!("[compact] summary model call failed, skipping compaction: {e}");
                None
            }
        }
    }
}
