pub mod actions;
mod context;
pub(crate) mod core_memory;
pub mod memory;
pub mod patch;
pub mod permission;
mod plan;
mod prompt;
pub mod prompt_profile;
pub mod render;
pub mod session_policy;
pub mod skill_tool;
mod streaming;
pub mod tool_registry;
mod dispatch;
mod tool_exec;
pub mod tools;
mod types;
pub mod web_fetch;
pub mod web_search;

// Re-export public API types
pub use types::{
    AgentEngine, AgentOutcome, AgentRole, ContextRecord, ContextType, EngineConfig,
    InterfaceMode, Plan, PlanStatus, ThinkingEvent,
};

pub use actions::{
    looks_like_final_answer, model_message_log_parts, parse_all_actions, text_before_first_json,
    ModelAction,
};

// Internal imports used by run_agent_loop
use streaming::{can_parallel_tool, has_write_path_conflicts};
use types::{LoopControl, LoopState, MessageImportance, ParsedToolCall};

use crate::ollama::ChatMessage;
use anyhow::Result;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use tracing::{debug, info, warn};

impl AgentEngine {
    /// Convert native ParsedToolCalls into ModelActions for dispatch.
    /// "Done" calls are filtered out — the loop stops when no tool calls remain.
    fn tool_calls_to_actions(tool_calls: &[ParsedToolCall]) -> Vec<ModelAction> {
        tool_calls
            .iter()
            .filter_map(|tc| {
                match tc.name.as_str() {
                    "" => {
                        // Empty tool name — skip (some models produce phantom calls)
                        tracing::warn!("Skipping native tool call with empty name");
                        None
                    }
                    "Done" => {
                        // No longer a real tool — ignore. Loop stops when no tool calls.
                        None
                    }
                    _ => {
                        Some(ModelAction {
                            tool: tc.name.clone(),
                            args: tc.arguments.clone(),
                        })
                    }
                }
            })
            .collect()
    }

