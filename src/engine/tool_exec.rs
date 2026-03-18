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
    /// `parser` converts the selected option label into a `PermissionAction`.
    /// Returns `None` if no bridge is available (e.g. CLI mode).
    async fn ask_permission(
        &self,
        tool: &str,
        question: tools::AskUserQuestion,
        parser: fn(&str, &str) -> permission::PermissionAction,
    ) -> Option<permission::PermissionAction> {
        let bridge = match self.tools.ask_user_bridge() {
            Some(b) => Arc::clone(b),
            None => return None,
        };

        let question_id = uuid::Uuid::new_v4().to_string();
        let agent_id = self.agent_id.clone().unwrap_or_default();

        // Emit SSE event to push the permission question to the UI.
        info!("Permission: awaiting user approval for '{}'", tool);
        let questions = vec![question];
        let _ = bridge.events_tx.send(crate::server::ServerEvent::AskUser {
            agent_id: agent_id.clone(),
            question_id: question_id.clone(),
            questions: questions.clone(),
            session_id: bridge.session_id.clone(),
        });

        // Create a oneshot channel and register it for the response endpoint.
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

        // Block until the user responds or timeout (5 minutes).
        let response = tokio::time::timeout(std::time::Duration::from_secs(8 * 3600), rx).await;

        // Cleanup: remove from pending map regardless of outcome.
        bridge.pending.lock().await.remove(&question_id);

        match response {
            Ok(Ok(answers)) => {
                let answer = answers.first();
                // If the user typed custom text (via "Other..."), treat as deny + relay message.
                let custom = answer.and_then(|a| a.custom_text.as_deref()).unwrap_or("");
                if !custom.is_empty() {
                    info!("Permission: '{}' → DenyWithMessage", tool);
                    return Some(permission::PermissionAction::DenyWithMessage(custom.to_string()));
                }
                let selected = answer
                    .and_then(|a| a.selected.first())
                    .map(|s| s.as_str())
                    .unwrap_or("Cancel");
                let action = parser(selected, tool);
                info!("Permission: '{}' → {:?}", tool, action);
                Some(action)
            }
            Ok(Err(_)) => {
                warn!("Permission: '{}' channel closed → denied", tool);
                Some(permission::PermissionAction::Deny)
            }
            Err(_) => {
                warn!("Permission: '{}' timed out → stopping agent", tool);
                None // Timeout: no user response, signal to stop the loop
            }
        }
    }

    /// Ask the user for Bash permission with command-level granularity.
    /// Returns `(PermissionAction, Option<permission_key>)`.
    async fn ask_bash_permission(
        &self,
        command: &str,
        question: tools::AskUserQuestion,
        pattern: Option<&str>,
    ) -> Option<(permission::PermissionAction, Option<String>)> {
        let bridge = match self.tools.ask_user_bridge() {
            Some(b) => Arc::clone(b),
            None => return None,
        };

        let question_id = uuid::Uuid::new_v4().to_string();
        let agent_id = self.agent_id.clone().unwrap_or_default();

        info!(
            "Permission: awaiting user approval for Bash '{}' (session_id={:?})",
            command, bridge.session_id
        );
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
                // If the user typed custom text (via "Other..."), treat as deny + relay message.
                let custom = answer.and_then(|a| a.custom_text.as_deref()).unwrap_or("");
                if !custom.is_empty() {
                    info!("Permission: Bash '{}' → DenyWithMessage", command);
                    return Some((permission::PermissionAction::DenyWithMessage(custom.to_string()), None));
                }
                let selected = answer
                    .and_then(|a| a.selected.first())
                    .map(|s| s.as_str())
                    .unwrap_or("Cancel");
                let (action, key) =
                    permission::parse_bash_permission_answer(selected, "Bash", pattern);
                info!("Permission: Bash '{}' → {:?} key={:?}", command, action, key);
                Some((action, key))
            }
            Ok(Err(_)) => {
                warn!("Permission: Bash channel closed → denied");
                Some((permission::PermissionAction::Deny, None))
            }
            Err(_) => {
                warn!("Permission: Bash timed out → stopping agent");
                None // Timeout: no user response, signal to stop the loop
            }
        }
    }

    /// Ask the user for file-scoped Write/Edit permission.
    /// Returns `(PermissionAction, Option<permission_key>)`.
    async fn ask_file_permission(
        &self,
        file_path: &str,
        question: tools::AskUserQuestion,
        tool: &str,
        pattern: &str,
    ) -> Option<(permission::PermissionAction, Option<String>)> {
        let bridge = match self.tools.ask_user_bridge() {
            Some(b) => Arc::clone(b),
            None => return None,
        };

        let question_id = uuid::Uuid::new_v4().to_string();
        let agent_id = self.agent_id.clone().unwrap_or_default();

        info!("Permission: awaiting user approval for {} '{}'", tool, file_path);
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
                    info!("Permission: {} '{}' → DenyWithMessage", tool, file_path);
                    return Some((permission::PermissionAction::DenyWithMessage(custom.to_string()), None));
                }
                let selected = answer
                    .and_then(|a| a.selected.first())
                    .map(|s| s.as_str())
                    .unwrap_or("Cancel");
                let (action, key) =
                    permission::parse_file_permission_answer(selected, tool, pattern);
                info!("Permission: {} '{}' → {:?} key={:?}", tool, file_path, action, key);
                Some((action, key))
            }
            Ok(Err(_)) => {
                warn!("Permission: {} channel closed → denied", tool);
                Some((permission::PermissionAction::Deny, None))
            }
            Err(_) => {
                warn!("Permission: {} timed out → stopping agent", tool);
                None
            }
        }
    }

    /// Pre-execution phase: validate permissions, record context, check caches,
    /// and emit SSE "start" events. Returns `Ready` with the prepared ToolCall
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
        info!("Tool: {} {}", canonical_tool, safe_args);
        if canonical_tool == "Read" {
            if let Some(path) = normalize_tool_path_arg(&self.cfg.ws_root, &args) {
                read_paths.insert(path);
            }
        }

        // Compute tool call signature early for denied-check and later redundancy tracking.
        let sig = tool_call_signature(&canonical_tool, &args);

        // --- write-safety gate ---
        if matches!(canonical_tool.as_str(), "Write" | "Edit") {
            if let Some(path) = normalize_tool_path_arg(&self.cfg.ws_root, &args) {
                let existing = self.cfg.ws_root.join(&path).exists();
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

        // --- mission tool restriction gate ---
        // Block tools not in mission_allowed_tools (set by permission tier).
        if let Some(ref allowed) = self.cfg.mission_allowed_tools {
            if !allowed.contains(&canonical_tool) {
                let msg = format!(
                    "Tool '{}' is not allowed by this mission's permission tier. Available tools: {}",
                    canonical_tool,
                    allowed.iter().cloned().collect::<Vec<_>>().join(", ")
                );
                messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                return PreExecOutcome::Blocked(LoopControl::Continue);
            }
        }

        // --- tool permission gate ---
        let is_destructive = permission::is_destructive_tool(&canonical_tool);
        let is_web = permission::is_web_tool(&canonical_tool);

        // Extract bash command for pattern-based permission checking.
        let bash_command = if canonical_tool == "Bash" {
            args.get("cmd")
                .or_else(|| args.get("command"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        } else {
            None
        };

        // Extract file path for Write/Edit pattern-based permission checking.
        let file_path_arg = if matches!(canonical_tool.as_str(), "Write" | "Edit") {
            normalize_tool_path_arg(&self.cfg.ws_root, &args)
        } else {
            None
        };

        let already_allowed = self.permission_store.check(
            &canonical_tool,
            bash_command.as_deref().or(file_path_arg.as_deref()),
        );
        let needs_permission = if already_allowed {
            false
        } else {
            match self.cfg.tool_permission_mode {
                crate::config::ToolPermissionMode::Auto => false,
                crate::config::ToolPermissionMode::Ask => is_destructive || is_web,
                // AcceptEdits: auto-approve Write/Edit but still prompt for Bash and web tools.
                crate::config::ToolPermissionMode::AcceptEdits => {
                    if matches!(canonical_tool.as_str(), "Write" | "Edit") {
                        false
                    } else {
                        is_destructive || is_web
                    }
                }
            }
        };
        if is_destructive || is_web {
            info!(
                "Permission check for '{}': mode={:?}, already_allowed={}, destructive={}, web={} → needs_permission={}",
                canonical_tool, self.cfg.tool_permission_mode, already_allowed, is_destructive, is_web, needs_permission
            );
        }

        // Hard-block project-denied tools (no prompt, immediate rejection).
        if self.permission_store.is_denied(
            &canonical_tool,
            bash_command.as_deref().or(file_path_arg.as_deref()),
        ) {
            let summary =
                permission::permission_target_summary(&canonical_tool, &args, &self.cfg.ws_root);
            let msg = format!(
                "Permission denied: {} '{}' is blocked by a project deny rule in .linggen/permissions.json. \
                 Edit that file to remove the deny rule if needed.",
                canonical_tool, summary
            );
            messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
            return PreExecOutcome::Blocked(LoopControl::Continue);
        }

        // --- mission bash prefix restriction ---
        // When bash_allow_prefixes is set, block bash commands not matching any prefix.
        if canonical_tool == "Bash" {
            if let Some(ref prefixes) = self.cfg.bash_allow_prefixes {
                let cmd = bash_command.as_deref().unwrap_or("");
                let cmd_trimmed = cmd.trim();
                let allowed = prefixes.iter().any(|prefix| {
                    cmd_trimmed.starts_with(prefix)
                });
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

        if needs_permission {
            // Auto-block retries of tool calls the user already denied this session.
            if self.denied_tool_sigs.contains(&sig) {
                let summary =
                    permission::permission_target_summary(&canonical_tool, &args, &self.cfg.ws_root);
                let msg = self.prompt_store.render_or_fallback(
                    crate::prompts::keys::PERMISSION_DENIED,
                    &[("tool", &canonical_tool), ("summary", &summary)],
                );
                messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                return PreExecOutcome::Blocked(LoopControl::Continue);
            }

            let summary =
                permission::permission_target_summary(&canonical_tool, &args, &self.cfg.ws_root);

            if canonical_tool == "Bash" {
                // Bash uses command-level granularity with glob patterns.
                let cmd = bash_command.as_deref().unwrap_or("");
                if cmd.is_empty() {
                    let msg = "Error: Bash tool requires a 'cmd' argument with the command to run.".to_string();
                    messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                    return PreExecOutcome::Blocked(LoopControl::Continue);
                }
                let pattern = permission::derive_command_pattern(cmd);
                let question = permission::build_bash_permission_question(
                    cmd,
                    pattern.as_deref(),
                );
                match self
                    .ask_bash_permission(cmd, question, pattern.as_deref())
                    .await
                {
                    Some((permission::PermissionAction::AllowOnce, _)) => { /* proceed */ }
                    Some((permission::PermissionAction::AllowSession, Some(key))) => {
                        self.permission_store.allow_for_session(&key);
                    }
                    Some((permission::PermissionAction::AllowProject, Some(key))) => {
                        self.permission_store.allow_for_project(&key);
                    }
                    Some((permission::PermissionAction::Deny, _)) => {
                        self.denied_tool_sigs.insert(sig.clone());
                        let msg = self.prompt_store.render_or_fallback(
                            crate::prompts::keys::PERMISSION_DENIED,
                            &[("tool", "Bash"), ("summary", &summary)],
                        );
                        messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                        return PreExecOutcome::Blocked(LoopControl::Continue);
                    }
                    Some((permission::PermissionAction::DenyProject, key)) => {
                        if let Some(k) = key {
                            self.permission_store.deny_for_project(&k);
                        }
                        self.denied_tool_sigs.insert(sig.clone());
                        let msg = self.prompt_store.render_or_fallback(
                            crate::prompts::keys::PERMISSION_DENIED,
                            &[("tool", "Bash"), ("summary", &summary)],
                        );
                        messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                        return PreExecOutcome::Blocked(LoopControl::Continue);
                    }
                    Some((permission::PermissionAction::DenyWithMessage(user_msg), _)) => {
                        self.denied_tool_sigs.insert(sig.clone());
                        let msg = format!(
                            "Permission denied by user for Bash on '{}'. User says: {}",
                            summary, user_msg
                        );
                        messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                        return PreExecOutcome::Blocked(LoopControl::Continue);
                    }
                    None => {
                        // Timeout or no bridge — stop the loop so the agent goes idle.
                        let msg = self.prompt_store.render_or_fallback(
                            crate::prompts::keys::PERMISSION_TIMEOUT,
                            &[],
                        );
                        let _ = self
                            .persist_assistant_message(&msg, session_id)
                            .await;
                        return PreExecOutcome::Blocked(
                            LoopControl::Return(AgentOutcome::None),
                        );
                    }
                    _ => { /* proceed: AllowOnce, AllowSession/AllowProject without key */ }
                }
            } else if permission::is_web_tool(&canonical_tool) {
                let question =
                    permission::build_web_permission_question(&canonical_tool, &summary);
                match self.ask_permission(&canonical_tool, question, permission::parse_web_permission_answer).await {
                    Some(permission::PermissionAction::AllowOnce) => { /* proceed */ }
                    Some(permission::PermissionAction::AllowSession) => {
                        self.permission_store.allow_for_session(&canonical_tool);
                    }
                    Some(permission::PermissionAction::AllowProject) => {
                        self.permission_store.allow_for_project(&canonical_tool);
                    }
                    Some(permission::PermissionAction::Deny) => {
                        self.denied_tool_sigs.insert(sig.clone());
                        let msg = self.prompt_store.render_or_fallback(
                            crate::prompts::keys::PERMISSION_DENIED,
                            &[("tool", &canonical_tool), ("summary", &summary)],
                        );
                        messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                        return PreExecOutcome::Blocked(LoopControl::Continue);
                    }
                    Some(permission::PermissionAction::DenyProject) => {
                        self.permission_store.deny_for_project(&canonical_tool);
                        self.denied_tool_sigs.insert(sig.clone());
                        let msg = self.prompt_store.render_or_fallback(
                            crate::prompts::keys::PERMISSION_DENIED,
                            &[("tool", &canonical_tool), ("summary", &summary)],
                        );
                        messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                        return PreExecOutcome::Blocked(LoopControl::Continue);
                    }
                    Some(permission::PermissionAction::DenyWithMessage(user_msg)) => {
                        self.denied_tool_sigs.insert(sig.clone());
                        let msg = format!(
                            "Permission denied by user for {} on '{}'. User says: {}",
                            canonical_tool, summary, user_msg
                        );
                        messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                        return PreExecOutcome::Blocked(LoopControl::Continue);
                    }
                    None => {
                        let msg = self.prompt_store.render_or_fallback(
                            crate::prompts::keys::PERMISSION_TIMEOUT,
                            &[],
                        );
                        let _ = self
                            .persist_assistant_message(&msg, session_id)
                            .await;
                        return PreExecOutcome::Blocked(
                            LoopControl::Return(AgentOutcome::None),
                        );
                    }
                }
            } else if matches!(canonical_tool.as_str(), "Write" | "Edit") {
                // Write/Edit use file-scoped permission prompts (directory glob patterns).
                let file_rel = file_path_arg.as_deref().unwrap_or(&summary);
                let pattern = permission::derive_file_pattern(file_rel);
                let question = permission::build_file_permission_question(
                    &canonical_tool,
                    file_rel,
                    &pattern,
                );
                match self
                    .ask_file_permission(file_rel, question, &canonical_tool, &pattern)
                    .await
                {
                    Some((permission::PermissionAction::AllowOnce, _)) => { /* proceed */ }
                    Some((permission::PermissionAction::AllowSession, Some(key))) => {
                        self.permission_store.allow_for_session(&key);
                    }
                    Some((permission::PermissionAction::AllowProject, Some(key))) => {
                        self.permission_store.allow_for_project(&key);
                    }
                    Some((permission::PermissionAction::Deny, _)) => {
                        self.denied_tool_sigs.insert(sig.clone());
                        let msg = self.prompt_store.render_or_fallback(
                            crate::prompts::keys::PERMISSION_DENIED,
                            &[("tool", &canonical_tool), ("summary", &summary)],
                        );
                        messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                        return PreExecOutcome::Blocked(LoopControl::Continue);
                    }
                    Some((permission::PermissionAction::DenyProject, key)) => {
                        if let Some(k) = key {
                            self.permission_store.deny_for_project(&k);
                        }
                        self.denied_tool_sigs.insert(sig.clone());
                        let msg = self.prompt_store.render_or_fallback(
                            crate::prompts::keys::PERMISSION_DENIED,
                            &[("tool", &canonical_tool), ("summary", &summary)],
                        );
                        messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                        return PreExecOutcome::Blocked(LoopControl::Continue);
                    }
                    Some((permission::PermissionAction::DenyWithMessage(user_msg), _)) => {
                        self.denied_tool_sigs.insert(sig.clone());
                        let msg = format!(
                            "Permission denied by user for {} on '{}'. User says: {}",
                            canonical_tool, summary, user_msg
                        );
                        messages.push(self.tool_result_msg_for(msg, &tool_call_id, &canonical_tool));
                        return PreExecOutcome::Blocked(LoopControl::Continue);
                    }
                    None => {
                        let msg = self.prompt_store.render_or_fallback(
                            crate::prompts::keys::PERMISSION_TIMEOUT,
                            &[],
                        );
                        let _ = self
                            .persist_assistant_message(&msg, session_id)
                            .await;
                        return PreExecOutcome::Blocked(
                            LoopControl::Return(AgentOutcome::None),
                        );
                    }
                    _ => { /* proceed: AllowOnce, AllowSession/AllowProject without key */ }
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
                })
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
                    self.session_storage_root(),
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

    /// Post-execution phase: render and cache the result, emit SSE "done"/"failed"
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
                        normalize_tool_path_arg(&self.cfg.ws_root, &original_args)
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
                        })
                        .await;
                    manager
                        .send_event(crate::agent_manager::AgentEvent::StateUpdated)
                        .await;
                    manager
                        .send_event(crate::agent_manager::AgentEvent::AgentStatus {
                            agent_id,
                            status: "thinking".to_string(),
                            detail: Some(format!("Thinking ({})", self.model_id)),
                            parent_id: self.parent_agent_id.clone(),
                        })
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
                    manager
                        .send_event(crate::agent_manager::AgentEvent::ContentBlockUpdate {
                            agent_id: agent_id.clone(),
                            block_id: block_id.clone(),
                            status: Some("failed".to_string()),
                            summary: Some(tool_failed_status.clone()),
                            is_error: Some(true),
                            parent_id: self.parent_agent_id.clone(),
                            extra: None,
                        })
                        .await;
                    manager
                        .send_event(crate::agent_manager::AgentEvent::AgentStatus {
                            agent_id,
                            status: "thinking".to_string(),
                            detail: Some(format!("Thinking ({})", self.model_id)),
                            parent_id: self.parent_agent_id.clone(),
                        })
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
                let rel = normalize_tool_path_arg(&self.cfg.ws_root, original_args)
                    .unwrap_or_else(|| path.to_string());
                let file_path = self.cfg.ws_root.join(&rel);
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

                let rel = normalize_tool_path_arg(&self.cfg.ws_root, original_args)
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
