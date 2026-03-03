pub mod actions;
mod context;
pub mod patch;
pub mod permission;
mod plan;
mod prompt;
pub mod render;
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
    Plan, PlanStatus, TaskPacket, ThinkingEvent,
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
use tracing::{debug, info};

impl AgentEngine {
    /// Convert native ParsedToolCalls into ModelActions for dispatch.
    /// Non-tool actions (Done, EnterPlanMode, UpdatePlan, Patch, FinalizeTask)
    /// are recognized by name and converted to the appropriate variant.
    fn tool_calls_to_actions(tool_calls: &[ParsedToolCall]) -> Vec<ModelAction> {
        tool_calls
            .iter()
            .filter_map(|tc| {
                match tc.name.as_str() {
                    "Done" => {
                        let message = tc.arguments.get("message").and_then(|v| v.as_str()).map(|s| s.to_string());
                        Some(ModelAction::Done { message })
                    }
                    "EnterPlanMode" => {
                        let reason = tc.arguments.get("reason").and_then(|v| v.as_str()).map(|s| s.to_string());
                        Some(ModelAction::EnterPlanMode { reason })
                    }
                    "UpdatePlan" => {
                        let items = tc.arguments.get("items")
                            .and_then(|v| v.as_array())
                            .cloned()
                            .unwrap_or_default();
                        Some(ModelAction::UpdatePlan { items })
                    }
                    "Patch" => {
                        let diff = tc.arguments.get("diff").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        Some(ModelAction::Patch { diff })
                    }
                    "FinalizeTask" => {
                        let packet = tc.arguments.get("packet").cloned().unwrap_or(serde_json::json!({}));
                        match serde_json::from_value(packet) {
                            Ok(p) => Some(ModelAction::FinalizeTask { packet: p }),
                            Err(_) => None,
                        }
                    }
                    _ => {
                        // Regular tool call
                        Some(ModelAction::Tool {
                            tool: tc.name.clone(),
                            args: tc.arguments.clone(),
                        })
                    }
                }
            })
            .collect()
    }