    pub async fn run_agent_loop(&mut self, session_id: Option<&str>) -> Result<AgentOutcome> {
        self.session_id = session_id.map(|s| s.to_string());

        // Load session permissions from permission.json (or initialize defaults).
        if let Some(sid) = session_id {
            let sdir = crate::paths::global_sessions_dir().join(sid);
            self.session_permissions = permission::SessionPermissions::load(&sdir);

            // Consumer/mission sessions must keep their non-interactive policy
            // even after reload from disk. SessionPolicy::apply() set the
            // policy before the loop starts, but SessionPermissions::load()
            // just overwrote it. Re-apply Trusted (no prompts, ask-rules
            // denied) if consumer/mission restrictions are active.
            if self.cfg.consumer_allowed_tools.is_some() || self.cfg.mission_allowed_tools.is_some() {
                self.session_permissions
                    .set_policy(permission::PermissionPolicy::trusted());
            } else if self.session_permissions.policy == permission::PermissionPolicy::default() {
                // First-time load (permission.json absent or at defaults) —
                // apply the user's configured default policy from
                // `[agent] default_policy`. Skill/mission/consumer sessions
                // already picked their own policy above, so this only affects
                // fresh user chats.
                if let Some(ref name) = self.cfg.default_policy {
                    self.session_permissions
                        .set_policy(permission::PermissionPolicy::from_preset(name));
                }
            }

            // Initialize default path_modes if empty (new session = read on cwd).
            if self.session_permissions.path_modes.is_empty()
                && !self.session_permissions.policy.is_locked()
            {
                let actual_cwd = self.tools.builtins.cwd();
                let cwd_str = if let Some(home) = dirs::home_dir() {
                    let ws = actual_cwd.to_string_lossy();
                    let hs = home.to_string_lossy();
                    if ws.starts_with(hs.as_ref()) {
                        format!("~{}", &ws[hs.len()..])
                    } else {
                        ws.to_string()
                    }
                } else {
                    actual_cwd.to_string_lossy().to_string()
                };
                self.session_permissions.set_path_mode(
                    &cwd_str,
                    self.cfg.permission_mode.clone(),
                );
                self.session_permissions.save(&sdir);
            }

            self.session_dir = Some(sdir);
        }

        if self.is_cancelled().await {
            anyhow::bail!("run cancelled");
        }

        if let Some(manager) = self.tools.get_manager() {
            let agent_id = self
                .agent_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            manager
                .send_event(crate::agent_manager::AgentEvent::AgentStatus {
                    agent_id,
                    status: "working".to_string(),
                    detail: Some("Running".to_string()),
                    parent_id: self.parent_agent_id.clone(),
                    run_id: self.run_id.clone(),
                    parent_run_id: self.parent_run_id.clone(),
                }, self.session_id.clone())
                .await;
        }

        // Sync world state before running the loop if we have a manager
        if let Some(manager) = self.tools.get_manager() {
            let _ = manager.sync_world_state(&self.cfg.ws_root).await;
        }

        // Set up progress channel for streaming Bash output.
        let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel::<(String, String, String)>();
        self.tools.builtins.set_progress_tx(progress_tx);

        let Some(task) = self.task.clone() else {
            anyhow::bail!("no task set; use /task <text>");
        };

        let log_run = self
            .run_id
            .clone()
            .unwrap_or_else(|| "root".to_string());
        // Task strings can be 100+ lines (e.g. MEMORY_EXTRACT embeds the full
        // dashboard-page-layout prompt). Logging the whole thing at INFO
        // overwhelms the log. Show the first non-blank line + size so a
        // human can match run IDs to tasks; full text remains at DEBUG.
        let task_summary = summarize_task_for_log(task.as_ref());
        info!(
            "[{}] Starting agent loop for role {:?} (depth={}) task: {}",
            log_run,
            self.role,
            self.tools.builtins.delegation_depth(),
            task_summary,
        );
        tracing::debug!("[{}] Full task: {}", log_run, task);
        self.push_context_record(
            ContextType::Status,
            Some("autonomous_loop_start".to_string()),
            self.agent_id.clone(),
            None,
            format!("Starting autonomous loop for task: {}", task),
            serde_json::json!({ "mode": "structured" }),
        );

        // Record start of loop in session store
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
                        content: format!("Starting autonomous loop for task: {}", task),
                        timestamp: crate::util::now_ts_secs(),
                        is_observation: true,
                    },
                )
                .await;
        }

        // Query and cache the model's context window for adaptive thresholds.
        if self.context_window_tokens.is_none() {
            self.context_window_tokens = self
                .model_manager
                .context_window(&self.model_id)
                .await
                .ok()
                .flatten();
            if let Some(cw) = self.context_window_tokens {
                debug!("Context window: {}t, soft_limit={}, keep_tail={}, max_passes={}",
                    cw, self.context_soft_token_limit(), self.context_keep_tail_messages(), self.context_max_summary_passes());
            }
        }

        let use_native_tools = self.model_manager.supports_tools(&self.model_id);
        info!(
            "[{}] Agent loop: model_id={}, native_tools={}",
            log_run, self.model_id, use_native_tools
        );
        let (messages, allowed_tools, read_paths) =
            self.prepare_loop_messages(&task, use_native_tools);

        // Initialize importance tags: system=Critical, everything else=High.
        self.message_importance = Vec::with_capacity(messages.len() + 128);
        for (i, msg) in messages.iter().enumerate() {
            let importance = if i == 0 {
                MessageImportance::Critical // system prompt
            } else if msg.role == "user" && msg.content.contains("Autonomous agent loop started") {
                MessageImportance::Critical // user task message
            } else {
                MessageImportance::High // chat history, observations
            };
            self.message_importance.push(importance);
        }
        // Initialize accumulated token estimate from current messages.
        self.accumulated_token_estimate = Self::estimate_tokens_for_messages(&messages);

        let mut state = LoopState {
            messages,
            allowed_tools,
            read_paths,
            tool_cache: HashMap::new(),
            empty_search_streak: 0,
            redundant_tool_streak: 0,
            last_tool_sig: String::new(),
            invalid_json_streak: 0,
            last_assistant_response: String::new(),
            identical_response_streak: 0,
            loop_nudge_count: 0,
            empty_response_streak: 0,
            progress_rx,
        };
        self.native_tool_mode = use_native_tools;

        let mut interrupted_by_user = false;
        for _ in 0..self.cfg.max_iters {
            if self.is_cancelled().await {
                anyhow::bail!("run cancelled");
            }

            // If the user sent a new message while we were working, break the
            // loop and let the queued message start a fresh chat round. The
            // model will see the full conversation history and decide whether
            // to continue the previous task or handle the new request.
            if let Some(rx) = &mut self.interrupt_rx {
                if rx.try_recv().is_ok() {
                    info!("Breaking loop: user sent a new message");
                    interrupted_by_user = true;
                    break;
                }
            }

            if let Some(manager) = self.tools.get_manager() {
                let agent_id = self
                    .agent_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string());
                manager
                    .send_event(crate::agent_manager::AgentEvent::AgentStatus {
                        agent_id,
                        status: "thinking".to_string(),
                        detail: Some(format!("Thinking ({})", self.model_id)),
                        parent_id: self.parent_agent_id.clone(),
                        run_id: self.run_id.clone(),
                        parent_run_id: self.parent_run_id.clone(),
                    }, self.session_id.clone())
                    .await;
            }

            // Skip compaction while executing an approved plan — the model
            // needs full tool-result context to track progress against plan steps.
            let executing_plan = self.plan.as_ref()
                .map(|p| p.status == PlanStatus::Executing || p.status == PlanStatus::Approved)
                .unwrap_or(false);
            let summary_count = if executing_plan {
                0
            } else {
                self.maybe_compact_model_messages(&mut state.messages, "loop_iter").await
            };
            self.emit_context_usage_event("loop_iter", &state.messages, summary_count)
                .await;

            // Determine whether to use native tool calling for this model.
            let native_tools = if self.model_manager.supports_tools(&self.model_id) {
                Some(self.tools.oai_tool_definitions(state.allowed_tools.as_ref()))
            } else {
                None
            };

            // Ask model for the next action, streaming thinking tokens.
            let stream_result = self.stream_with_fallback(&state.messages, native_tools.clone()).await?;
            let raw = stream_result.full_text;
            let stream_first_action = stream_result.first_action;
            let native_tool_calls = stream_result.tool_calls;

            // Log model output: text + json (truncated).
            let (text_part, json_part) = model_message_log_parts(&raw, 100, 100);
            let json_rendered = json_part
                .as_ref()
                .and_then(|v| serde_json::to_string(v).ok())
                .unwrap_or_else(|| "null".to_string());
            if !native_tool_calls.is_empty() {
                debug!(
                    "[{}] Model response (native): text='{}' tool_calls={}",
                    log_run,
                    text_part.replace('\n', "\\n"),
                    native_tool_calls.len()
                );
            } else {
                debug!(
                    "[{}] Model response: text='{}' json={}",
                    log_run,
                    text_part.replace('\n', "\\n"),
                    json_rendered
                );
            }

            // --- Empty response protection ---
            // Some providers (e.g. Gemini) may return empty text with no tool calls.
            // Bail out after consecutive empty responses to avoid infinite loops.
            if raw.trim().is_empty() && native_tool_calls.is_empty() {
                state.empty_response_streak += 1;
                warn!(
                    "Empty model response (streak {}): model={} native_tools={}",
                    state.empty_response_streak,
                    self.model_id,
                    native_tools.is_some(),
                );
                if state.empty_response_streak >= 3 {
                    warn!("Bailing out after {} consecutive empty responses from {}", state.empty_response_streak, self.model_id);

                    // In plan mode, auto-finalize with whatever research was gathered
                    // instead of showing an error. The user can still see and approve the plan.
                    if self.plan_mode {
                        let plan_text = if !state.last_assistant_response.is_empty() {
                            state.last_assistant_response.clone()
                        } else {
                            format!(
                                "Plan could not be completed — model '{}' stopped responding after research phase. \
                                 Please review the tool results above and try again or switch models.",
                                self.model_id
                            )
                        };
                        info!("[plan] Empty response streak in plan mode → auto-finalizing plan");
                        let outcome = self.finalize_plan_mode(plan_text).await;
                        return Ok(outcome);
                    }

                    // Bail out with error message.
                    let msg = format!(
                        "Model '{}' returned {} consecutive empty responses. This usually means the model doesn't support the tool calling format being used. Please check the model configuration.",
                        self.model_id, state.empty_response_streak
                    );
                    let _ = self.persist_assistant_message(&msg, session_id).await;
                    if let Some(manager) = self.tools.get_manager() {
                        let agent_id = self.agent_id.clone().unwrap_or_else(|| "unknown".to_string());
                        manager.send_event(crate::agent_manager::AgentEvent::TextSegment {
                            agent_id,
                            text: msg.clone(),
                            parent_id: self.parent_agent_id.clone(),
                        }, self.session_id.clone()).await;
                    }
                    self.active_skill = None;
                    return Ok(AgentOutcome::None);
                }
                // Push a nudge and retry
                let nudge = self.tool_result_msg(
                    "Your response was empty. Please respond with either a tool call or text.".to_string(),
                );
                self.push_tracked_message(&mut state.messages, nudge, MessageImportance::Normal);
                continue;
            }
            state.empty_response_streak = 0;

            // --- Native tool calling path ---
            if !native_tool_calls.is_empty() {
                // Emit visible text content (strip any embedded JSON actions).
                // In plan mode, suppress text content blocks — plan text reaches
                // the UI via PlanUpdate events instead.  Emitting it here
                // would create a duplicate text message that hides the PlanBlock.
                let visible_text = text_before_first_json(&raw);
                if !visible_text.is_empty() && !self.plan_mode {
                    if let Some(manager) = self.tools.get_manager() {
                        let agent_id = self
                            .agent_id
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string());
                        manager
                            .send_event(crate::agent_manager::AgentEvent::TextSegment {
                                agent_id: agent_id.clone(),
                                text: visible_text.clone(),
                                parent_id: self.parent_agent_id.clone(),
                            }, self.session_id.clone())
                            .await;
                        manager
                            .send_event(crate::agent_manager::AgentEvent::ContentBlockStart {
                                agent_id,
                                block_id: uuid::Uuid::new_v4().to_string(),
                                block_type: "text".to_string(),
                                tool: None,
                                args: Some(visible_text),
                                parent_id: self.parent_agent_id.clone(),
                                run_id: self.run_id.clone(),
                                parent_run_id: self.parent_run_id.clone(),
                            }, self.session_id.clone())
                            .await;
                    }
                }

                // Record the assistant message with tool_calls in chat history.
                // Preserve id/call_type so OpenAI-compatible APIs (Gemini, etc.)
                // can match tool results back to their calls.
                let tool_call_msgs: Vec<crate::ollama::ToolCallMessage> = native_tool_calls
                    .iter()
                    .map(|tc| crate::ollama::ToolCallMessage {
                        id: tc.id.clone(),
                        call_type: "function".to_string(),
                        function: crate::ollama::ToolCallFunction {
                            name: tc.name.clone(),
                            arguments: tc.arguments.clone(),
                        },
                        thought_signature: tc.thought_signature.clone(),
                    })
                    .collect();
                let mut assistant_msg = ChatMessage::assistant_with_tool_calls(tool_call_msgs);
                if !raw.is_empty() {
                    assistant_msg.content = raw.clone();
                }
                self.push_tracked_message(&mut state.messages, assistant_msg, MessageImportance::High);

                // Convert ParsedToolCalls to ModelActions
                let actions: Vec<ModelAction> = Self::tool_calls_to_actions(&native_tool_calls);

                // All native tool calls were filtered (e.g. all were "Done") →
                // treat as text-only response and end the loop.
                if actions.is_empty() {
                    // Plan mode fallback: model emitted Done without calling
                    // ExitPlanMode.  Treat the text as the plan and auto-finalize.
                    if self.plan_mode {
                        info!("[plan] Done-only tool call in plan mode → implicit ExitPlanMode");
                        let plan_text = raw.trim().to_string();
                        if !plan_text.is_empty() {
                            self.last_assistant_text = Some(plan_text.clone());
                        }
                        let outcome = self.finalize_plan_mode(plan_text).await;
                        return Ok(outcome);
                    }
                    let _ = self.persist_assistant_message(&raw, session_id).await;
                    self.chat_history.push(ChatMessage::new("assistant", raw.clone()));
                    self.truncate_chat_history();
                    self.last_assistant_text = Some(raw);
                    self.active_skill = None;
                    return Ok(AgentOutcome::None);
                }

                state.invalid_json_streak = 0;
                // Track latest response so plan mode can read it (ExitPlanMode
                // uses last_assistant_response as plan text).  Only update
                // when non-empty to avoid losing the plan text if ExitPlanMode
                // arrives in a turn with no text content.
                if !raw.is_empty() {
                    state.last_assistant_response = raw.clone();
                }

                // Execute actions (reusing shared dispatch logic)
                let tc_ids: Vec<String> = native_tool_calls.iter().map(|tc| tc.id.clone()).collect();
                if let Some(outcome) = self.execute_action_loop(actions, &mut state, session_id, &tc_ids).await {
                    return Ok(outcome);
                }
                continue;
            }

            // --- Native mode: text-only response (no tool calls) ---
            if native_tools.is_some() && native_tool_calls.is_empty() && !raw.trim().is_empty() {
                // Check if the text contains JSON actions (some models output JSON text
                // instead of using native function calls). If so, fall through to the
                // legacy JSON action parser below.
                let has_json_actions = {
                    let trimmed = raw.trim();
                    trimmed.starts_with('{') && (
                        trimmed.contains("\"type\"") || trimmed.contains("\"name\"")
                    )
                };
                if !has_json_actions {
                    // Plan mode fallback: model output plan text without calling
                    // ExitPlanMode.  Treat the text as the plan and auto-finalize.
                    if self.plan_mode {
                        info!("[plan] Text-only response in plan mode → implicit ExitPlanMode");
                        let plan_text = raw.trim().to_string();
                        if !plan_text.is_empty() {
                            self.last_assistant_text = Some(plan_text.clone());
                        }
                        let outcome = self.finalize_plan_mode(plan_text).await;
                        return Ok(outcome);
                    }

                    // Pure conversational text — emit as a complete block.
                    if let Some(manager) = self.tools.get_manager() {
                        let agent_id = self
                            .agent_id
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string());
                        manager
                            .send_event(crate::agent_manager::AgentEvent::TextSegment {
                                agent_id: agent_id.clone(),
                                text: raw.clone(),
                                parent_id: self.parent_agent_id.clone(),
                            }, self.session_id.clone())
                            .await;
                        manager
                            .send_event(crate::agent_manager::AgentEvent::ContentBlockStart {
                                agent_id,
                                block_id: uuid::Uuid::new_v4().to_string(),
                                block_type: "text".to_string(),
                                tool: None,
                                args: Some(raw.clone()),
                                parent_id: self.parent_agent_id.clone(),
                                run_id: self.run_id.clone(),
                                parent_run_id: self.parent_run_id.clone(),
                            }, self.session_id.clone())
                            .await;
                    }
                    let _ = self.persist_assistant_message(&raw, session_id).await;
                    self.chat_history.push(ChatMessage::new("assistant", raw.clone()));
                    self.truncate_chat_history();
                    self.last_assistant_text = Some(raw);
                    self.active_skill = None;
                    return Ok(AgentOutcome::None);
                }
                // Fall through to legacy JSON action parser
            }

            // --- Legacy path: parse JSON actions from free-form text ---

            // Emit text segment event for text before the first JSON object.
            // Suppress in plan mode — plan text is delivered via PlanUpdate events.
            {
                let text_before = text_before_first_json(&raw);
                if !text_before.is_empty() && !self.plan_mode {
                    if let Some(manager) = self.tools.get_manager() {
                        let agent_id = self
                            .agent_id
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string());
                        // Emit TextSegment for streaming display in UI.
                        manager
                            .send_event(crate::agent_manager::AgentEvent::TextSegment {
                                agent_id: agent_id.clone(),
                                text: text_before.clone(),
                                parent_id: self.parent_agent_id.clone(),
                            }, self.session_id.clone())
                            .await;
                        // Also emit structured ContentBlockStart(text) for Web UI.
                        manager
                            .send_event(crate::agent_manager::AgentEvent::ContentBlockStart {
                                agent_id,
                                block_id: uuid::Uuid::new_v4().to_string(),
                                block_type: "text".to_string(),
                                tool: None,
                                args: Some(text_before),
                                parent_id: self.parent_agent_id.clone(),
                                run_id: self.run_id.clone(),
                                parent_run_id: self.parent_run_id.clone(),
                            }, self.session_id.clone())
                            .await;
                    }
                }
            }

            // Repetition check
            if let Some(ctrl) = self
                .handle_repetition_check(
                    &raw,
                    &mut state.last_assistant_response,
                    &mut state.identical_response_streak,
                    &mut state.loop_nudge_count,
                    &mut state.messages,
                    session_id,
                )
                .await
            {
                match ctrl {
                    LoopControl::Return(outcome) => return Ok(outcome),
                    LoopControl::Continue => continue,
                }
            }

            // Parse actions from model response.
            let actions = if let Some((first, offset)) = stream_first_action {
                let remainder = &raw[offset..];
                let mut all = vec![first];
                if let Ok(rest) = parse_all_actions(remainder) {
                    all.extend(rest);
                }
                Ok(all)
            } else {
                parse_all_actions(&raw)
            };
            let actions = match actions {
                Ok(v) if !v.is_empty() => v,
                Ok(_) | Err(_) => {
                    if !raw.contains('{') && looks_like_final_answer(&raw) {
                        // Plain text that looks like a substantive answer (long enough,
                        // not "thinking out loud") — treat as an implicit done.
                        let _ = self
                            .persist_assistant_message(&raw, session_id)
                            .await;
                        return Ok(AgentOutcome::None);
                    }
                    // Either malformed JSON or short/incomplete plain text — nudge.
                    state.invalid_json_streak += 1;
                    if state.invalid_json_streak >= 4 {
                        let message = if !raw.contains('{') {
                            raw.clone()
                        } else {
                            self.prompt_store.render_or_fallback(
                                crate::prompts::keys::BAILOUT_MALFORMED_OUTPUT,
                                &[],
                            )
                        };
                        let _ = self
                            .persist_assistant_message(&message, session_id)
                            .await;
                        self.active_skill = None;
                        return Ok(AgentOutcome::None);
                    }
                    let nudge = self.tool_result_msg(
                        self.prompt_store.render_or_fallback(crate::prompts::NUDGE_INVALID_JSON, &[("raw", &raw)]),
                    );
                    self.push_tracked_message(&mut state.messages, nudge, MessageImportance::Normal);
                    self.push_context_record(
                        ContextType::Error,
                        Some("invalid_json".to_string()),
                        self.agent_id.clone(),
                        None,
                        "invalid_json: no valid action found".to_string(),
                        serde_json::json!({ "raw": raw }),
                    );
                    continue;
                }
            };
            state.invalid_json_streak = 0;

            // Execute actions with parallel delegation support.
            if let Some(outcome) = self.execute_action_loop(actions, &mut state, session_id, &[]).await {
                return Ok(outcome);
            }
        }

        self.active_skill = None;

        // Only emit the bailout message when we actually hit the iteration
        // limit.  If the loop exited because the user sent a new message,
        // silently yield so the next chat round can start cleanly.
        if !interrupted_by_user {
            let fallback = self.prompt_store.render_or_fallback(
                crate::prompts::keys::BAILOUT_LOOP_LIMIT,
                &[],
            );
            self.push_context_record(
                ContextType::Status,
                Some("loop_limit_reached".to_string()),
                self.agent_id.clone(),
                Some("user".to_string()),
                fallback.clone(),
                serde_json::json!({ "max_iters": self.cfg.max_iters }),
            );
            let _ = self
                .persist_assistant_message(&fallback, session_id)
                .await;
        }
        Ok(AgentOutcome::None)
    }

    /// Execute a list of model actions with batching (delegation, parallel, sequential).
    /// Shared by both native tool-call and legacy JSON paths.
    /// Returns `Some(outcome)` if the loop should exit early.
    async fn execute_action_loop(
        &mut self,
        actions: Vec<ModelAction>,
        state: &mut LoopState,
        session_id: Option<&str>,
        tc_ids: &[String],
    ) -> Option<AgentOutcome> {
        let mut actions = actions;
        let mut action_idx: usize = 0;

        while !actions.is_empty() {
            // Check cancellation before dispatching each tool/batch.
            if self.is_cancelled().await {
                return Some(AgentOutcome::None);
            }

            let front_is_delegation = {
                let tool = &actions[0].tool;
                self.tools
                    .canonical_tool_name(tool)
                    .unwrap_or(tool.as_str())
                    == "Task"
            };

            let parallel_batch_size = if !front_is_delegation {
                let candidate_count = actions
                    .iter()
                    .take_while(|a| {
                        let c = self
                            .tools
                            .canonical_tool_name(&a.tool)
                            .unwrap_or(a.tool.as_str());
                        can_parallel_tool(c)
                    })
                    .count()
                    .min(8);
                if candidate_count >= 2 {
                    let pairs: Vec<(&str, &JsonValue)> = actions[..candidate_count]
                        .iter()
                        .map(|a| {
                            let c = self.tools.canonical_tool_name(&a.tool).unwrap_or(a.tool.as_str());
                            (c, &a.args)
                        })
                        .collect();
                    if has_write_path_conflicts(&pairs, &self.tools.builtins.cwd()) {
                        1
                    } else {
                        candidate_count
                    }
                } else {
                    candidate_count
                }
            } else {
                0
            };

            if front_is_delegation {
                let batch_size = actions
                    .iter()
                    .take_while(|a| {
                        self.tools
                            .canonical_tool_name(&a.tool)
                            .unwrap_or(a.tool.as_str())
                            == "Task"
                    })
                    .count();
                let batch: Vec<ModelAction> = actions.drain(..batch_size).collect();
                let batch_start = action_idx;
                action_idx += batch_size;

                if let Some(outcome) = self
                    .handle_delegation_batch(
                        batch,
                        &state.allowed_tools,
                        &mut state.messages,
                        session_id,
                        tc_ids,
                        batch_start,
                    )
                    .await
                {
                    self.drain_tool_progress(&mut state.progress_rx).await;
                    return Some(outcome);
                }
            } else if parallel_batch_size >= 2 {
                let batch: Vec<ModelAction> =
                    actions.drain(..parallel_batch_size).collect();
                let batch_start = action_idx;
                action_idx += parallel_batch_size;

                if let Some(outcome) = self
                    .handle_parallel_batch(batch, state, session_id, tc_ids, batch_start)
                    .await
                {
                    self.drain_tool_progress(&mut state.progress_rx).await;
                    return Some(outcome);
                }
            } else {
                let action = actions.remove(0);
                let tc_id = tc_ids.get(action_idx).cloned();
                action_idx += 1;
                let actions_remaining = !actions.is_empty();
                if let Some(outcome) = self
                    .dispatch_sequential_action(
                        action,
                        state,
                        actions_remaining,
                        session_id,
                        tc_id,
                    )
                    .await
                {
                    self.drain_tool_progress(&mut state.progress_rx).await;
                    return Some(outcome);
                }
            }
        }

        self.drain_tool_progress(&mut state.progress_rx).await;
        // Check if the agent cd'd into/out of a git project
        self.check_working_folder_change();
        None
    }
}

/// Build a single-line preview of an agent task for the INFO log. Strips
/// `[HIDDEN]` marker tags (used internally to hide prompts from UI), then
/// keeps the first non-empty line. Multi-line/very long tasks (e.g. the
/// full MEMORY_EXTRACT payload that embeds the dashboard schema) are
/// truncated with an ellipsis and a byte/line count so the reader can tell
/// there's more hidden at DEBUG.
fn summarize_task_for_log(task: &str) -> String {
    const MAX: usize = 160;
    let stripped = task.trim_start_matches("[HIDDEN] ").trim_start();
    // Collapse the leading marker again if the model double-marked it.
    let stripped = stripped.trim_start_matches("[HIDDEN] ").trim();
    let first_line = stripped.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let total_bytes = task.len();
    let total_lines = task.lines().count();
    let one_line = if first_line.chars().count() > MAX {
        let cut: String = first_line.chars().take(MAX).collect();
        format!("{cut}…")
    } else {
        first_line.to_string()
    };
    if total_lines <= 1 && total_bytes <= MAX {
        one_line
    } else {
        format!("{one_line} ({total_bytes}B/{total_lines}L — full at DEBUG)")
    }
}
