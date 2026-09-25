use super::types::*;
use crate::engine::permission;
use crate::engine::render::{
    normalize_tool_path_arg, render_tool_result, render_tool_result_public,
    sanitize_tool_args_for_display, tool_call_signature,
};
use crate::engine::tools::{self, ToolCall};
use crate::message::ChatMessage;
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
        let _ = bridge
            .events_tx
            .send(crate::engine::events::ServerEvent::AskUser {
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
                    return Some(permission::PermissionAction::DenyWithMessage(
                        custom.to_string(),
                    ));
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
                    "Allow for this session"
                    | "Run in current mode"
                    | "Allow this site for the session" => {
                        Some(permission::PermissionAction::AllowSession)
                    }
                    other => {
                        // Match both the current "Switch this folder to {mode}"
                        // wording from build_exceeds_ceiling_question and the
                        // legacy "Switch to {mode} mode" / "Allow read on …"
                        // forms still in older prompts. Anything starting with
                        // "Switch" is treated as a persistent grant approval.
                        if other.starts_with("Switch ") || other.starts_with("Allow read on ") {
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
    ///
    /// The gates run in a fixed order; the first to refuse ends the call.
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
        let t = ToolTurn {
            tool: &tool,
            canonical: &canonical_tool,
            args: &args,
            session_id,
            tool_call_id: &tool_call_id,
        };

        if let Some(stop) = self.gate_pet_scoped(&t, allowed_tools, messages).await {
            return stop;
        }
        if let Some(stop) = self.gate_allowed_list(&t, allowed_tools, messages).await {
            return stop;
        }

        let safe_args = self.record_tool_call(&t, read_paths);

        // Compute tool call signature early for denied-check and later redundancy tracking.
        let sig = tool_call_signature(&canonical_tool, &args);

        if let Some(stop) = self.gate_write_safety(&t, read_paths, messages).await {
            return stop;
        }
        if let Some(stop) = self.gate_restrictions(&t, messages) {
            return stop;
        }
        if let Some(stop) = self.gate_permission(&t, messages).await {
            return stop;
        }

        // Browser_* mutating actions are gated by the extension itself
        // (browser-control-spec.md): the Allow prompt renders in the browser
        // and the trust list lives in extension storage — one gate for every
        // caller (engine sessions and /mcp alike). A user deny surfaces here
        // as a normal `not_permitted` tool error.

        if let Some(stop) = self.gate_redundancy_and_cache(
            &t,
            &sig,
            messages,
            tool_cache,
            last_tool_sig,
            redundant_tool_streak,
        ) {
            return stop;
        }

        // --- status lines ---
        let tool_done_status = crate::engine::tool_render::tool_status_line(
            &canonical_tool,
            Some(&args),
            crate::engine::tool_render::ToolStatusPhase::Done,
        );
        let tool_failed_status = crate::engine::tool_render::tool_status_line(
            &canonical_tool,
            Some(&args),
            crate::engine::tool_render::ToolStatusPhase::Failed,
        );

        let block_id = self.announce_tool_start(&t, &safe_args).await;

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

    /// Answer the model's call with `msg` instead of running it, and go on.
    fn refuse(
        &self,
        t: &ToolTurn<'_>,
        messages: &mut Vec<ChatMessage>,
        msg: String,
    ) -> Option<PreExecOutcome> {
        messages.push(self.tool_result_msg_for(msg, t.tool_call_id, t.canonical));
        Some(PreExecOutcome::Blocked(LoopControl::Continue))
    }

    /// Pet-scoping gate (defense-in-depth).
    ///
    /// Pet/companion tools (Express) drive the on-screen avatar and only run
    /// for an agent that lists them EXPLICITLY. The model-facing schema already
    /// hides them from `*` (wildcard) agents (tool_registry `is_allowed`), but
    /// the permission gate below is skipped when allowed_tools is None (`*`), so
    /// a hallucinated or legacy emission by a worker agent would otherwise
    /// execute. Mirror the schema rule here: only an explicit listing grants it.
    async fn gate_pet_scoped(
        &mut self,
        t: &ToolTurn<'_>,
        allowed_tools: &Option<HashSet<String>>,
        messages: &mut Vec<ChatMessage>,
    ) -> Option<PreExecOutcome> {
        if tools::is_pet_scoped(t.canonical)
            && !matches!(allowed_tools, Some(set) if set.contains(t.canonical))
        {
            let rendered = format!(
                "tool_not_allowed: tool={} reason=pet_scoped: only the pet agent may drive the avatar; ask Yinyue via agent_chat instead",
                t.canonical
            );
            self.upsert_observation("error", t.canonical, rendered.clone());
            let _ = self
                .persist_observation(t.canonical, &rendered, t.session_id)
                .await;
            return self.refuse(t, messages, rendered);
        }
        None
    }

    /// The agent's allowed-tools list, when it has one.
    async fn gate_allowed_list(
        &mut self,
        t: &ToolTurn<'_>,
        allowed_tools: &Option<HashSet<String>>,
        messages: &mut Vec<ChatMessage>,
    ) -> Option<PreExecOutcome> {
        let allowed = allowed_tools.as_ref()?;
        if self.is_tool_allowed(allowed, t.tool) {
            return None;
        }
        let mut allowed_list = allowed.iter().cloned().collect::<Vec<_>>();
        allowed_list.sort();
        let rendered = format!(
            "tool_not_allowed: tool={} canonical={} allowed={}",
            t.tool,
            t.canonical,
            allowed_list.join(",")
        );
        self.upsert_observation("error", t.canonical, rendered.clone());
        let _ = self
            .persist_observation(t.canonical, &rendered, t.session_id)
            .await;
        let msg = self.prompt_store.render_or_fallback(
            crate::prompts::keys::TOOL_NOT_ALLOWED,
            &[("tool", t.tool), ("allowed_list", &allowed_list.join(", "))],
        );
        self.refuse(t, messages, msg)
    }

    /// Record the call in context and the log; remember a Read's path for
    /// the write-safety gate. Returns the args as shown to people.
    fn record_tool_call(
        &mut self,
        t: &ToolTurn<'_>,
        read_paths: &mut HashSet<String>,
    ) -> JsonValue {
        let safe_args = sanitize_tool_args_for_display(t.canonical, t.args);
        self.upsert_context_record_by_type_name(
            ContextType::ToolCall,
            t.canonical,
            self.agent_id.clone(),
            Some(self.outbound_target()),
            serde_json::to_string(&safe_args).unwrap_or_else(|_| "{}".to_string()),
            serde_json::json!({ "args": safe_args.clone() }),
        );
        let log_run = self.run_id.clone().unwrap_or_else(|| "root".to_string());
        info!("[{}] Tool: {} {}", log_run, t.canonical, safe_args);
        if t.canonical == "Read" {
            if let Some(path) = normalize_tool_path_arg(&self.tools.builtins.cwd(), t.args) {
                read_paths.insert(path);
            }
        }
        safe_args
    }

    /// Write-safety gate: Write/Edit on an existing file not Read first.
    async fn gate_write_safety(
        &mut self,
        t: &ToolTurn<'_>,
        read_paths: &HashSet<String>,
        messages: &mut Vec<ChatMessage>,
    ) -> Option<PreExecOutcome> {
        if !matches!(t.canonical, "Write" | "Edit") {
            return None;
        }
        let path = normalize_tool_path_arg(&self.tools.builtins.cwd(), t.args)?;
        let existing = self.tools.builtins.cwd().join(&path).exists();
        if !existing || read_paths.contains(&path) {
            return None;
        }
        let action = if t.canonical == "Edit" {
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
                    .persist_observation(action, &rendered, t.session_id)
                    .await;
                let msg = self.prompt_store.render_or_fallback(
                    crate::prompts::keys::WRITE_SAFETY_BLOCKED,
                    &[("rendered", &rendered)],
                );
                self.refuse(t, messages, msg)
            }
            crate::config::WriteSafetyMode::Warn => {
                let rendered = format!(
                    "tool_warning: tool={} warning=writing_existing_file_without_prior_read path='{}'",
                    action, path
                );
                self.upsert_observation("warning", action, rendered.clone());
                let _ = self
                    .persist_observation(action, &rendered, t.session_id)
                    .await;
                None
            }
            crate::config::WriteSafetyMode::Off => None,
        }
    }

    /// Restriction gates (defense-in-depth), in order: the config-level tool
    /// set, tools withheld this turn, a guest's Skill, a consumer's skill
    /// list, and a mission's bash prefixes.
    fn gate_restrictions(
        &self,
        t: &ToolTurn<'_>,
        messages: &mut Vec<ChatMessage>,
    ) -> Option<PreExecOutcome> {
        let msg = self
            .config_restriction(t)
            .or_else(|| self.withheld_restriction(t))
            .or_else(|| self.guest_restriction(t))
            .or_else(|| self.consumer_skill_restriction(t))
            .or_else(|| self.bash_prefix_restriction(t))?;
        self.refuse(t, messages, msg)
    }

    /// Blocks tools not allowed by mission tiers or consumer room settings.
    /// The prompt already excludes these tools, but this catches hallucinations.
    fn config_restriction(&self, t: &ToolTurn<'_>) -> Option<String> {
        if self.cfg.is_tool_allowed(t.canonical) {
            return None;
        }
        let available = self
            .cfg
            .effective_tool_restrictions()
            .map(|s| s.into_iter().collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        Some(format!(
            "Tool '{}' is not available. Allowed: {}",
            t.canonical, available
        ))
    }

    /// A tool this turn's caller withheld is refused even if the model
    /// names it (it was never offered).
    fn withheld_restriction(&self, t: &ToolTurn<'_>) -> Option<String> {
        self.is_withheld(t.canonical).then(|| {
            format!(
                "tool_not_allowed: tool={} reason=withheld: not available on this turn",
                t.canonical
            )
        })
    }

    /// A guest takes up no skill at a table that isn't its own, even when
    /// its list is `*` (no allowed set to check above) — the skill's
    /// habits would come with it.
    fn guest_restriction(&self, t: &ToolTurn<'_>) -> Option<String> {
        (t.canonical == "Skill" && self.is_guest_seat()).then(|| {
            "tool_not_allowed: tool=Skill reason=guest: a guest brings only its own tools to this session".to_string()
        })
    }

    /// When consumer_allowed_skills is set, block Skill invocations not in the list.
    fn consumer_skill_restriction(&self, t: &ToolTurn<'_>) -> Option<String> {
        if t.canonical != "Skill" {
            return None;
        }
        let allowed_skills = self.cfg.consumer_allowed_skills.as_ref()?;
        let skill_name = t
            .args
            .get("skill")
            .or_else(|| t.args.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        (!allowed_skills.contains(skill_name))
            .then(|| format!("Skill '{}' is not available to consumers.", skill_name))
    }

    /// Mission bash prefix restriction (legacy, kept for backward compat).
    fn bash_prefix_restriction(&self, t: &ToolTurn<'_>) -> Option<String> {
        if t.canonical != "Bash" {
            return None;
        }
        let prefixes = self.cfg.bash_allow_prefixes.as_ref()?;
        let cmd = bash_command_arg(t.args);
        let cmd_trimmed = cmd.as_deref().unwrap_or("").trim();
        let allowed = prefixes
            .iter()
            .any(|prefix| cmd_trimmed.starts_with(prefix));
        (!allowed).then(|| {
            format!(
                "Bash command not allowed by this mission's permission tier. \
                 Command: '{}'. Allowed prefixes: {}",
                cmd_trimmed,
                prefixes.join(", ")
            )
        })
    }

    /// The path/tier permission gate (permission-spec.md), asking the user
    /// when the call exceeds the session's grant.
    async fn gate_permission(
        &mut self,
        t: &ToolTurn<'_>,
        messages: &mut Vec<ChatMessage>,
    ) -> Option<PreExecOutcome> {
        // Skill data tools (empty cmd, e.g. PageUpdate) pass their args
        // straight through as a content block — they don't read files, run
        // commands, or reach outside the process. Running them through the
        // path/tier permission gate produces nonsense prompts like
        // "PageUpdate /Users/<you> — switch to admin?" because the gate
        // synthesizes cwd as the file_path_arg and classifies unknown tools
        // as Admin tier. Skip the gate entirely for pure data tools.
        if self.tools.is_skill_data_tool(t.canonical) {
            return None;
        }

        // Extract bash command and file path for permission checking.
        let bash_command = if t.canonical == "Bash" {
            bash_command_arg(t.args)
        } else {
            None
        };
        let file_path_arg = if matches!(t.canonical, "Write" | "Edit" | "Read") {
            normalize_tool_path_arg(&self.tools.builtins.cwd(), t.args)
        } else {
            // For tools without an explicit file path (Glob, Grep, Task, etc.),
            // use the agent's cwd so permission checks resolve against the
            // session's path_mode grants for the workspace.
            Some(self.tools.builtins.cwd().to_string_lossy().to_string())
        };

        // Skill-declared tier wins over the built-in table. For HTTP and
        // shell skill tools that declare `tier: read|edit|admin` in their
        // manifest, parse it into a PermissionMode and pass to
        // check_permission as an override. Unknown / missing tier falls
        // back to the built-in classification.
        let skill_tier_override = self
            .tools
            .skill_tools
            .get(t.canonical)
            .and_then(|def| def.tier.as_deref())
            .and_then(permission::parse_skill_tier);

        let check_result = permission::check_permission(
            t.canonical,
            bash_command.as_deref(),
            file_path_arg.as_deref(),
            &self.tools.builtins.cwd(),
            &self.session_permissions,
            skill_tier_override,
        );

        let prompt_kind = match check_result {
            permission::PermissionCheckResult::Allowed => return None,
            permission::PermissionCheckResult::Blocked(reason) => {
                info!("Permission blocked: {} — {}", t.canonical, reason);
                let msg = self.permission_denied_msg(t);
                return self.refuse(t, messages, msg);
            }
            permission::PermissionCheckResult::NeedsPrompt(prompt_kind) => prompt_kind,
        };

        // Non-interactive sessions (mission, proxy consumer) cannot prompt —
        // return permission-needed immediately.
        if !self.session_permissions.interactive {
            let msg = self.permission_denied_msg(t);
            return self.refuse(t, messages, msg);
        }

        let permission::PromptKind::ExceedsCeiling {
            target_mode,
            path,
            tool_summary,
        } = prompt_kind;
        let question =
            permission::build_exceeds_ceiling_question(&tool_summary, &target_mode, &path);
        match self.ask_permission_raw(t.canonical, question).await {
            Some(permission::PermissionAction::AllowOnce) => None,
            Some(permission::PermissionAction::AllowSession) => {
                // Switch mode — update path_modes and save.
                self.session_permissions.set_path_mode(&path, target_mode);
                if let Some(ref sdir) = self.session_dir {
                    self.session_permissions.save(sdir);
                }
                // Notify UI so the mode badge updates.
                if let Some(manager) = self.tools.get_manager() {
                    manager
                        .send_event(
                            crate::engine::agent::AgentEvent::StateUpdated,
                            self.session_id.clone(),
                        )
                        .await;
                }
                None
            }
            Some(permission::PermissionAction::Deny) => {
                let msg = self.permission_denied_msg(t);
                self.refuse(t, messages, msg)
            }
            Some(permission::PermissionAction::DenyWithMessage(user_msg)) => {
                let summary = self.permission_summary(t);
                let msg = format!(
                    "Permission denied by user for {} '{}'. User says: {}",
                    t.canonical, summary, user_msg
                );
                self.refuse(t, messages, msg)
            }
            None => {
                let msg = self
                    .prompt_store
                    .render_or_fallback(crate::prompts::keys::PERMISSION_TIMEOUT, &[]);
                let _ = self.persist_assistant_message(&msg, t.session_id).await;
                Some(PreExecOutcome::Blocked(LoopControl::Return(
                    AgentOutcome::None,
                )))
            }
        }
    }

    fn permission_summary(&self, t: &ToolTurn<'_>) -> String {
        permission::permission_target_summary(t.canonical, t.args, &self.tools.builtins.cwd())
    }

    fn permission_denied_msg(&self, t: &ToolTurn<'_>) -> String {
        let summary = self.permission_summary(t);
        self.prompt_store.render_or_fallback(
            crate::prompts::keys::PERMISSION_DENIED,
            &[("tool", t.canonical), ("summary", &summary)],
        )
    }

    /// Redundancy / cache gates.
    ///
    /// Only pure workspace reads take part (`tool_cacheable`). Anything
    /// else runs every time and is never counted toward the redundant-loop
    /// nudge: repeating an action or a live read is legitimate, and the
    /// model sees each real result (a refusal, a changed page) rather than
    /// a replay. It also breaks a run of identical reads — the world may
    /// have changed in between.
    fn gate_redundancy_and_cache(
        &mut self,
        t: &ToolTurn<'_>,
        sig: &str,
        messages: &mut Vec<ChatMessage>,
        tool_cache: &HashMap<String, CachedToolObs>,
        last_tool_sig: &mut String,
        redundant_tool_streak: &mut usize,
    ) -> Option<PreExecOutcome> {
        let canonical_tool = t.canonical;
        let cacheable = tools::tool_cacheable(canonical_tool);

        if !cacheable {
            *redundant_tool_streak = 0;
            last_tool_sig.clear();
        } else if sig == *last_tool_sig {
            *redundant_tool_streak += 1;
        } else {
            *redundant_tool_streak = 0;
            *last_tool_sig = sig.to_string();
        }

        if cacheable && *redundant_tool_streak >= 3 {
            let loop_breaker_prompt = self
                .cfg
                .prompt_loop_breaker
                .as_deref()
                .map(|template| Self::render_loop_breaker_prompt(template, canonical_tool))
                .unwrap_or_else(|| {
                    self.prompt_store.render_or_fallback(
                        crate::prompts::NUDGE_REDUNDANT_TOOL,
                        &[("tool", canonical_tool)],
                    )
                });
            messages.push(self.tool_result_msg_for(
                loop_breaker_prompt,
                t.tool_call_id,
                canonical_tool,
            ));
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
            return Some(PreExecOutcome::Blocked(LoopControl::Continue));
        }

        if cacheable {
            if let Some(cached) = tool_cache.get(sig) {
                self.upsert_observation("tool", canonical_tool, cached.model.clone());
                let msg = self.observation_text("tool", canonical_tool, &cached.model);
                return self.refuse(t, messages, msg);
            }
        }
        None
    }

    /// Tell the UI what tool we're about to use, and keep it in the session
    /// transcript. Returns the call's block id.
    async fn announce_tool_start(&mut self, t: &ToolTurn<'_>, safe_args: &JsonValue) -> String {
        let block_id = uuid::Uuid::new_v4().to_string();
        let Some(manager) = self.tools.get_manager() else {
            return block_id;
        };
        let from = self
            .agent_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let target = self.outbound_target();
        // Emit structured ContentBlockStart for the Web UI.
        let compact_args = serde_json::to_string(safe_args).unwrap_or_else(|_| "{}".to_string());
        let _ = manager
            .send_event(
                crate::engine::agent::AgentEvent::ContentBlockStart {
                    agent_id: from.clone(),
                    block_id: block_id.clone(),
                    block_type: "tool_use".to_string(),
                    tool: Some(t.canonical.to_string()),
                    args: Some(compact_args),
                    parent_id: self.parent_agent_id.clone(),
                    run_id: self.run_id.clone(),
                    parent_run_id: self.parent_run_id.clone(),
                },
                self.session_id.clone(),
            )
            .await;
        // Persist tool call to session store as an observation (not loaded
        // into chat history on reload — tool results are ephemeral context).
        // Skip for subagents: they share the parent's session_id, so
        // every subagent tool call would otherwise show up as a `system`
        // observation in the parent's transcript on reload.
        if self.tools.builtins.delegation_depth() == 0 {
            let tool_msg = serde_json::json!({
                "type": "tool",
                "tool": t.canonical,
                "args": safe_args
            })
            .to_string();
            manager
                .add_chat_message(
                    &self.tools.builtins.cwd(),
                    t.session_id.unwrap_or("default"),
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
        block_id
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
                self.note_closing_ask(&canonical_tool, &result);
                let rendered_model = render_tool_result(&result);
                let rendered_public = render_tool_result_public(&result);

                // A pure read is remembered; anything else may have changed
                // what the reads saw — a Write, a shell command, a skill tool
                // writing its state — so the remembered reads go.
                if tools::tool_cacheable(&canonical_tool) {
                    tool_cache.insert(
                        sig,
                        CachedToolObs {
                            model: rendered_model.clone(),
                        },
                    );
                } else {
                    tool_cache.clear();
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
                    let extra = self
                        .build_tool_extra(&canonical_tool, &original_args)
                        .or_else(|| {
                            if canonical_tool == "Bash" {
                                if let tools::ToolResult::CommandOutput {
                                    ref stdout,
                                    ref stderr,
                                    ..
                                } = result
                                {
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
                        .send_event(
                            crate::engine::agent::AgentEvent::ContentBlockUpdate {
                                agent_id: agent_id.clone(),
                                block_id: block_id.clone(),
                                status: Some("done".to_string()),
                                summary: Some(tool_done_status.clone()),
                                is_error: Some(false),
                                parent_id: self.parent_agent_id.clone(),
                                extra,
                                run_id: self.run_id.clone(),
                                parent_run_id: self.parent_run_id.clone(),
                            },
                            self.session_id.clone(),
                        )
                        .await;
                    manager
                        .send_event(
                            crate::engine::agent::AgentEvent::StateUpdated,
                            self.session_id.clone(),
                        )
                        .await;
                    manager
                        .send_event(
                            crate::engine::agent::AgentEvent::AgentStatus {
                                agent_id,
                                status: "thinking".to_string(),
                                detail: Some(format!("Thinking ({})", self.model_id)),
                                parent_id: self.parent_agent_id.clone(),
                                run_id: self.run_id.clone(),
                                parent_run_id: self.parent_run_id.clone(),
                            },
                            self.session_id.clone(),
                        )
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
                    let _ = self.persist_assistant_message(&msg, session_id).await;
                }

                let obs_content = self.observation_text("tool", &canonical_tool, &rendered_model);
                let obs_msg = self.tool_result_msg_for(obs_content, &tool_call_id, &canonical_tool);

                self.push_tracked_message(messages, obs_msg);

                // Browser screenshots: attach the actual image so the model
                // sees what it captured — the text observation only records
                // that a capture happened.
                if canonical_tool == "Browser_screenshot" {
                    if let tools::ToolResult::Screenshot {
                        ref url,
                        ref base64,
                    } = result
                    {
                        if !base64.is_empty() {
                            let img_msg =
                                ChatMessage::new("user", format!("[screenshot of {url}]"))
                                    .with_images(vec![base64.clone()]);
                            self.push_tracked_message(messages, img_msg);
                        }
                    }
                }

                let is_empty_search = (canonical_tool == "Grep"
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
                    messages.push(
                        self.tool_result_msg(
                            self.prompt_store
                                .render_or_fallback(crate::prompts::keys::NUDGE_EMPTY_SEARCH, &[]),
                        ),
                    );
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

                // Skill tool side-effect: register the invoked skill's
                // tool defs into the session and set `active_skill`. This
                // lives at the dispatch layer (not inside `invoke_skill`)
                // because the engine handle isn't reachable from the
                // `Tools` struct that hosts tool implementations. The
                // result text (SKILL.md content) was already produced by
                // `invoke_skill`; this is the activation side-effect that
                // makes the skill's tools callable on the next turn.
                // Accumulates across invocations; collisions warn + win
                // newer (see `register_skill_tools` in skill_activation).
                if canonical_tool == "Skill" {
                    let skill_name = original_args
                        .get("skill")
                        .or_else(|| original_args.get("name"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    if let Some(name) = skill_name {
                        let lookup = self.tools.get_manager().map(|m| m.skills.clone());
                        if let Some(skills) = lookup {
                            if let Some(skill) = skills.reload_one(&name).await {
                                let _ = self
                                    .activate_skill(
                                        skill,
                                        crate::engine::ActivationMode::ToolInvocation,
                                    )
                                    .await;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Tool failed: {} err={:#}", canonical_tool, e);
                let rendered = tool_error_text(&canonical_tool, &e);
                if tools::tool_cacheable(&canonical_tool) {
                    tool_cache.insert(
                        sig,
                        CachedToolObs {
                            model: rendered.clone(),
                        },
                    );
                } else {
                    tool_cache.clear();
                }
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
                        .send_event(
                            crate::engine::agent::AgentEvent::ContentBlockUpdate {
                                agent_id: agent_id.clone(),
                                block_id: block_id.clone(),
                                status: Some("failed".to_string()),
                                summary: Some(err_summary),
                                is_error: Some(true),
                                parent_id: self.parent_agent_id.clone(),
                                extra: None,
                                run_id: self.run_id.clone(),
                                parent_run_id: self.parent_run_id.clone(),
                            },
                            self.session_id.clone(),
                        )
                        .await;
                    manager
                        .send_event(
                            crate::engine::agent::AgentEvent::AgentStatus {
                                agent_id,
                                status: "thinking".to_string(),
                                detail: Some(format!("Thinking ({})", self.model_id)),
                                parent_id: self.parent_agent_id.clone(),
                                run_id: self.run_id.clone(),
                                parent_run_id: self.parent_run_id.clone(),
                            },
                            self.session_id.clone(),
                        )
                        .await;
                }
                let err_content = self.prompt_store.render_or_fallback(
                    crate::prompts::keys::TOOL_EXEC_FAILED,
                    &[("tool", &canonical_tool), ("error", &e.to_string())],
                );
                let err_msg = self.tool_result_msg_for(err_content, &tool_call_id, &canonical_tool);
                self.push_tracked_message(messages, err_msg);
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
                        content
                            .find(new_s)
                            .map(|pos| content[..pos].lines().count().max(1))
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
                    format!(
                        "{}…\n(truncated, {} total chars)",
                        &content[..10_000],
                        content.len()
                    )
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
                self.tools.builtins.parent_interactive = self.session_permissions.interactive;
                let tools_clone = self.tools.clone();
                // Each tool declares its own ceiling (`Tool::max_duration`);
                // `None` means open-ended by nature — AskUser waits on a
                // person, Task on a subagent. The progress tick below is
                // already a 150 ms heartbeat, so the deadline needs no timer
                // of its own.
                let ceiling = tools::tool_max_duration(&exec.canonical_tool);
                let started = std::time::Instant::now();
                let mut exec_fut = std::pin::pin!(tools_clone.execute(call));
                let result = loop {
                    tokio::select! {
                        res = &mut exec_fut => break res,
                        _ = tokio::time::sleep(std::time::Duration::from_millis(150)) => {
                            self.drain_tool_progress(progress_rx).await;
                            // A user cancel aborts the in-flight call the same
                            // way a timeout does: drop the future, hand back an
                            // error. The post-loop cancel check ends the run;
                            // post_execute settles the tool's UI block first.
                            if self.is_cancelled().await {
                                warn!(
                                    "[{}] Tool {} interrupted by user cancel",
                                    self.run_id.as_deref().unwrap_or("root"),
                                    exec.canonical_tool
                                );
                                break Err(anyhow::anyhow!("run cancelled"));
                            }
                            let Some(limit) = ceiling else { continue };
                            if started.elapsed() < limit {
                                continue;
                            }
                            // Abandon the call and hand the model an error it
                            // can act on. Dropping the future frees the RUN;
                            // a thread already inside a blocking syscall keeps
                            // going until the kernel returns, which we cannot
                            // help — but it no longer owns the turn.
                            warn!(
                                "[{}] Tool {} exceeded {}s — abandoned",
                                self.run_id.as_deref().unwrap_or("root"),
                                exec.canonical_tool,
                                limit.as_secs()
                            );
                            break Err(anyhow::anyhow!(
                                "timeout: {} did not finish within {}s and was abandoned. \
                                 Narrow the scope (a specific directory or pattern rather than \
                                 a whole tree) or try another approach.",
                                exec.canonical_tool,
                                limit.as_secs()
                            ));
                        }
                    }
                };
                self.drain_tool_progress(progress_rx).await;

                // Settle the tool's UI block (done/failed status line) BEFORE
                // honoring a cancel — the old early return here left the
                // chip spinning forever after a stop.
                let control = self
                    .post_execute_tool(
                        exec,
                        result,
                        messages,
                        tool_cache,
                        empty_search_streak,
                        session_id,
                    )
                    .await;

                // Check cancellation after tool execution before feeding
                // the result into the next step (spec: agentic-loop.md).
                if self.is_cancelled().await {
                    return LoopControl::Return(AgentOutcome::None);
                }

                control
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
            let _ = self.persist_assistant_message(&message, session_id).await;
            self.active_skill = None;
            return Some(LoopControl::Return(AgentOutcome::None));
        }

        messages.push(
            self.tool_result_msg(
                self.prompt_store
                    .render_or_fallback(crate::prompts::NUDGE_REPETITION, &[]),
            ),
        );
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

/// What the model reads when a tool fails. `{:#}` keeps the cause chain: `{}`
/// showed only the outer context ("MCP tools/call memory_add") and hid the
/// timeout underneath, so a dream read a slow save as a refused one.
/// What one call to a tool is, as every pre-execution gate sees it.
struct ToolTurn<'a> {
    /// The name as the model wrote it.
    tool: &'a str,
    canonical: &'a str,
    args: &'a JsonValue,
    session_id: Option<&'a str>,
    tool_call_id: &'a Option<String>,
}

/// A Bash call's command, under either of its argument names.
fn bash_command_arg(args: &JsonValue) -> Option<String> {
    args.get("cmd")
        .or_else(|| args.get("command"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn tool_error_text(tool: &str, err: &anyhow::Error) -> String {
    format!("tool_error: tool={tool} error={err:#}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn a_tool_error_keeps_its_cause() {
        let err = Err::<(), _>(anyhow::anyhow!("MCP error -32603: add timed out after 25s"))
            .context("MCP tools/call memory_add")
            .unwrap_err();
        assert_eq!(
            tool_error_text("mcp__memory__memory_add", &err),
            "tool_error: tool=mcp__memory__memory_add error=MCP tools/call memory_add: MCP error -32603: add timed out after 25s"
        );
    }
}
