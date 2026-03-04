use super::types::*;
use crate::engine::actions::ModelAction;
use crate::engine::render::render_tool_result;
use crate::engine::streaming::check_context_staleness;
use crate::engine::tools::{self, ToolCall};
use crate::ollama::ChatMessage;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, info, warn};

impl AgentEngine {
    /// Handle a batch of consecutive `Task` (delegation) actions.
    /// Returns `Some(outcome)` if the loop should exit early.
    pub(crate) async fn handle_delegation_batch(
        &mut self,
        batch: Vec<ModelAction>,
        allowed_tools: &Option<HashSet<String>>,
        messages: &mut Vec<ChatMessage>,
        session_id: Option<&str>,
    ) -> Option<AgentOutcome> {
        use crate::agent_manager::AgentManager;

        // Parse TaskArgs from each action.
        let mut delegation_args: Vec<tools::TaskArgs> = Vec::new();
        for action in batch {
            if let ModelAction::Tool { tool, args } = action {
                let normalized = tools::normalize_tool_args(&tool, args.clone());
                match serde_json::from_value::<tools::TaskArgs>(normalized) {
                    Ok(da) => delegation_args.push(da),
                    Err(e) => {
                        messages.push(self.tool_result_msg(
                            self.prompt_store.render_or_fallback(
                                crate::prompts::keys::INVALID_TASK_ARGS,
                                &[("error", &e.to_string())],
                            ),
                        ));
                    }
                }
            }
        }

        if delegation_args.is_empty() {
            return None;
        }

        // Permission check (once for the whole batch).
        if let Some(allowed) = allowed_tools {
            if !self.is_tool_allowed(allowed, "Task") {
                for da in &delegation_args {
                    messages.push(self.tool_result_msg(
                        self.prompt_store.render_or_fallback(
                            crate::prompts::keys::DELEGATION_BLOCKED,
                            &[("target", &da.target_agent_id)],
                        ),
                    ));
                }
                return None;
            }
        }

        // Validate all delegations and collect spawn params.
        struct DelegationSpawn {
            manager: Arc<AgentManager>,
            caller_id: String,
            target_agent_id: String,
            task: String,
            parent_run_id: Option<String>,
            depth: usize,
            max_depth: usize,
        }
        let mut spawns: Vec<DelegationSpawn> = Vec::new();

        for da in delegation_args {
            match self.tools.builtins.validate_delegation(&da) {
                Ok((manager, caller_id)) => {
                    spawns.push(DelegationSpawn {
                        manager,
                        caller_id,
                        target_agent_id: da.target_agent_id,
                        task: da.task,
                        parent_run_id: self.run_id.clone(),
                        depth: self.tools.builtins.delegation_depth(),
                        max_depth: self.tools.builtins.max_delegation_depth(),
                    });
                }
                Err(e) => {
                    messages.push(self.tool_result_msg(
                        self.prompt_store.render_or_fallback(
                            crate::prompts::keys::DELEGATION_VALIDATION_FAILED,
                            &[("target", &da.target_agent_id), ("error", &e.to_string())],
                        ),
                    ));
                    let rendered = format!(
                        "tool_error: tool=Task target={} error={}",
                        da.target_agent_id, e
                    );
                    self.upsert_observation("error", "Task", rendered);
                }
            }
        }

        if spawns.is_empty() {
            return None;
        }

        let ws_root = self.cfg.ws_root.clone();
        let ask_bridge = self.tools.ask_user_bridge().cloned();

        // Spawn each delegation on a blocking thread with its own tokio runtime.
        let mut join_set = tokio::task::JoinSet::new();
        for (spawn_idx, spawn) in spawns.into_iter().enumerate() {
            let ws = ws_root.clone();
            let bridge = ask_bridge.clone();
            join_set.spawn_blocking(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(1)
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let target = spawn.target_agent_id.clone();
                        return (spawn_idx, target, Err(anyhow::anyhow!("failed to create delegation runtime: {}", e)));
                    }
                };
                let target = spawn.target_agent_id.clone();
                let result = rt.block_on(async move {
                    tools::run_delegation(
                        spawn.manager, ws, spawn.caller_id, spawn.target_agent_id,
                        spawn.task, spawn.parent_run_id, spawn.depth, spawn.max_depth,
                        bridge,
                    )
                    .await
                });
                (spawn_idx, target, result)
            });
        }

        // Await all and collect results.
        let mut results: Vec<(usize, String, anyhow::Result<tools::ToolResult>)> = Vec::new();
        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok((idx, target, result)) => results.push((idx, target, result)),
                Err(join_err) => {
                    warn!("Delegation task panicked: {}", join_err);
                    results.push((usize::MAX, "unknown".to_string(),
                        Err(anyhow::anyhow!("delegation task panicked: {}", join_err))));
                }
            }
        }
        results.sort_by_key(|(idx, _, _)| *idx);

        // Merge results into messages.
        for (_idx, target, result) in results {
            match result {
                Ok(tool_result) => {
                    let rendered = render_tool_result(&tool_result);
                    let _ = self
                        .persist_observation("Task", &rendered, session_id)
                        .await;
                    messages.push(self.tool_result_msg(
                        self.observation_text(
                            "tool",
                            &format!("Task({})", target),
                            &rendered,
                        ),
                    ));
                    self.upsert_observation("tool", "Task", rendered);
                }
                Err(e) => {
                    let rendered = format!(
                        "tool_error: tool=Task target={} error={}", target, e
                    );
                    let _ = self
                        .persist_observation("Task", &rendered, session_id)
                        .await;
                    messages.push(self.tool_result_msg(
                        self.prompt_store.render_or_fallback(
                            crate::prompts::keys::DELEGATION_FAILED,
                            &[("target", &target), ("error", &e.to_string())],
                        ),
                    ));
                    self.upsert_observation("error", "Task", rendered);
                }
            }
        }
        None
    }

    /// Handle a batch of parallelizable tool actions (read-only + non-conflicting writes).
    /// Returns `Some(outcome)` if the loop should exit early.
    pub(crate) async fn handle_parallel_batch(
        &mut self,
        batch: Vec<ModelAction>,
        state: &mut LoopState,
        session_id: Option<&str>,
        tc_ids: &[String],
        action_idx_start: usize,
    ) -> Option<AgentOutcome> {
        // Phase 1: pre-execute each tool sequentially (needs &mut self).
        let mut ready: Vec<(usize, ToolCall, ReadyExec)> = Vec::new();
        for (idx, action) in batch.into_iter().enumerate() {
            if let ModelAction::Tool { tool, args } = action {
                let tc_id = tc_ids.get(action_idx_start + idx).cloned();
                match self
                    .pre_execute_tool(
                        tool, args, &state.allowed_tools, &mut state.messages,
                        &mut state.tool_cache, &mut state.read_paths,
                        &mut state.last_tool_sig, &mut state.redundant_tool_streak,
                        session_id, tc_id,
                    )
                    .await
                {
                    PreExecOutcome::Blocked(LoopControl::Return(outcome)) => {
                        return Some(outcome);
                    }
                    PreExecOutcome::Blocked(LoopControl::Continue) => {}
                    PreExecOutcome::Ready(call, exec) => {
                        ready.push((idx, call, exec));
                    }
                }
            }
        }
        if ready.is_empty() {
            return None;
        }

        // Phase 2: execute tools in parallel via std::thread::scope.
        let tools_ref = &self.tools;
        let cached_hash = self.cached_system_prompt.as_ref().map(|c| c.input_hash);
        let staleness_ws_root = &self.cfg.ws_root;
        let staleness_memory_dir = self.tools.memory_dir().map(|p| p.as_path());
        let (results, prompt_stale): (Vec<(usize, ReadyExec, anyhow::Result<tools::ToolResult>)>, bool) =
            tokio::task::block_in_place(|| {
                std::thread::scope(|scope| {
                    let staleness_handle = scope.spawn(|| {
                        check_context_staleness(cached_hash, staleness_ws_root, staleness_memory_dir)
                    });
                    let handles: Vec<_> = ready
                        .into_iter()
                        .map(|(idx, call, exec)| {
                            let handle = scope.spawn(move || {
                                // No runtime is set up here — sync tools (Read, Glob, Grep)
                                // don't need one, and async tools (WebFetch, WebSearch, etc.)
                                // create their own via block_on_async().
                                tools_ref.execute(call)
                            });
                            (idx, exec, handle)
                        })
                        .collect();
                    let tool_results: Vec<_> = handles
                        .into_iter()
                        .map(|(idx, exec, handle)| {
                            let result = handle.join().unwrap_or_else(|_| {
                                Err(anyhow::anyhow!("tool execution panicked"))
                            });
                            (idx, exec, result)
                        })
                        .collect();
                    let stale = staleness_handle.join().unwrap_or(false);
                    (tool_results, stale)
                })
            });

        // If context files changed, rebuild the system prompt cache.
        if prompt_stale {
            debug!("Context files changed; rebuilding system prompt");
            let (new_stable, new_hash) = self.build_stable_system_content();
            self.cached_system_prompt = Some(CachedSystemPrompt {
                input_hash: new_hash,
                content: new_stable.clone(),
            });
            if !state.messages.is_empty() {
                let mut sys = new_stable;
                if let Some(dyn_start) = state.messages[0].content.find("\n\nYou are in PLAN MODE") {
                    sys.push_str(&state.messages[0].content[dyn_start..]);
                }
                state.messages[0] = ChatMessage::new("system", sys);
            }
        }

        // Phase 3: post-execute each result sequentially.
        let mut results = results;
        results.sort_by_key(|(idx, _, _)| *idx);
        for (_idx, exec, result) in results {
            match self
                .post_execute_tool(
                    exec, result, &mut state.messages,
                    &mut state.tool_cache, &mut state.empty_search_streak, session_id,
                )
                .await
            {
                LoopControl::Return(outcome) => return Some(outcome),
                LoopControl::Continue => {}
            }
        }
        None
    }

    /// Dispatch a single non-delegation, non-parallel action.
    /// Returns `Some(outcome)` if the loop should exit early.
    pub(crate) async fn dispatch_sequential_action(
        &mut self,
        action: ModelAction,
        state: &mut LoopState,
        _actions_remaining: bool,
        session_id: Option<&str>,
        tc_id: Option<String>,
    ) -> Option<AgentOutcome> {
        match action {
            ModelAction::Tool { ref tool, .. } if tool == "ExitPlanMode" => {
                if self.plan_mode {
                    info!("ExitPlanMode → asking user for approval");
                    // Extract the plan prose before any JSON tool calls.
                    let plan_text = crate::engine::actions::text_before_first_json(
                        &state.last_assistant_response,
                    );

                    // Capture the assistant text so the outer chat handler can
                    // persist and emit it as a Message.
                    if !plan_text.is_empty() {
                        self.last_assistant_text = Some(plan_text.clone());
                    }

                    // Acknowledge ExitPlanMode so the model knows it was processed.
                    state.messages.push(self.tool_result_msg(
                        self.prompt_store.render_or_fallback(
                            crate::prompts::keys::PLAN_SUBMITTED,
                            &[],
                        ),
                    ));

                    // Persist + ask for approval via shared helper.
                    let (outcome, feedback) = self.finalize_plan_mode(plan_text).await;

                    if let Some(feedback) = feedback {
                        self.inject_plan_feedback(&mut state.messages, &feedback);
                    } else {
                        return Some(outcome);
                    }
                } else {
                    state.messages.push(self.tool_result_msg(
                        self.prompt_store.render_or_fallback(
                            crate::prompts::keys::EXIT_PLAN_MODE_OUTSIDE_PLAN,
                            &[],
                        ),
                    ));
                }
            }
            ModelAction::Tool { tool, args } => {
                match self
                    .handle_tool_action(
                        tool, args, &state.allowed_tools, &mut state.messages,
                        &mut state.tool_cache, &mut state.read_paths,
                        &mut state.last_tool_sig, &mut state.redundant_tool_streak,
                        &mut state.empty_search_streak, &mut state.progress_rx, session_id,
                        tc_id,
                    )
                    .await
                {
                    LoopControl::Return(outcome) => return Some(outcome),
                    LoopControl::Continue => {}
                }
            }
            ModelAction::Patch { diff } => {
                match self.handle_patch_action(diff, &mut state.messages).await {
                    LoopControl::Return(outcome) => return Some(outcome),
                    LoopControl::Continue => {}
                }
            }
            ModelAction::FinalizeTask { packet } => {
                match self.handle_finalize_action(packet, &mut state.messages, session_id).await {
                    LoopControl::Return(outcome) => return Some(outcome),
                    LoopControl::Continue => {}
                }
            }
            ModelAction::Done { message } => {
                let msg = message.unwrap_or_else(|| {
                    self.prompt_store.render_or_fallback(
                        crate::prompts::keys::DONE_DEFAULT,
                        &[],
                    )
                });
                info!("Done: {}", crate::engine::render::truncate_for_log(&msg, 200));
                self.push_context_record(
                    ContextType::Status, Some("done".to_string()),
                    self.agent_id.clone(), Some("user".to_string()),
                    msg.clone(), serde_json::json!({ "kind": "done" }),
                );
                let _ = self.persist_assistant_message(&msg, session_id).await;
                self.chat_history.push(ChatMessage::new("assistant", msg.clone()));
                self.truncate_chat_history();
                self.last_assistant_text = Some(msg.clone());
                self.active_skill = None;
                if self.plan_mode {
                    // Fallback: model emitted done in plan mode without calling ExitPlanMode.
                    // Treat the last response as the plan text (strip JSON).
                    info!("Done in plan mode → implicit ExitPlanMode");
                    let plan_text = crate::engine::actions::text_before_first_json(
                        &state.last_assistant_response,
                    );
                    let (outcome, feedback) = self.finalize_plan_mode(plan_text).await;
                    if let Some(feedback) = feedback {
                        self.inject_plan_feedback(&mut state.messages, &feedback);
                    } else {
                        return Some(outcome);
                    }
                }
                return Some(AgentOutcome::None);
            }
            ModelAction::EnterPlanMode { reason } => {
                info!("EnterPlanMode: {:?}", reason);
                return Some(AgentOutcome::PlanModeRequested { reason });
            }
            ModelAction::UpdatePlan { items } => {
                self.handle_update_plan_action(items, &mut state.messages, session_id).await;
            }
        }
        None
    }

    /// Inject user feedback on a plan into the message stream so the model
    /// can revise. Used by both the ExitPlanMode and Done-in-plan-mode paths.
    fn inject_plan_feedback(&self, messages: &mut Vec<ChatMessage>, feedback: &str) {
        info!("User feedback on plan: {}", feedback);
        messages.push(self.tool_result_msg(format!(
            "User feedback on plan:\n{}\n\nPlease revise the plan based on this feedback.",
            feedback
        )));
    }

    /// Handle an `update_plan` action from the model: convert task items into
    /// a Plan and emit a PlanUpdate event so the UI can display progress.
    async fn handle_update_plan_action(
        &mut self,
        items: Vec<serde_json::Value>,
        messages: &mut Vec<ChatMessage>,
        session_id: Option<&str>,
    ) {
        // Build markdown plan text from items.
        let mut lines = Vec::new();
        for item in &items {
            let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("?");
            let status = item.get("status").and_then(|v| v.as_str()).unwrap_or("pending");
            let checkbox = match status {
                "completed" | "done" => "- [x]",
                "in_progress" | "working" => "- [~]",
                _ => "- [ ]",
            };
            let suffix = if status == "in_progress" || status == "working" {
                " *(in progress)*"
            } else {
                ""
            };
            lines.push(format!("{} {}{}", checkbox, title, suffix));
        }
        let plan_text = lines.join("\n");
        let summary = Self::extract_plan_summary(&plan_text);

        let plan = Plan {
            summary,
            status: PlanStatus::Executing,
            plan_text,
        };
        self.persist_and_emit_plan(plan).await;

        let ack = self.prompt_store.render_or_fallback(
            crate::prompts::keys::PLAN_UPDATED,
            &[("count", &items.len().to_string())],
        );
        info!("UpdatePlan: {}", ack);

        // Push acknowledgement to model messages so it sees the feedback.
        messages.push(self.tool_result_msg(ack.clone()));
        let _ = self
            .persist_observation("UpdatePlan", &ack, session_id)
            .await;
    }
}