    pub async fn run_agent_loop(&mut self, session_id: Option<&str>) -> Result<AgentOutcome> {
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
                })
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

        info!(
            "Starting agent loop for role {:?} with task: {}",
            self.role, task
        );
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
            progress_rx,
        };
        self.native_tool_mode = use_native_tools;

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
                    })
                    .await;
            }

            let summary_count = self.maybe_compact_model_messages(&mut state.messages, "loop_iter");
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
                    "Model response (native): text='{}' tool_calls={}",
                    text_part.replace('\n', "\\n"),
                    native_tool_calls.len()
                );
            } else {
                debug!(
                    "Model response: text='{}' json={}",
                    text_part.replace('\n', "\\n"),
                    json_rendered
                );
            }

            // --- Native tool calling path ---
            if !native_tool_calls.is_empty() {
                // Emit text content if present
                if !raw.is_empty() {
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
                            })
                            .await;
                        manager
                            .send_event(crate::agent_manager::AgentEvent::ContentBlockStart {
                                agent_id,
                                block_id: uuid::Uuid::new_v4().to_string(),
                                block_type: "text".to_string(),
                                tool: None,
                                args: Some(raw.clone()),
                                parent_id: self.parent_agent_id.clone(),
                            })
                            .await;
                    }
                }

                // Record the assistant message with tool_calls in chat history.
                // Use empty id/call_type so they are omitted from Ollama requests
                // (Ollama's native API doesn't use these OpenAI-specific fields).
                let tool_call_msgs: Vec<crate::ollama::ToolCallMessage> = native_tool_calls
                    .iter()
                    .map(|tc| crate::ollama::ToolCallMessage {
                        id: String::new(),
                        call_type: String::new(),
                        function: crate::ollama::ToolCallFunction {
                            name: tc.name.clone(),
                            arguments: tc.arguments.clone(),
                        },
                    })
                    .collect();
                let mut assistant_msg = ChatMessage::assistant_with_tool_calls(tool_call_msgs);
                if !raw.is_empty() {
                    assistant_msg.content = raw.clone();
                }
                self.push_tracked_message(&mut state.messages, assistant_msg, MessageImportance::High);

                // Convert ParsedToolCalls to ModelActions
                let actions: Vec<ModelAction> = Self::tool_calls_to_actions(&native_tool_calls);
                state.invalid_json_streak = 0;

                // Execute actions (reusing existing dispatch logic)
                let mut early_return: Option<AgentOutcome> = None;
                let mut actions = actions;
                // Track tool_call_ids for result messages
                let tc_ids: Vec<String> = native_tool_calls.iter().map(|tc| tc.id.clone()).collect();
                let mut action_idx = 0;

                while !actions.is_empty() && early_return.is_none() {
                    let front_is_delegation = match &actions[0] {
                        ModelAction::Tool { tool, .. } => {
                            self.tools
                                .canonical_tool_name(tool)
                                .unwrap_or(tool.as_str())
                                == "Task"
                        }
                        _ => false,
                    };

                    let parallel_batch_size = if !front_is_delegation {
                        let candidate_count = actions
                            .iter()
                            .take_while(|a| match a {
                                ModelAction::Tool { tool, .. } => {
                                    let c = self
                                        .tools
                                        .canonical_tool_name(tool)
                                        .unwrap_or(tool.as_str());
                                    can_parallel_tool(c)
                                }
                                _ => false,
                            })
                            .count()
                            .min(8);
                        if candidate_count >= 2 {
                            let pairs: Vec<(&str, &JsonValue)> = actions[..candidate_count]
                                .iter()
                                .filter_map(|a| match a {
                                    ModelAction::Tool { tool, args } => {
                                        let c = self.tools.canonical_tool_name(tool).unwrap_or(tool.as_str());
                                        Some((c, args))
                                    }
                                    _ => None,
                                })
                                .collect();
                            if has_write_path_conflicts(&pairs, &self.cfg.ws_root) {
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
                            .take_while(|a| match a {
                                ModelAction::Tool { tool, .. } => {
                                    self.tools
                                        .canonical_tool_name(tool)
                                        .unwrap_or(tool.as_str())
                                        == "Task"
                                }
                                _ => false,
                            })
                            .count();
                        let batch: Vec<ModelAction> = actions.drain(..batch_size).collect();
                        action_idx += batch_size;

                        if let Some(outcome) = self
                            .handle_delegation_batch(
                                batch,
                                &state.allowed_tools,
                                &mut state.messages,
                                session_id,
                            )
                            .await
                        {
                            early_return = Some(outcome);
                        }
                    } else if parallel_batch_size >= 2 {
                        let batch: Vec<ModelAction> =
                            actions.drain(..parallel_batch_size).collect();
                        action_idx += parallel_batch_size;

                        if let Some(outcome) = self
                            .handle_parallel_batch(batch, &mut state, session_id)
                            .await
                        {
                            early_return = Some(outcome);
                        }
                    } else {
                        let action = actions.remove(0);
                        let _tc_id = tc_ids.get(action_idx).cloned();
                        action_idx += 1;
                        let actions_remaining = !actions.is_empty();
                        if let Some(outcome) = self
                            .dispatch_sequential_action(
                                action,
                                &mut state,
                                actions_remaining,
                                session_id,
                            )
                            .await
                        {
                            early_return = Some(outcome);
                        }
                    }
                }

                self.drain_tool_progress(&mut state.progress_rx).await;

                if let Some(outcome) = early_return {
                    return Ok(outcome);
                }
                continue;
            }

            // --- Native mode: text-only response (no tool calls) ---
            if native_tools.is_some() && native_tool_calls.is_empty() && !raw.trim().is_empty() {
                // In native mode, a text-only response IS the conversational reply.
                // Content tokens were already streamed to the UI in real-time via
                // ContentToken events in stream_with_tool_calling(), so we don't
                // need to emit TextSegment/ContentBlockStart events here.
                let _ = self.manager_db_add_assistant_message(&raw, session_id).await;
                self.chat_history.push(ChatMessage::new("assistant", raw.clone()));
                self.truncate_chat_history();
                self.last_assistant_text = Some(raw);
                self.active_skill = None;
                return Ok(AgentOutcome::None);
            }

            // --- Legacy path: parse JSON actions from free-form text ---

            // Emit text segment event for text before the first JSON object.
            {
                let text_before = text_before_first_json(&raw);
                if !text_before.is_empty() {
                    if let Some(manager) = self.tools.get_manager() {
                        let agent_id = self
                            .agent_id
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string());
                        // Keep TextSegment for backward compat (TUI, older clients).
                        manager
                            .send_event(crate::agent_manager::AgentEvent::TextSegment {
                                agent_id: agent_id.clone(),
                                text: text_before.clone(),
                                parent_id: self.parent_agent_id.clone(),
                            })
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
                            })
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
                            .manager_db_add_assistant_message(&raw, session_id)
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
                            .manager_db_add_assistant_message(&message, session_id)
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
            let mut early_return: Option<AgentOutcome> = None;
            let mut actions = actions;

            while !actions.is_empty() && early_return.is_none() {
                // Check if the front action is a Task (delegation) tool call.
                let front_is_delegation = match &actions[0] {
                    ModelAction::Tool { tool, .. } => {
                        self.tools
                            .canonical_tool_name(tool)
                            .unwrap_or(tool.as_str())
                            == "Task"
                    }
                    _ => false,
                };

                // Check for a batch of consecutive parallelizable tool calls (cap at 8).
                let parallel_batch_size = if !front_is_delegation {
                    let candidate_count = actions
                        .iter()
                        .take_while(|a| match a {
                            ModelAction::Tool { tool, .. } => {
                                let c = self
                                    .tools
                                    .canonical_tool_name(tool)
                                    .unwrap_or(tool.as_str());
                                can_parallel_tool(c)
                            }
                            _ => false,
                        })
                        .count()
                        .min(8);
                    if candidate_count >= 2 {
                        let pairs: Vec<(&str, &JsonValue)> = actions[..candidate_count]
                            .iter()
                            .filter_map(|a| match a {
                                ModelAction::Tool { tool, args } => {
                                    let c = self.tools.canonical_tool_name(tool).unwrap_or(tool.as_str());
                                    Some((c, args))
                                }
                                _ => None,
                            })
                            .collect();
                        if has_write_path_conflicts(&pairs, &self.cfg.ws_root) {
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
                    // Collect a run of consecutive Task (delegation) actions.
                    let batch_size = actions
                        .iter()
                        .take_while(|a| match a {
                            ModelAction::Tool { tool, .. } => {
                                self.tools
                                    .canonical_tool_name(tool)
                                    .unwrap_or(tool.as_str())
                                    == "Task"
                            }
                            _ => false,
                        })
                        .count();
                    let batch: Vec<ModelAction> = actions.drain(..batch_size).collect();

                    if let Some(outcome) = self
                        .handle_delegation_batch(
                            batch,
                            &state.allowed_tools,
                            &mut state.messages,
                            session_id,
                        )
                        .await
                    {
                        early_return = Some(outcome);
                    }
                } else if parallel_batch_size >= 2 {
                    let batch: Vec<ModelAction> =
                        actions.drain(..parallel_batch_size).collect();

                    if let Some(outcome) = self
                        .handle_parallel_batch(batch, &mut state, session_id)
                        .await
                    {
                        early_return = Some(outcome);
                    }
                } else {
                    let action = actions.remove(0);
                    let actions_remaining = !actions.is_empty();
                    if let Some(outcome) = self
                        .dispatch_sequential_action(
                            action,
                            &mut state,
                            actions_remaining,
                            session_id,
                        )
                        .await
                    {
                        early_return = Some(outcome);
                    }
                }
            }

            // Drain any tool progress lines accumulated during this iteration.
            self.drain_tool_progress(&mut state.progress_rx).await;

            if let Some(outcome) = early_return {
                return Ok(outcome);
            }
        }

        self.active_skill = None;
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
            .manager_db_add_assistant_message(&fallback, session_id)
            .await;
        Ok(AgentOutcome::None)
    }
}
