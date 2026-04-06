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
        tc_ids: &[String],
        batch_start: usize,
    ) -> Option<AgentOutcome> {
        use crate::agent_manager::AgentManager;

        // Helper to get the tc_id for a given action index.
        let tc_id_for = |idx: usize| -> Option<String> {
            tc_ids.get(batch_start + idx).cloned()
        };
        let tool_msg = |this: &Self, content: String, idx: usize| -> ChatMessage {
            this.tool_result_msg_for(content, &tc_id_for(idx), "Task")
        };

        // Parse TaskArgs from each action, tracking original index.
        let mut delegation_args: Vec<(usize, tools::TaskArgs)> = Vec::new();
        for (i, action) in batch.into_iter().enumerate() {
            let normalized = tools::normalize_tool_args(&action.tool, action.args.clone());
            match serde_json::from_value::<tools::TaskArgs>(normalized) {
                Ok(da) => delegation_args.push((i, da)),
                Err(e) => {
                    messages.push(tool_msg(self,
                        self.prompt_store.render_or_fallback(
                            crate::prompts::keys::INVALID_TASK_ARGS,
                            &[("error", &e.to_string())],
                        ),
                        i,
                    ));
                }
            }
        }

        if delegation_args.is_empty() {
            return None;
        }

        // Permission check (once for the whole batch).
        if let Some(allowed) = allowed_tools {
            if !self.is_tool_allowed(allowed, "Task") {
                for (i, da) in &delegation_args {
                    messages.push(tool_msg(self,
                        self.prompt_store.render_or_fallback(
                            crate::prompts::keys::DELEGATION_BLOCKED,
                            &[("target", &da.target_agent_id)],
                        ),
                        *i,
                    ));
                }
                return None;
            }
        }

        // Validate all delegations and collect spawn params.
        struct DelegationSpawn {
            action_idx: usize,
            manager: Arc<AgentManager>,
            caller_id: String,
            target_agent_id: String,
            task: String,
            parent_run_id: Option<String>,
            depth: usize,
            max_depth: usize,
            session_id: Option<String>,
        }
        let mut spawns: Vec<DelegationSpawn> = Vec::new();

        for (i, da) in delegation_args {
            match self.tools.builtins.validate_delegation(&da) {
                Ok((manager, caller_id)) => {
                    spawns.push(DelegationSpawn {
                        action_idx: i,
                        manager,
                        caller_id,
                        target_agent_id: da.target_agent_id,
                        task: da.task,
                        parent_run_id: self.run_id.clone(),
                        depth: self.tools.builtins.delegation_depth(),
                        max_depth: self.tools.builtins.max_delegation_depth(),
                        session_id: self.tools.builtins.session_id.clone(),
                    });
                }
                Err(e) => {
                    messages.push(tool_msg(self,
                        self.prompt_store.render_or_fallback(
                            crate::prompts::keys::DELEGATION_VALIDATION_FAILED,
                            &[("target", &da.target_agent_id), ("error", &e.to_string())],
                        ),
                        i,
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
        for spawn in spawns.into_iter() {
            let ws = ws_root.clone();
            let bridge = ask_bridge.clone();
            let action_idx = spawn.action_idx;
            join_set.spawn_blocking(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(1)
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let target = spawn.target_agent_id.clone();
                        return (action_idx, target, Err(anyhow::anyhow!("failed to create delegation runtime: {}", e)));
                    }
                };
                let target = spawn.target_agent_id.clone();
                let result = rt.block_on(async move {
                    tools::run_delegation(
                        spawn.manager, ws, spawn.caller_id, spawn.target_agent_id,
                        spawn.task, spawn.parent_run_id, spawn.depth, spawn.max_depth,
                        bridge, spawn.session_id,
                    )
                    .await
                });
                (action_idx, target, result)
            });
        }

        // Await all and collect results, checking cancellation between joins.
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
            // Check cancellation after each delegation completes so we
            // stop collecting results promptly (spec: agentic-loop.md).
            if self.is_cancelled().await {
                join_set.abort_all();
                return Some(AgentOutcome::None);
            }
        }
        results.sort_by_key(|(idx, _, _)| *idx);

        // Merge results into messages.
        for (idx, target, result) in results {
            match result {
                Ok(tool_result) => {
                    let rendered = render_tool_result(&tool_result);
                    let _ = self
                        .persist_observation("Task", &rendered, session_id)
                        .await;
                    messages.push(tool_msg(self,
                        self.observation_text(
                            "tool",
                            &format!("Task({})", target),
                            &rendered,
                        ),
                        idx,
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
                    messages.push(tool_msg(self,
                        self.prompt_store.render_or_fallback(
                            crate::prompts::keys::DELEGATION_FAILED,
                            &[("target", &target), ("error", &e.to_string())],
                        ),
                        idx,
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
            let tc_id = tc_ids.get(action_idx_start + idx).cloned();
            match self
                .pre_execute_tool(
                    action.tool, action.args, &state.allowed_tools, &mut state.messages,
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

        // Phase 3: post-execute each result sequentially, checking cancellation.
        let mut results = results;
        results.sort_by_key(|(idx, _, _)| *idx);
        for (_idx, exec, result) in results {
            if self.is_cancelled().await {
                return Some(AgentOutcome::None);
            }
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
        let tool_name = action.tool.as_str();

        // Handle plan-mode tools as special dispatches.
        match tool_name {
            "ExitPlanMode" => {
                if self.plan_mode {
                    tracing::info!("[plan] ExitPlanMode → submitting plan for review");
                    // Gather plan text from both the tool argument and the
                    // response text.  Some models (e.g. Gemini) stream the full
                    // plan as response content but only put a truncated version
                    // in the ExitPlanMode plan_text argument.  Pick whichever
                    // source is more complete so the PlanBlock shows the full plan.
                    let arg_text = action.args.get("plan_text")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let response_text = crate::engine::actions::text_before_first_json(
                        &state.last_assistant_response,
                    );
                    let plan_text = if !response_text.is_empty() && response_text.len() > arg_text.len() {
                        response_text
                    } else if !arg_text.is_empty() {
                        arg_text
                    } else {
                        response_text
                    };
                    if !plan_text.is_empty() {
                        self.last_assistant_text = Some(plan_text.clone());
                    }
                    // Merge items from ExitPlanMode args into stored plan items.
                    if let Some(items_arr) = action.args.get("items").and_then(|v| v.as_array()) {
                        let mut plan_items = Vec::new();
                        for item in items_arr {
                            let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                            let status = item.get("status").and_then(|v| v.as_str()).unwrap_or("pending").to_string();
                            plan_items.push(PlanItem { id, title, status });
                        }
                        if !plan_items.is_empty() {
                            // Store items so finalize_plan_mode picks them up.
                            let plan = self.plan.get_or_insert_with(|| Plan {
                                summary: String::new(),
                                status: PlanStatus::Planned,
                                plan_text: String::new(),
                                items: Vec::new(),
                            });
                            plan.items = plan_items;
                            tracing::info!("[plan] ExitPlanMode: merged {} items from args", plan.items.len());
                        }
                    }
                    state.messages.push(self.tool_result_msg_for(
                        self.prompt_store.render_or_fallback(
                            crate::prompts::keys::PLAN_SUBMITTED, &[],
                        ),
                        &tc_id, "ExitPlanMode",
                    ));
                    let outcome = self.finalize_plan_mode(plan_text).await;
                    return Some(outcome);
                } else {
                    state.messages.push(self.tool_result_msg_for(
                        self.prompt_store.render_or_fallback(
                            crate::prompts::keys::EXIT_PLAN_MODE_OUTSIDE_PLAN, &[],
                        ),
                        &tc_id, "ExitPlanMode",
                    ));
                }
                return None;
            }
            "EnterPlanMode" => {
                let reason = action.args.get("reason")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                tracing::info!("[plan] EnterPlanMode: {:?}", reason);
                // If already in plan mode, treat as a no-op so the model
                // continues researching instead of exiting the loop.
                if self.plan_mode {
                    tracing::warn!("[plan] EnterPlanMode called while already in plan mode — ignoring");
                    state.messages.push(self.tool_result_msg_for(
                        "You are already in plan mode. Continue researching and produce your plan text directly — do not call EnterPlanMode again.".to_string(),
                        &tc_id, "EnterPlanMode",
                    ));
                    return None;
                }
                // Push tool result so native tool calling APIs see a matching response.
                state.messages.push(self.tool_result_msg_for(
                    "Entering plan mode.".to_string(),
                    &tc_id, "EnterPlanMode",
                ));
                return Some(AgentOutcome::PlanModeRequested { reason });
            }
            "UpdatePlan" => {
                let plan_text = action.args.get("plan_text")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let items = action.args.get("items")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                // During planning (before approval), store items silently
                // without emitting PlanUpdate to the UI. The PlanBlock is
                // only created when ExitPlanMode fires. This avoids the
                // premature PlanBlock + duplicate content bug.
                if self.plan_mode && self.plan.as_ref().map(|p| p.status == PlanStatus::Planned).unwrap_or(true) {
                    let mut plan_items = Vec::new();
                    for item in &items {
                        let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                        let status = item.get("status").and_then(|v| v.as_str()).unwrap_or("pending").to_string();
                        plan_items.push(PlanItem { id, title, status });
                    }
                    let plan = self.plan.get_or_insert_with(|| Plan {
                        summary: String::new(),
                        status: PlanStatus::Planned,
                        plan_text: String::new(),
                        items: Vec::new(),
                    });
                    if !plan_items.is_empty() {
                        plan.items = plan_items;
                    }
                    if let Some(text) = plan_text.filter(|s| !s.trim().is_empty()) {
                        plan.plan_text = text;
                    }
                    info!("[plan] UpdatePlan (planning phase): stored {} items silently, no SSE", plan.items.len());
                    state.messages.push(self.tool_result_msg_for(
                        self.prompt_store.render_or_fallback(
                            crate::prompts::keys::PLAN_UPDATED,
                            &[("count", &items.len().to_string())],
                        ),
                        &tc_id, "UpdatePlan",
                    ));
                } else {
                    self.handle_update_plan_action(plan_text, items, &mut state.messages, session_id, &tc_id).await;
                }
                return None;
            }
            _ => {}
        }

        // Regular tool dispatch.
        match self
            .handle_tool_action(
                action.tool, action.args, &state.allowed_tools, &mut state.messages,
                &mut state.tool_cache, &mut state.read_paths,
                &mut state.last_tool_sig, &mut state.redundant_tool_streak,
                &mut state.empty_search_streak, &mut state.progress_rx, session_id,
                tc_id,
            )
            .await
        {
            LoopControl::Return(outcome) => Some(outcome),
            LoopControl::Continue => None,
        }
    }

    /// Handle an `update_plan` action from the model: convert task items into
    /// a Plan and emit a PlanUpdate event so the UI can display progress.
    async fn handle_update_plan_action(
        &mut self,
        explicit_plan_text: Option<String>,
        items: Vec<serde_json::Value>,
        messages: &mut Vec<ChatMessage>,
        session_id: Option<&str>,
        tc_id: &Option<String>,
    ) {
        // Build structured items + markdown checklist.
        let mut plan_items = Vec::new();
        let mut lines = Vec::new();
        for item in &items {
            let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("?").to_string();
            let status = item.get("status").and_then(|v| v.as_str()).unwrap_or("pending").to_string();
            let checkbox = match status.as_str() {
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
            plan_items.push(PlanItem { id, title, status });
        }
        // Use existing plan_text if it has substantial content (from planning
        // phase). Only replace if the model provides something longer, or if
        // no existing plan_text exists. This prevents execution UpdatePlan
        // calls from overwriting the full plan with a short summary.
        let existing_text = self.plan.as_ref()
            .map(|p| p.plan_text.clone())
            .filter(|s| !s.trim().is_empty());
        let plan_text = match (explicit_plan_text.filter(|s| !s.trim().is_empty()), existing_text) {
            (Some(explicit), Some(existing)) => {
                if explicit.len() > existing.len() { explicit } else { existing }
            }
            (Some(explicit), None) => explicit,
            (None, Some(existing)) => existing,
            (None, None) => lines.join("\n"),
        };
        let summary = Self::extract_plan_summary(&plan_text);

        // Preserve pre-approval status — don't promote Planned to Executing.
        // During plan mode (self.plan is None), stay in Planned so the UI
        // doesn't flash Executing → Planned when ExitPlanMode fires.
        let status = if self.plan_mode && self.plan.is_none() {
            PlanStatus::Planned
        } else {
            self.plan.as_ref()
                .map(|p| p.status.clone())
                .filter(|s| *s == PlanStatus::Planned || *s == PlanStatus::Approved)
                .unwrap_or(PlanStatus::Executing)
        };
        info!("[plan] UpdatePlan: plan_mode={} existing_status={:?} → new_status={:?}",
            self.plan_mode,
            self.plan.as_ref().map(|p| &p.status),
            status);
        let plan = Plan {
            summary,
            status,
            plan_text,
            items: plan_items,
        };
        self.persist_and_emit_plan(plan).await;

        let ack = self.prompt_store.render_or_fallback(
            crate::prompts::keys::PLAN_UPDATED,
            &[("count", &items.len().to_string())],
        );
        info!("[plan] UpdatePlan: {}", ack);

        // Push acknowledgement to model messages so it sees the feedback.
        messages.push(self.tool_result_msg_for(ack.clone(), tc_id, "UpdatePlan"));
        let _ = self
            .persist_observation("UpdatePlan", &ack, session_id)
            .await;
    }
}
