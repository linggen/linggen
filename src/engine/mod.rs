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

pub use actions::{model_message_log_parts, parse_all_actions, text_before_first_json, ModelAction};

// Internal imports used by run_agent_loop
use streaming::{can_parallel_tool, has_write_path_conflicts};
use types::{LoopControl, LoopState, MessageImportance};

use crate::ollama::ChatMessage;
use anyhow::Result;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use tracing::{debug, info};

impl AgentEngine {
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

        let (messages, allowed_tools, read_paths) =
            self.prepare_loop_messages(&task);

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

            // Ask model for the next action, streaming thinking tokens.
            let stream_result = self.stream_with_fallback(&state.messages).await?;
            let raw = stream_result.full_text;
            let stream_first_action = stream_result.first_action;

            // Log model output: text + json (truncated).
            let (text_part, json_part) = model_message_log_parts(&raw, 100, 100);
            let json_rendered = json_part
                .as_ref()
                .and_then(|v| serde_json::to_string(v).ok())
                .unwrap_or_else(|| "null".to_string());
            debug!(
                "Model response: text='{}' json={}",
                text_part.replace('\n', "\\n"),
                json_rendered
            );

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
                    if !raw.contains('{') {
                        vec![ModelAction::Done {
                            message: Some(raw.clone()),
                        }]
                    } else {
                        state.invalid_json_streak += 1;
                        if state.invalid_json_streak >= 4 {
                            let message = "I couldn't continue automatically because the model kept returning malformed structured output.".to_string();
                            let _ = self
                                .manager_db_add_assistant_message(&message, session_id)
                                .await;
                            self.active_skill = None;
                            return Ok(AgentOutcome::None);
                        }
                        let nudge = ChatMessage::new(
                            "user",
                            self.nudge(crate::prompts::NUDGE_INVALID_JSON, &[("raw", &raw)]),
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
        let fallback = "I couldn't complete this automatically within the current tool loop limit. Please refine the request and try again."
            .to_string();
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
