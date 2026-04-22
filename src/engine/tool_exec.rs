use super::types::*;
use crate::engine::permission;
use crate::engine::render::{
    normalize_tool_path_arg, render_tool_result, render_tool_result_public,
    sanitize_tool_args_for_display, tool_call_signature,
};
use crate::engine::tools::{self, ToolCall};
use crate::ollama::ChatMessage;
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::{info, warn};

impl AgentEngine {
    /// Create a message with the correct role for tool results.
    /// In native tool calling mode, uses `role: "tool"` (required by Ollama).
    /// In JSON-action mode, uses `role: "user"` (tool results are observations).
    ///
    /// NOTE: This always uses `role: "user"` for synthetic/system messages
    /// (nudges, plan acks, delegation results, etc.). For messages that
    /// correspond to an actual native tool call, use `tool_result_msg_for()`
    /// which includes `tool_call_id` and `name` fields required by strict
    /// OpenAI-compatible APIs (e.g. Gemini).
    pub(crate) fn tool_result_msg(&self, content: String) -> ChatMessage {
        ChatMessage::new("user", content)
    }

    /// Create a tool result message tied to a specific native tool call.
    /// Uses `tool_call_id` and `name` when available (required by Gemini's
    /// OpenAI-compatible API), falls back to `tool_result_msg` otherwise.
    pub(crate) fn tool_result_msg_for(
        &self,
        content: String,
        tool_call_id: &Option<String>,
        tool_name: &str,
    ) -> ChatMessage {
        if let Some(ref tc_id) = tool_call_id {
            ChatMessage::tool_result_named(tc_id.clone(), tool_name, content)
        } else {
            self.tool_result_msg(content)
        }
    }

    /// Ask the user for permission via the AskUser bridge.
    /// Ask user for permission, returning the raw selected label and optional custom text.
    /// Returns `None` on timeout, `Some(PermissionAction)` otherwise.
    pub async fn ask_permission_raw(
        &self,
        tool: &str,
        question: tools::AskUserQuestion,
    ) -> Option<permission::PermissionAction> {
        let bridge = match self.tools.ask_user_bridge() {
            Some(b) => Arc::clone(b),
            None => return None,
        };

        let question_id = uuid::Uuid::new_v4().to_string();
        let agent_id = self.agent_id.clone().unwrap_or_default();

        info!("Permission: awaiting user approval for '{}'", tool);
        let questions = vec![question];
        let _ = bridge.events_tx.send(crate::server::ServerEvent::AskUser {
            agent_id: agent_id.clone(),
            question_id: question_id.clone(),
            questions: questions.clone(),
            session_id: bridge.session_id.clone(),
        });

        let (tx, rx) = tokio::sync::oneshot::channel();
        bridge.pending.lock().await.insert(
            question_id.clone(),
            tools::PendingAskUser {
                agent_id,
                questions,
                sender: tx,
                session_id: bridge.session_id.clone(),
            },
        );

        let response = tokio::time::timeout(std::time::Duration::from_secs(8 * 3600), rx).await;
        bridge.pending.lock().await.remove(&question_id);

        match response {
            Ok(Ok(answers)) => {
                let answer = answers.first();
                let custom = answer.and_then(|a| a.custom_text.as_deref()).unwrap_or("");
                if !custom.is_empty() {
                    info!("Permission: '{}' → DenyWithMessage", tool);
                    return Some(permission::PermissionAction::DenyWithMessage(custom.to_string()));
                }
                let selected = answer
                    .and_then(|a| a.selected.first())
                    .map(|s| s.as_str())
                    .unwrap_or("Cancel");
                info!("Permission: '{}' → selected '{}'", tool, selected);
                // Map common labels to actions
                match selected {
                    "Allow once" | "Approve" => Some(permission::PermissionAction::AllowOnce),
                    "Deny" | "Cancel" => Some(permission::PermissionAction::Deny),
                    "Allow for this session" | "Run in current mode" => {
                        Some(permission::PermissionAction::AllowSession)
                    }
                    other => {
                        if other.starts_with("Switch to ") || other.starts_with("Allow read on ") {
                            Some(permission::PermissionAction::AllowSession)
                        } else {
                            Some(permission::PermissionAction::Deny)
                        }
                    }
                }
            }
            Ok(Err(_)) => {
                warn!("Permission: '{}' channel closed → denied", tool);
                Some(permission::PermissionAction::Deny)
            }
            Err(_) => {
                warn!("Permission: '{}' timed out → stopping agent", tool);
                None
            }
        }
    }

    // Old ask_permission / ask_bash_permission / ask_file_permission methods removed.
    // New permission flow uses ask_permission_raw (above) + PromptKind-specific prompt builders.

    /// Pre-execution phase: validate permissions, record context, check caches,
    /// and emit "start" events. Returns `Ready` with the prepared ToolCall
    /// and metadata, or `Blocked` if the call should not proceed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn pre_execute_tool(
        &mut self,
        tool: String,
        args: JsonValue,
        allowed_tools: &Option<HashSet<String>>,
        messages: &mut Vec<ChatMessage>,
        tool_cache: &mut HashMap<String, CachedToolObs>,
        read_paths: &mut HashSet<String>,
        last_tool_sig: &mut String,
        redundant_tool_streak: &mut usize,
        session_id: Option<&str>,
        tool_call_id: Option<String>,
    ) -> PreExecOutcome {
        let canonical_tool = self
            .tools
            .canonical_tool_name(&tool)
            .unwrap_or(tool.as_str())
            .to_string();

        // --- permission gate ---
        if let Some(allowed) = allowed_tools {
            if !self.is_tool_allowed(allowed, &tool) {
                let mut allowed_list = allowed.iter().cloned().collect::<Vec<_>>();
                allowed_list.sort();
                let rendered = format!(
                    "tool_not_allowed: tool={} canonical={} allowed={}",
                    tool,
                    canonical_tool,
                    allowed_list.join(",")
                );
                self.upsert_observation("error", &canonical_tool, rendered.clone());
                let _ = self
                    .persist_observation(&canonical_tool, &rendered, session_id)
                    .await;
                messages.push(self.tool_result_msg_for(
                    self.prompt_store.render_or_fallback(
                        crate::prompts::keys::TOOL_NOT_ALLOWED,
                        &[("tool", &tool), ("allowed_list", &allowed_list.join(", "))],
                    ),
                    &tool_call_id, &canonical_tool,
                ));
                return PreExecOutcome::Blocked(LoopControl::Continue);
            }
        }

        let safe_args = sanitize_tool_args_for_display(&canonical_tool, &args);
        self.upsert_context_record_by_type_name(
            ContextType::ToolCall,
            &canonical_tool,
            self.agent_id.clone(),
            Some(self.outbound_target()),
            serde_json::to_string(&safe_args).unwrap_or_else(|_| "{}".to_string()),
            serde_json::json!({ "args": safe_args.clone() }),
        );
        let log_run = self
            .run_id
            .clone()
            .unwrap_or_else(|| "root".to_string());
        info!("[{}] Tool: {} {}", log_run, canonical_tool, safe_args);
        if canonical_tool == "Read" {
            if let Some(path) = normalize_tool_path_arg(&self.tools.builtins.cwd(), &args) {
                read_paths.insert(path);
            }
        }

        // Compute tool call signature early for denied-check and later redundancy tracking.
        let sig = tool_call_signature(&canonical_tool, &args);

        // --- write-safety gate ---
        if matches!(canonical_tool.as_str(), "Write" | "Edit") {
            if let Some(path) = normalize_tool_path_arg(&self.tools.builtins.cwd(), &args) {
                let existing = self.tools.builtins.cwd().join(&path).exists();
                if existing && !read_paths.contains(&path) {
                    let action = if canonical_tool == "Edit" {
                        "Edit"
                    } else {
                        "Write"
                    };
                    match self.cfg.write_safety_mode {
                        crate::config::WriteSafetyMode::Strict => {
                            let rendered = format!(
                                "tool_error: tool={} error=precondition_failed: must call Read on '{}' before {} for existing files",
                                action, path, action
                            );
                            self.upsert_observation("error", action, rendered.clone());
                            let _ = self
                                .persist_observation(action, &rendered, session_id)
                                .await;
                            messages.push(self.tool_result_msg_for(
                                self.prompt_store.render_or_fallback(
                                    crate::prompts::keys::WRITE_SAFETY_BLOCKED,
                                    &[("rendered", &rendered)],
                                ),
                                &tool_call_id, &canonical_tool,
                            ));
                            return PreExecOutcome::Blocked(LoopControl::Continue);
                        }
                        crate::config::WriteSafetyMode::Warn => {
                            let rendered = format!(
                                "tool_warning: tool={} warning=writing_existing_file_without_prior_read path='{}'",
                                action, path
                            );
                            self.upsert_observation("warning", action, rendered.clone());
                            let _ = self
                                .persist_observation(action, &rendered, session_id)
                                .await;
                        }
                        crate::config::WriteSafetyMode::Off => {}
                    }
                }
            }
        }

        // --- config-level tool restriction gate (defense-in-depth) ---
        // Blocks tools not allowed by mission tiers or consumer room settings.
        // The prompt already excludes these tools, but this catches hallucinations.
        if !self.cfg.is_tool_allowed(&canonical_tool) {
            let available = self.cfg.effective_tool_restrictions()
                .map(|s| s.into_iter().collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            let msg = format!("Tool '{}' is not available. Allowed: {}", canonical_tool, available);
            messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
            return PreExecOutcome::Blocked(LoopControl::Continue);
        }

        // --- consumer skill restriction gate (defense-in-depth) ---
        // When consumer_allowed_skills is set, block Skill invocations not in the list.
        if canonical_tool == "Skill" {
            if let Some(ref allowed_skills) = self.cfg.consumer_allowed_skills {
                let skill_name = args.get("skill")
                    .or_else(|| args.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !allowed_skills.contains(skill_name) {
                    let msg = format!("Skill '{}' is not available to consumers.", skill_name);
                    messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                    return PreExecOutcome::Blocked(LoopControl::Continue);
                }
            }
        }

        // --- new permission gate (permission-spec.md) ---

        // Skill data tools (empty cmd, e.g. PageUpdate) pass their args
        // straight through as a content block — they don't read files, run
        // commands, or reach outside the process. Running them through the
        // path/tier permission gate produces nonsense prompts like
        // "PageUpdate /Users/lianghuang — switch to admin?" because the gate
        // synthesizes cwd as the file_path_arg and classifies unknown tools
        // as Admin tier. Skip the gate entirely for pure data tools.
        let skip_permission_gate = self.tools.is_skill_data_tool(&canonical_tool);

        // Extract bash command and file path for permission checking.
        let bash_command = if canonical_tool == "Bash" {
            args.get("cmd")
                .or_else(|| args.get("command"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        } else {
            None
        };
        let file_path_arg = if matches!(canonical_tool.as_str(), "Write" | "Edit" | "Read") {
            normalize_tool_path_arg(&self.tools.builtins.cwd(), &args)
        } else {
            // For tools without an explicit file path (Glob, Grep, Task, etc.),
            // use the agent's cwd so permission checks resolve against the
            // session's path_mode grants for the workspace.
            Some(self.tools.builtins.cwd().to_string_lossy().to_string())
        };

        // Auto-block retries of denied tool calls.
        if self.session_permissions.denied_sigs.contains(&sig) {
            let summary =
                permission::permission_target_summary(&canonical_tool, &args, &self.tools.builtins.cwd());
            let msg = self.prompt_store.render_or_fallback(
                crate::prompts::keys::PERMISSION_DENIED,
                &[("tool", &canonical_tool), ("summary", &summary)],
            );
            messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
            return PreExecOutcome::Blocked(LoopControl::Continue);
        }

        // --- mission bash prefix restriction (legacy, kept for backward compat) ---
        if canonical_tool == "Bash" {
            if let Some(ref prefixes) = self.cfg.bash_allow_prefixes {
                let cmd = bash_command.as_deref().unwrap_or("");
                let cmd_trimmed = cmd.trim();
                let allowed = prefixes.iter().any(|prefix| cmd_trimmed.starts_with(prefix));
                if !allowed {
                    let msg = format!(
                        "Bash command not allowed by this mission's permission tier. \
                         Command: '{}'. Allowed prefixes: {}",
                        cmd_trimmed,
                        prefixes.join(", ")
                    );
                    messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                    return PreExecOutcome::Blocked(LoopControl::Continue);
                }
            }
        }

        // Skill-declared tier wins over the built-in table. For HTTP and
        // shell skill tools that declare `tier: read|edit|admin` in their
        // manifest, parse it into a PermissionMode and pass to
        // check_permission as an override. Unknown / missing tier falls
        // back to the built-in classification.
        let skill_tier_override = self
            .tools
            .skill_tools
            .get(canonical_tool.as_str())
            .and_then(|def| def.tier.as_deref())
            .and_then(permission::parse_skill_tier);

        // Run the new permission check (skipped for skill data tools, which
        // don't touch the filesystem or run anything).
        let check_result = if skip_permission_gate {
            permission::PermissionCheckResult::Allowed
        } else {
            permission::check_permission(
                &canonical_tool,
                bash_command.as_deref(),
                file_path_arg.as_deref(),
                &self.tools.builtins.cwd(),
                &self.session_permissions,
                &self.cfg.deny_rules,
                &self.cfg.ask_rules,
                skill_tier_override,
            )
        };

        match check_result {
            permission::PermissionCheckResult::Allowed => { /* proceed */ }
            permission::PermissionCheckResult::Blocked(reason) => {
                info!("Permission blocked: {} — {}", canonical_tool, reason);
                let summary =
                    permission::permission_target_summary(&canonical_tool, &args, &self.tools.builtins.cwd());
                let msg = self.prompt_store.render_or_fallback(
                    crate::prompts::keys::PERMISSION_DENIED,
                    &[("tool", &canonical_tool), ("summary", &summary)],
                );
                messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                return PreExecOutcome::Blocked(LoopControl::Continue);
            }
            permission::PermissionCheckResult::NeedsPrompt(prompt_kind) => {
                let summary =
                    permission::permission_target_summary(&canonical_tool, &args, &self.tools.builtins.cwd());

                match prompt_kind {
                    permission::PromptKind::ExceedsCeiling { target_mode, path, tool_summary } => {
                        let question = permission::build_exceeds_ceiling_question(
                            &tool_summary, &target_mode, &path,
                        );
                        match self.ask_permission_raw(&canonical_tool, question).await {
                            Some(permission::PermissionAction::AllowOnce) => { /* proceed */ }
                            Some(permission::PermissionAction::AllowSession) => {
                                // Switch mode — update path_modes and save.
                                self.session_permissions.set_path_mode(&path, target_mode);
                                if let Some(ref sdir) = self.session_dir {
                                    self.session_permissions.save(sdir);
                                }
                                // Notify UI so the mode badge updates.
                                if let Some(manager) = self.tools.get_manager() {
                                    manager.send_event(
                                        crate::agent_manager::AgentEvent::StateUpdated,
                                        self.session_id.clone(),
                                    );
                                }
                            }
                            Some(permission::PermissionAction::Deny) => {
                                self.session_permissions.denied_sigs.insert(sig.clone());
                                if let Some(ref sdir) = self.session_dir {
                                    self.session_permissions.save(sdir);
                                }
                                let msg = self.prompt_store.render_or_fallback(
                                    crate::prompts::keys::PERMISSION_DENIED,
                                    &[("tool", &canonical_tool), ("summary", &summary)],
                                );
                                messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                                return PreExecOutcome::Blocked(LoopControl::Continue);
                            }
                            Some(permission::PermissionAction::DenyWithMessage(user_msg)) => {
                                self.session_permissions.denied_sigs.insert(sig.clone());
                                if let Some(ref sdir) = self.session_dir {
                                    self.session_permissions.save(sdir);
                                }
                                let msg = format!(
                                    "Permission denied by user for {} '{}'. User says: {}",
                                    canonical_tool, summary, user_msg
                                );
                                messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                                return PreExecOutcome::Blocked(LoopControl::Continue);
                            }
                            None => {
                                let msg = self.prompt_store.render_or_fallback(
                                    crate::prompts::keys::PERMISSION_TIMEOUT, &[],
                                );
                                let _ = self.persist_assistant_message(&msg, session_id).await;
                                return PreExecOutcome::Blocked(LoopControl::Return(AgentOutcome::None));
                            }
                        }
                    }
                    permission::PromptKind::SystemZoneWrite { tool_summary } => {
                        let question = permission::build_system_zone_question(&tool_summary);
                        match self.ask_permission_raw(&canonical_tool, question).await {
                            Some(permission::PermissionAction::AllowOnce) => { /* proceed */ }
                            None => {
                                let msg = self.prompt_store.render_or_fallback(
                                    crate::prompts::keys::PERMISSION_TIMEOUT, &[],
                                );
                                let _ = self.persist_assistant_message(&msg, session_id).await;
                                return PreExecOutcome::Blocked(LoopControl::Return(AgentOutcome::None));
                            }
                            Some(permission::PermissionAction::Deny)
                            | Some(permission::PermissionAction::DenyWithMessage(_)) => {
                                self.session_permissions.denied_sigs.insert(sig.clone());
                                if let Some(ref sdir) = self.session_dir {
                                    self.session_permissions.save(sdir);
                                }
                                let msg = self.prompt_store.render_or_fallback(
                                    crate::prompts::keys::PERMISSION_DENIED,
                                    &[("tool", &canonical_tool), ("summary", &summary)],
                                );
                                messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                                return PreExecOutcome::Blocked(LoopControl::Continue);
                            }
                            _ => { /* unexpected response — allow once as fallback */ }
                        }
                    }
                    permission::PromptKind::AskRuleOverride { rule, tool_summary } => {
                        let question = permission::build_ask_rule_question(&tool_summary, &rule);
                        match self.ask_permission_raw(&canonical_tool, question).await {
                            Some(permission::PermissionAction::AllowOnce) => { /* proceed */ }
                            Some(permission::PermissionAction::AllowSession) => {
                                // Store the matched rule as override key so suppression
                                // covers all commands matching the pattern.
                                self.session_permissions.allows.insert(rule.clone());
                                if let Some(ref sdir) = self.session_dir {
                                    self.session_permissions.save(sdir);
                                }
                            }
                            Some(permission::PermissionAction::Deny) => {
                                self.session_permissions.denied_sigs.insert(sig.clone());
                                if let Some(ref sdir) = self.session_dir {
                                    self.session_permissions.save(sdir);
                                }
                                let msg = self.prompt_store.render_or_fallback(
                                    crate::prompts::keys::PERMISSION_DENIED,
                                    &[("tool", &canonical_tool), ("summary", &summary)],
                                );
                                messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                                return PreExecOutcome::Blocked(LoopControl::Continue);
                            }
                            None => {
                                let msg = self.prompt_store.render_or_fallback(
                                    crate::prompts::keys::PERMISSION_TIMEOUT, &[],
                                );
                                let _ = self.persist_assistant_message(&msg, session_id).await;
                                return PreExecOutcome::Blocked(LoopControl::Return(AgentOutcome::None));
                            }
                            _ => {}
                        }
                    }
                    permission::PromptKind::ReadOutsidePath { dir, tool_summary } => {
                        let question = permission::build_read_outside_path_question(&tool_summary, &dir);
                        match self.ask_permission_raw(&canonical_tool, question).await {
                            Some(permission::PermissionAction::AllowOnce) => { /* proceed */ }
                            Some(permission::PermissionAction::AllowSession) => {
                                // Grant read on the directory.
                                self.session_permissions.set_path_mode(
                                    &dir, permission::PermissionMode::Read,
                                );
                                if let Some(ref sdir) = self.session_dir {
                                    self.session_permissions.save(sdir);
                                }
                            }
                            Some(permission::PermissionAction::Deny) => {
                                self.session_permissions.denied_sigs.insert(sig.clone());
                                if let Some(ref sdir) = self.session_dir {
                                    self.session_permissions.save(sdir);
                                }
                                let msg = self.prompt_store.render_or_fallback(
                                    crate::prompts::keys::PERMISSION_DENIED,
                                    &[("tool", &canonical_tool), ("summary", &summary)],
                                );
                                messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                                return PreExecOutcome::Blocked(LoopControl::Continue);
                            }
                            None => {
                                let msg = self.prompt_store.render_or_fallback(
                                    crate::prompts::keys::PERMISSION_TIMEOUT, &[],
                                );
                                let _ = self.persist_assistant_message(&msg, session_id).await;
                                return PreExecOutcome::Blocked(LoopControl::Return(AgentOutcome::None));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // --- redundancy / cache gates ---
        if sig == *last_tool_sig {
            *redundant_tool_streak += 1;
        } else {
            *redundant_tool_streak = 0;
            *last_tool_sig = sig.clone();
        }

        if *redundant_tool_streak >= 3 {
            let loop_breaker_prompt = self
                .cfg
                .prompt_loop_breaker
                .as_deref()
                .map(|template| Self::render_loop_breaker_prompt(template, &canonical_tool))
                .unwrap_or_else(|| {
                    self.prompt_store.render_or_fallback(crate::prompts::NUDGE_REDUNDANT_TOOL, &[("tool", &canonical_tool)])
                });
            messages.push(self.tool_result_msg_for(loop_breaker_prompt, &tool_call_id, &canonical_tool));
            self.push_context_record(
                ContextType::Error,
                Some("redundant_tool_loop".to_string()),
                self.agent_id.clone(),
                None,
                format!(
                    "Repeated tool call loop detected for '{}'; nudging model to change approach.",
                    canonical_tool
                ),
                serde_json::json!({ "tool": canonical_tool, "streak": *redundant_tool_streak + 1 }),
            );
            *redundant_tool_streak = 0;
            return PreExecOutcome::Blocked(LoopControl::Continue);
        }

        if let Some(cached) = tool_cache.get(&sig) {
            self.upsert_observation("tool", &canonical_tool, cached.model.clone());
            messages.push(self.tool_result_msg_for(
                self.observation_text("tool", &canonical_tool, &cached.model),
                &tool_call_id, &canonical_tool,
            ));
            return PreExecOutcome::Blocked(LoopControl::Continue);
        }

        // --- status lines ---
        let tool_done_status = crate::server::chat_helpers::tool_status_line(
            &canonical_tool,
            Some(&args),
            crate::server::chat_helpers::ToolStatusPhase::Done,
        );
        let tool_failed_status = crate::server::chat_helpers::tool_status_line(
            &canonical_tool,
            Some(&args),
            crate::server::chat_helpers::ToolStatusPhase::Failed,
        );

        // Tell the UI what tool we're about to use.
        let block_id = uuid::Uuid::new_v4().to_string();
        if let Some(manager) = self.tools.get_manager() {
            let from = self
                .agent_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let target = self.outbound_target();
            // Emit structured ContentBlockStart for the Web UI.
            let compact_args = serde_json::to_string(&safe_args)
                .unwrap_or_else(|_| "{}".to_string());
            let _ = manager
                .send_event(crate::agent_manager::AgentEvent::ContentBlockStart {
                    agent_id: from.clone(),
                    block_id: block_id.clone(),
                    block_type: "tool_use".to_string(),
                    tool: Some(canonical_tool.clone()),
                    args: Some(compact_args),
                    parent_id: self.parent_agent_id.clone(),
                    run_id: self.run_id.clone(),
                    parent_run_id: self.parent_run_id.clone(),
                }, self.session_id.clone())
                .await;
            // Persist tool call to session store as an observation (not loaded
            // into chat history on reload — tool results are ephemeral context).
            let tool_msg = serde_json::json!({
                "type": "tool",
                "tool": canonical_tool.clone(),
                "args": safe_args
            })
            .to_string();
            manager
                .add_chat_message(
                    &self.tools.builtins.cwd(),
                    session_id.unwrap_or("default"),
                    &crate::state_fs::sessions::ChatMsg {
                        agent_id: from.clone(),
                        from_id: from,
                        to_id: target,
                        content: tool_msg,
                        timestamp: crate::util::now_ts_secs(),
                        is_observation: true,
                    },
                )
                .await;
        }

        let call = ToolCall {
            tool: canonical_tool.clone(),
            args: args.clone(),
            block_id: Some(block_id.clone()),
        };
        PreExecOutcome::Ready(
            call,
            ReadyExec {
                canonical_tool,
                sig,
                original_args: args,
                tool_done_status,
                tool_failed_status,
                block_id,
                tool_call_id,
            },
        )
    }

    /// Post-execution phase: render and cache the result, emit "done"/"failed"
    /// events, push the observation message, and track empty-search streaks.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn post_execute_tool(
        &mut self,
        exec: ReadyExec,
        result: anyhow::Result<tools::ToolResult>,
        messages: &mut Vec<ChatMessage>,
        tool_cache: &mut HashMap<String, CachedToolObs>,
        empty_search_streak: &mut usize,
        session_id: Option<&str>,
    ) -> LoopControl {
        let ReadyExec {
            canonical_tool,
            sig,
            original_args,
            tool_done_status,
            tool_failed_status,
            block_id,
            tool_call_id,
        } = exec;

        match result {
            Ok(result) => {
                let rendered_model = render_tool_result(&result);
                let rendered_public = render_tool_result_public(&result);

                tool_cache.insert(
                    sig,
                    CachedToolObs {
                        model: rendered_model.clone(),
                    },
                );

                // Invalidate cached Read results for the same file after a successful mutation.
                if matches!(canonical_tool.as_str(), "Write" | "Edit") {
                    if let Some(path) =
                        normalize_tool_path_arg(&self.tools.builtins.cwd(), &original_args)
                    {
                        tool_cache.retain(|key, _| {
                            if !key.starts_with("Read|") {
                                return true;
                            }
                            !key.contains(&format!("\"{}\"", path))
                        });
                    }
                }

                self.upsert_observation("tool", &canonical_tool, rendered_model.clone());

                let _ = self
                    .persist_observation(&canonical_tool, &rendered_public, session_id)
                    .await;
                if let Some(manager) = self.tools.get_manager() {
                    let agent_id = self
                        .agent_id
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string());
                    // Emit structured ContentBlockUpdate for the Web UI.
                    // For Edit/Write, include diff data so the frontend can render inline diffs.
                    // For Bash, include output lines so the widget has them even if
                    // progress events arrived after the block was marked done.
                    let extra = self.build_tool_extra(&canonical_tool, &original_args)
                        .or_else(|| {
                            if canonical_tool == "Bash" {
                                if let tools::ToolResult::CommandOutput { ref stdout, ref stderr, .. } = result {
                                    let mut lines: Vec<&str> = stdout.lines().collect();
                                    if !stderr.is_empty() {
                                        lines.extend(stderr.lines());
                                    }
                                    // Cap at 500 lines to keep the event small.
                                    lines.truncate(500);
                                    Some(serde_json::json!({ "bash_output": lines }))
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        });
                    manager
                        .send_event(crate::agent_manager::AgentEvent::ContentBlockUpdate {
                            agent_id: agent_id.clone(),
                            block_id: block_id.clone(),
                            status: Some("done".to_string()),
                            summary: Some(tool_done_status.clone()),
                            is_error: Some(false),
                            parent_id: self.parent_agent_id.clone(),
                            extra,
                            run_id: self.run_id.clone(),
                            parent_run_id: self.parent_run_id.clone(),
                        }, self.session_id.clone())
                        .await;
                    manager
                        .send_event(crate::agent_manager::AgentEvent::StateUpdated, self.session_id.clone())
                        .await;
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

                // For file mutations, emit a brief user-visible summary line.
                if matches!(canonical_tool.as_str(), "Write" | "Edit")
                    && (rendered_public.starts_with("File written:")
                        || rendered_public.starts_with("Edited file:")
                        || rendered_public.starts_with("File unchanged"))
                {
                    let msg = if rendered_public.starts_with("File unchanged") {
                        if let Some(idx) = rendered_public.rfind(':') {
                            let path = rendered_public[idx + 1..].trim();
                            format!("No changes to `{}`.", path)
                        } else {
                            "No file changes.".to_string()
                        }
                    } else if let Some(idx) = rendered_public.rfind(':') {
                        let rest = rendered_public[idx + 1..].trim();
                        let path = rest.split_whitespace().next().unwrap_or(rest);
                        format!("Updated `{}`.", path)
                    } else {
                        "File updated.".to_string()
                    };
                    let _ = self
                        .persist_assistant_message(&msg, session_id)
                        .await;
                }

                let obs_content = self.observation_text("tool", &canonical_tool, &rendered_model);
                let obs_msg = self.tool_result_msg_for(obs_content, &tool_call_id, &canonical_tool);

                // Assign importance based on tool type and result.
                let importance = if matches!(canonical_tool.as_str(), "Write" | "Edit") {
                    MessageImportance::High
                } else if canonical_tool == "Grep"
                    && (rendered_model.contains("(no matches)")
                        || rendered_model.contains("no file candidates found"))
                {
                    MessageImportance::Low
                } else if canonical_tool == "Glob" && rendered_model.contains("(no files)") {
                    MessageImportance::Low
                } else {
                    MessageImportance::Normal
                };
                self.push_tracked_message(messages, obs_msg, importance);

                let is_empty_search =
                    (canonical_tool == "Grep"
                        && (rendered_model.contains("(no matches)")
                            || rendered_model.contains("no file candidates found")))
                    || (canonical_tool == "Glob" && rendered_model.contains("(no files)"));
                if is_empty_search {
                    *empty_search_streak += 1;
                } else if matches!(canonical_tool.as_str(), "Grep" | "Glob") {
                    // Only reset streak on successful search tools — non-search tools
                    // (Read, Edit, Bash) shouldn't break a search streak.
                    *empty_search_streak = 0;
                }
                if *empty_search_streak >= 4 {
                    messages.push(self.tool_result_msg(
                        self.prompt_store.render_or_fallback(
                            crate::prompts::keys::NUDGE_EMPTY_SEARCH,
                            &[],
                        ),
                    ));
                    self.push_context_record(
                        ContextType::Error,
                        Some("empty_search_loop".to_string()),
                        self.agent_id.clone(),
                        None,
                        "Repeated no-match search loop detected; nudging model to change strategy."
                            .to_string(),
                        serde_json::json!({ "streak": *empty_search_streak }),
                    );
                    *empty_search_streak = 0;
                }
            }
            Err(e) => {
                warn!("Tool failed: {} err={}", canonical_tool, e);
                let rendered = format!("tool_error: tool={} error={}", canonical_tool, e);
                tool_cache.insert(
                    sig,
                    CachedToolObs {
                        model: rendered.clone(),
                    },
                );
                self.upsert_observation("error", &canonical_tool, rendered.clone());
                let _ = self
                    .persist_observation(&canonical_tool, &rendered, session_id)
                    .await;
                if let Some(manager) = self.tools.get_manager() {
                    let agent_id = self
                        .agent_id
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string());
                    // Emit structured ContentBlockUpdate (failed) for the Web UI.
                    let err_summary = format!("{}: {}", tool_failed_status, e);
                    manager
                        .send_event(crate::agent_manager::AgentEvent::ContentBlockUpdate {
                            agent_id: agent_id.clone(),
                            block_id: block_id.clone(),
                            status: Some("failed".to_string()),
                            summary: Some(err_summary),
                            is_error: Some(true),
                            parent_id: self.parent_agent_id.clone(),
                            extra: None,
                            run_id: self.run_id.clone(),
                            parent_run_id: self.parent_run_id.clone(),
                        }, self.session_id.clone())
                        .await;
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
                let err_content = self.prompt_store.render_or_fallback(
                    crate::prompts::keys::TOOL_EXEC_FAILED,
                    &[("tool", &canonical_tool), ("error", &e.to_string())],
                );
                let err_msg = self.tool_result_msg_for(err_content, &tool_call_id, &canonical_tool);
                self.push_tracked_message(messages, err_msg, MessageImportance::High);
            }
        }
        LoopControl::Continue
    }

    /// Build optional extra payload for ContentBlockUpdate (e.g. diff data for Edit/Write).
    fn build_tool_extra(
        &self,
        canonical_tool: &str,
        original_args: &JsonValue,
    ) -> Option<serde_json::Value> {
        match canonical_tool {
            "Edit" => {
                let old_string = original_args
                    .get("old_string")
                    .or_else(|| original_args.get("old"))
                    .or_else(|| original_args.get("old_text"))
                    .or_else(|| original_args.get("search"))
                    .or_else(|| original_args.get("from"))
                    .and_then(|v| v.as_str());
                let new_string = original_args
                    .get("new_string")
                    .or_else(|| original_args.get("new"))
                    .or_else(|| original_args.get("new_text"))
                    .or_else(|| original_args.get("replace"))
                    .or_else(|| original_args.get("to"))
                    .and_then(|v| v.as_str());
                let path = original_args
                    .get("path")
                    .or_else(|| original_args.get("file"))
                    .or_else(|| original_args.get("filepath"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let (old_s, new_s) = match (old_string, new_string) {
                    (Some(o), Some(n)) => (o, n),
                    _ => return None,
                };

                // Compute start_line by reading the (post-edit) file and finding new_string.
                // old_string has already been replaced, so we search for new_string instead.
                let rel = normalize_tool_path_arg(&self.tools.builtins.cwd(), original_args)
                    .unwrap_or_else(|| path.to_string());
                let file_path = self.tools.builtins.cwd().join(&rel);
                let start_line = std::fs::read_to_string(&file_path)
                    .ok()
                    .and_then(|content| {
                        content.find(new_s).map(|pos| {
                            content[..pos].lines().count().max(1)
                        })
                    });

                Some(serde_json::json!({
                    "diff_type": "edit",
                    "path": rel,
                    "old_string": old_s,
                    "new_string": new_s,
                    "start_line": start_line,
                }))
            }
            "Write" => {
                let path = original_args
                    .get("path")
                    .or_else(|| original_args.get("file"))
                    .or_else(|| original_args.get("filepath"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let content = original_args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let rel = normalize_tool_path_arg(&self.tools.builtins.cwd(), original_args)
                    .unwrap_or_else(|| path.to_string());
                let line_count = content.lines().count();

                // Truncate content for diff display (avoid huge payloads)
                let preview = if content.len() > 10_000 {
                    format!("{}…\n(truncated, {} total chars)", &content[..10_000], content.len())
                } else {
                    content.to_string()
                };
                Some(serde_json::json!({
                    "diff_type": "write",
                    "path": rel,
                    "lines_written": line_count,
                    "new_content": preview,
                }))
            }
            _ => None,
        }
    }

    /// Validate, dispatch, and record a single tool call from the model.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn handle_tool_action(
        &mut self,
        tool: String,
        args: JsonValue,
        allowed_tools: &Option<HashSet<String>>,
        messages: &mut Vec<ChatMessage>,
        tool_cache: &mut HashMap<String, CachedToolObs>,
        read_paths: &mut HashSet<String>,
        last_tool_sig: &mut String,
        redundant_tool_streak: &mut usize,
        empty_search_streak: &mut usize,
        progress_rx: &mut tokio::sync::mpsc::UnboundedReceiver<(String, String, String)>,
        session_id: Option<&str>,
        tool_call_id: Option<String>,
    ) -> LoopControl {
        match self
            .pre_execute_tool(
                tool,
                args,
                allowed_tools,
                messages,
                tool_cache,
                read_paths,
                last_tool_sig,
                redundant_tool_streak,
                session_id,
                tool_call_id,
            )
            .await
        {
            PreExecOutcome::Blocked(ctrl) => ctrl,
            PreExecOutcome::Ready(call, exec) => {
                // Sync the current path-mode grants into tools so any Task
                // delegation spawned inside this call inherits them. Grants can
                // change during a run (user approves mid-turn), so re-sync here
                // rather than at engine construction time.
                self.tools.builtins.parent_path_modes = self.session_permissions.path_modes.clone();
                let tools_clone = self.tools.clone();
                let mut handle = tokio::task::spawn_blocking(move || tools_clone.execute(call));
                let result = loop {
                    tokio::select! {
                        res = &mut handle => {
                            break res.unwrap_or_else(|e| Err(anyhow::anyhow!("spawn_blocking join: {e}")));
                        }
                        _ = tokio::time::sleep(std::time::Duration::from_millis(150)) => {
                            self.drain_tool_progress(progress_rx).await;
                        }
                    }
                };
                self.drain_tool_progress(progress_rx).await;

                // Check cancellation after tool execution before feeding
                // the result back into context (spec: agentic-loop.md).
                if self.is_cancelled().await {
                    return LoopControl::Return(AgentOutcome::None);
                }

                self.post_execute_tool(exec, result, messages, tool_cache, empty_search_streak, session_id)
                    .await
            }
        }
    }

    /// Detect identical model responses and nudge or bail.
    pub(crate) async fn handle_repetition_check(
        &mut self,
        raw: &str,
        last_response: &mut String,
        streak: &mut usize,
        nudge_count: &mut usize,
        messages: &mut Vec<ChatMessage>,
        session_id: Option<&str>,
    ) -> Option<LoopControl> {
        if raw == last_response.as_str() {
            *streak += 1;
        } else {
            *streak = 0;
            *last_response = raw.to_string();
        }

        if *streak < 3 {
            return None;
        }

        *nudge_count += 1;
        if *nudge_count >= 2 {
            let message = self.prompt_store.render_or_fallback(
                crate::prompts::keys::BAILOUT_REPETITION_LOOP,
                &[("count", &(*streak + 1).to_string())],
            );
            let _ = self
                .persist_assistant_message(&message, session_id)
                .await;
            self.active_skill = None;
            return Some(LoopControl::Return(AgentOutcome::None));
        }

        messages.push(self.tool_result_msg(
            self.prompt_store.render_or_fallback(crate::prompts::NUDGE_REPETITION, &[]),
        ));
        self.push_context_record(
            ContextType::Error,
            Some("loop_detected".to_string()),
            self.agent_id.clone(),
            None,
            "Model trapped in a loop. Nudging with a warning message.".to_string(),
            serde_json::json!({ "streak": *streak + 1 }),
        );
        *streak = 0;
        Some(LoopControl::Continue)
    }

}
