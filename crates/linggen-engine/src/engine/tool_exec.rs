use super::types::*;
use crate::engine::permission;
use crate::engine::render::{
    normalize_tool_path_arg, render_tool_result, render_tool_result_public,
    sanitize_tool_args_for_display, tool_call_signature,
};
use crate::engine::tool_render::tool_call_status;
use crate::engine::tools::{self, ToolCall};
use crate::message::ChatMessage;
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Clone, serde::Serialize)]
pub struct DeniedAction {
    pub tool: String,
    pub summary: String,
    pub mode: String,
    pub reason: String,
}

#[derive(Default)]
pub struct DenialStore(std::collections::HashMap<String, Vec<DeniedAction>>);
impl DenialStore {
    fn record(&mut self, session: &str, action: DeniedAction) {
        if !self.0.contains_key(session) && self.0.len() >= 100 {
            if let Some(key) = self.0.keys().next().cloned() { self.0.remove(&key); }
        }
        let rows = self.0.entry(session.into()).or_default();
        if rows.len() == 50 { rows.remove(0); }
        rows.push(action);
    }
    pub fn clear(&mut self, session: &str) { self.0.remove(session); }
    pub fn get(&self, session: &str) -> Vec<DeniedAction> {
        self.0.get(session).cloned().unwrap_or_default()
    }
}
pub static DENIALS: std::sync::LazyLock<std::sync::Mutex<DenialStore>> =
    std::sync::LazyLock::new(Default::default);

#[cfg(test)]
mod denial_store_tests {
    use super::*;
    #[test]
    fn denial_store_record_cap_clear() {
        let mut store = DenialStore::default();
        for i in 0..55 { store.record("s", DeniedAction { tool: i.to_string(), summary: "x".into(), mode: "edit".into(), reason: "denied".into() }); }
        assert_eq!(store.get("s").len(), 50);
        assert_eq!(store.get("s")[0].tool, "5");
        store.clear("s");
        assert!(store.get("s").is_empty());
    }
}

/// The chat's line for a call refused inside a declined folder.
fn declined_notice(dir: &str, language: &str) -> String {
    if language.starts_with("zh") {
        format!("本轮已拒绝访问 {dir}")
    } else {
        format!("Access to {dir} declined for this turn")
    }
}

/// How long a person in the chat has to answer a permission question.
const PERSON_ASK_WAIT: std::time::Duration = std::time::Duration::from_secs(8 * 3600);
/// How long a non-interactive session waits for its caller's decision
/// before the action counts as denied.
pub(crate) const CALLER_ASK_WAIT: std::time::Duration = std::time::Duration::from_secs(600);

/// Raise one question on the bridge and wait up to `wait` for its answer:
/// `None` on timeout, `Some(None)` when the question was dropped unanswered.
pub async fn ask_on_bridge(
    bridge: &tools::AskUserBridge,
    (agent_id, run_id): (String, Option<String>),
    question: tools::AskUserQuestion,
    caller_request: Option<tools::CallerPermissionRequest>,
    wait: std::time::Duration,
) -> Option<Option<Vec<tools::AskUserAnswer>>> {
    let question_id = uuid::Uuid::new_v4().to_string();
    let questions = vec![question];
    let (tx, rx) = tokio::sync::oneshot::channel();
    // Register before publishing so an immediate answer cannot race it.
    bridge.pending.lock().await.insert(
        question_id.clone(),
        tools::PendingAskUser {
            agent_id: agent_id.clone(),
            questions: questions.clone(),
            sender: tx,
            session_id: bridge.session_id.clone(),
            run_id,
            caller_request,
        },
    );
    let _ = bridge
        .events_tx
        .send(crate::engine::events::ServerEvent::AskUser {
            agent_id,
            question_id: question_id.clone(),
            questions,
            session_id: bridge.session_id.clone(),
        });
    let response = tokio::time::timeout(wait, rx).await;
    if bridge.pending.lock().await.remove(&question_id).is_some() && response.is_err() {
        // Nobody answered: tell every client to drop the card.
        let _ = bridge
            .events_tx
            .send(crate::engine::events::ServerEvent::WidgetResolved {
                widget_id: question_id,
                session_id: bridge.session_id.clone(),
            });
    }
    Some(response.ok()?.ok())
}

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

    /// Ask for permission via the AskUser bridge and wait up to `wait`:
    /// the person in the chat, or — with `caller_request` — the session's
    /// caller (`agent_permit`). Returns `None` on timeout.
    pub async fn ask_permission_raw(
        &self,
        tool: &str,
        question: tools::AskUserQuestion,
        caller_request: Option<tools::CallerPermissionRequest>,
        wait: std::time::Duration,
    ) -> Option<permission::PermissionAction> {
        let bridge = Arc::clone(self.tools.ask_user_bridge()?);
        let agent_id = self.agent_id.clone().unwrap_or_default();

        info!("Permission: awaiting approval for '{}'", tool);
        // The offered grants are for this level only; parse against it.
        let target_mode = question.permission.as_ref().map(|meta| meta.mode);
        let offered: Vec<String> = question.options.iter().map(|opt| opt.label.clone()).collect();
        let response = ask_on_bridge(&bridge, (agent_id, self.run_id.clone()), question, caller_request, wait).await;

        match response {
            Some(Some(answers)) => {
                if let Some(target_mode) = target_mode {
                    return Some(permission::parse_exceeds_ceiling_answer(
                        &answers,
                        &target_mode,
                        &offered,
                    ));
                }
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
            Some(None) => {
                warn!("Permission: '{}' channel closed → denied", tool);
                Some(permission::PermissionAction::Deny)
            }
            None => {
                warn!("Permission: '{}' timed out", tool);
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

        let safe_args = self.record_tool_call(&t);

        // Compute tool call signature early for denied-check and later redundancy tracking.
        let sig = tool_call_signature(&canonical_tool, &args);

        if let Some(stop) = self.gate_write_safety(&t, read_paths, messages).await {
            return stop;
        }
        if let Some(stop) = self.gate_restrictions(&t, messages).await {
            return stop;
        }
        let mut approved_reads = Vec::new();
        if let Some(stop) = self.gate_permission(&t, messages, &mut approved_reads).await {
            return stop;
        }
        if let Some(stop) = self.gate_place_rules(&t, allowed_tools, messages).await {
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
                approved_reads,
                tool_done_status,
                tool_failed_status,
                block_id,
                tool_call_id,
            },
        )
    }

    /// Refuse the model's call with `msg` and go on — saved like any other
    /// call: its call row and the refusal as its result, shown live as a
    /// failed tool block, so the thread and the history hold what the model
    /// read (a withheld or denied call left no row until 2026-10-05).
    async fn refuse_saved(
        &mut self,
        t: &ToolTurn<'_>,
        messages: &mut Vec<ChatMessage>,
        msg: String,
    ) -> Option<PreExecOutcome> {
        let notice = msg.clone();
        self.refuse_saved_noted(t, messages, msg, &notice).await
    }

    /// Refuse a call inside a folder the person declined this run. `auto`:
    /// refused without a prompt, so the chat gets one short line saying so.
    async fn refuse_declined(
        &mut self,
        t: &ToolTurn<'_>,
        messages: &mut Vec<ChatMessage>,
        scope: &Path,
        user_msg: Option<&str>,
        gate_reason: Option<&str>,
        auto: bool,
    ) -> Option<PreExecOutcome> {
        let dir = permission::display_path(scope);
        let summary = self.permission_summary(t);
        let mut msg = self.prompt_store.render_or_fallback(
            crate::prompts::keys::PERMISSION_DECLINED,
            &[("tool", t.canonical), ("summary", &summary), ("dir", &dir)],
        );
        msg = with_gate_reason(msg, gate_reason);
        if let Some(words) = user_msg.filter(|w| !w.trim().is_empty()) {
            msg.push_str(&format!("\nUser says: {words}"));
        }
        if !auto {
            return self.refuse_saved(t, messages, msg).await;
        }
        info!("Permission: {} inside declined {} → refused", t.canonical, dir);
        let notice = declined_notice(&dir, &crate::language::effective());
        self.refuse_saved_noted(t, messages, msg, &notice).await
    }

    /// [`Self::refuse_saved`] with its own line on the live tool block.
    async fn refuse_saved_noted(
        &mut self,
        t: &ToolTurn<'_>,
        messages: &mut Vec<ChatMessage>,
        msg: String,
        notice: &str,
    ) -> Option<PreExecOutcome> {
        self.run_faults.note_refused(t.canonical);
        let safe_args = sanitize_tool_args_for_display(t.canonical, t.args);
        let block_id = self.announce_tool_start(t, &safe_args).await;
        let _ = self
            .persist_observation(t.canonical, &msg, true, t.session_id)
            .await;
        self.announce_tool_refused(&block_id, notice).await;
        self.refuse(t, messages, msg)
    }

    /// The live tool block of a refused call, marked failed.
    async fn announce_tool_refused(&self, block_id: &str, msg: &str) {
        let agent_id = self.agent_id.clone().unwrap_or_else(|| "unknown".into());
        self.emit(crate::engine::agent::AgentEvent::ContentBlockUpdate {
            agent_id,
            block_id: block_id.to_string(),
            status: Some(tool_call_status(true).to_string()),
            summary: Some(msg.to_string()),
            is_error: Some(true),
            parent_id: self.parent_agent_id.clone(),
            extra: None,
            run_id: self.run_id.clone(),
            parent_run_id: self.parent_run_id.clone(),
        })
        .await;
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
    /// Listed-only tools — the pet's (Express drives the on-screen avatar) and
    /// an agent's own notes — only run for an agent that lists them
    /// EXPLICITLY (an unrestricted agent whose own spec lists its notes
    /// included). The model-facing schema already
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
        if tools::is_listed_only(t.canonical)
            && !matches!(allowed_tools, Some(set) if set.contains(t.canonical))
            && !self.holds_own_note_tool(allowed_tools.as_ref(), t.canonical)
        {
            let rendered = listed_only_refusal(t.canonical);
            self.upsert_observation("error", t.canonical, rendered.clone());
            return self.refuse_saved(t, messages, rendered).await;
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
        self.upsert_observation("error", t.canonical, rendered);
        let msg = self.prompt_store.render_or_fallback(
            crate::prompts::keys::TOOL_NOT_ALLOWED,
            &[("tool", t.tool), ("allowed_list", &allowed_list.join(", "))],
        );
        self.refuse_saved(t, messages, msg).await
    }

    /// Record the call in context and the log. Returns the args as shown
    /// to people.
    fn record_tool_call(&mut self, t: &ToolTurn<'_>) -> JsonValue {
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
        safe_args
    }

    /// Place-rules gate: a file tool's first call under a sub-folder's or a
    /// path-scoped rule brings that rule into the system message, and a
    /// change written before the rule reached the model waits one round
    /// (`instruction_files::AgentEngine::meet_place_rules` decides).
    async fn gate_place_rules(
        &mut self,
        t: &ToolTurn<'_>,
        allowed_tools: &Option<HashSet<String>>,
        messages: &mut Vec<ChatMessage>,
    ) -> Option<PreExecOutcome> {
        let meeting = self.meet_place_rules(t.canonical, t.args);
        if meeting.added {
            self.rebuild_system_message(messages, allowed_tools.as_ref());
        }
        let msg = meeting.wait?;
        self.refuse_saved_noted(t, messages, msg, "Held once: this place's rules were just loaded.").await
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
                let msg = self.prompt_store.render_or_fallback(
                    crate::prompts::keys::WRITE_SAFETY_BLOCKED,
                    &[("rendered", &rendered)],
                );
                self.refuse_saved(t, messages, msg).await
            }
            crate::config::WriteSafetyMode::Warn => {
                let rendered = format!(
                    "tool_warning: tool={} warning=writing_existing_file_without_prior_read path='{}'",
                    action, path
                );
                self.upsert_observation("warning", action, rendered.clone());
                let _ = self
                    .persist_observation(action, &rendered, false, t.session_id)
                    .await;
                None
            }
            crate::config::WriteSafetyMode::Off => None,
        }
    }

    /// Restriction gates (defense-in-depth), in order: the config-level tool
    /// set, tools withheld this turn, and a consumer's skill list.
    async fn gate_restrictions(
        &mut self,
        t: &ToolTurn<'_>,
        messages: &mut Vec<ChatMessage>,
    ) -> Option<PreExecOutcome> {
        let msg = self
            .config_restriction(t)
            .or_else(|| self.withheld_restriction(t))
            .or_else(|| self.consumer_skill_restriction(t))?;
        self.refuse_saved(t, messages, msg).await
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

    pub(crate) fn read_origin(&self) -> permission::ReadOrigin {
        if self.active_mission.is_some() || self.tools.builtins.private_app.is_some()
            || self.tools.builtins.turn_place.as_ref().is_some_and(|place| place.skill.is_some())
        {
            permission::ReadStart::NoStanding.standing()
        } else {
            self.session_spec.read_origin
        }
    }

    /// What a `Task` child of this turn stands as for reads — a Task tool
    /// call and a batch of delegations alike.
    pub(crate) fn child_read_origin(&self) -> permission::ReadOrigin {
        permission::ReadStart::TaskChild(self.read_origin()).standing()
    }

    /// The path/tier permission gate (permission-spec.md), asking the user
    /// when the call exceeds the session's grant.
    async fn gate_permission(
        &mut self,
        t: &ToolTurn<'_>,
        messages: &mut Vec<ChatMessage>,
        approved_reads: &mut Vec<PathBuf>,
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
        let file_path_arg = if t.canonical == "Read" {
            t.args.get("path").or_else(|| t.args.get("file")).or_else(|| t.args.get("filepath"))
                .and_then(JsonValue::as_str).map(str::to_owned)
        } else if matches!(t.canonical, "Write" | "Edit") {
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

        let mut last_grant: Option<(String, permission::PermissionMode)> = None;
        loop {
            // A grant made in another run of this chat (a subagent's prompt,
            // the main chat's, the badge) holds here too.
            self.sync_session_grants();
            let coding = self.is_coding_session();
            let origin = self.read_origin();
            let cwd = self.tools.builtins.cwd();
            let url = t.args.get("url").and_then(JsonValue::as_str).filter(|_| t.canonical == "WebFetch");
            let check = |path: Option<&str>| {
                permission::check_permission_under_rules(
                    t.canonical, bash_command.as_deref(), path, &cwd,
                    &self.session_permissions, skill_tier_override, coding,
                    self.session_spec.ceiling, origin, Some(&self.declined_scopes), approved_reads,
                    self.claude_rules.for_call(url),
                )
            };
            let check_result = if t.canonical == "Glob" {
                let mut ask = None;
                for root in tools::glob_roots(t.args, &cwd) {
                    match check(root.to_str()) {
                        permission::PermissionCheckResult::Allowed => {},
                        blocked @ permission::PermissionCheckResult::Blocked(_) => {
                            ask = Some(blocked);
                            break;
                        }
                        request @ permission::PermissionCheckResult::NeedsPrompt(_) => {
                            ask.get_or_insert(request);
                        }
                    }
                }
                ask.unwrap_or(permission::PermissionCheckResult::Allowed)
            } else {
                check(file_path_arg.as_deref())
            };

            let prompt_kind = match check_result {
                permission::PermissionCheckResult::Allowed => return None,
                permission::PermissionCheckResult::Blocked(reason) => {
                    info!("Permission blocked: {} — {}", t.canonical, reason);
                    self.record_denial(t, "blocked", &reason);
                    // Under a turn's ceiling the model is told why: no
                    // person declined it, the device's level did.
                    let msg = match self.session_spec.ceiling {
                        Some(_) => format!("Tool {}: {reason}", t.canonical),
                        None if t.canonical == "Bash" && coding => reason,
                        // A deny rule of the person's Claude Code settings
                        // names itself and its file, whatever the tool.
                        None if permission::is_claude_refusal(&reason) => reason,
                        None => self.permission_denied_msg(t),
                    };
                    return self.refuse_saved(t, messages, msg).await;
                }
                permission::PermissionCheckResult::NeedsPrompt(prompt_kind) => prompt_kind,
            };

            // Non-interactive sessions (mission, proxy consumer) cannot prompt —
            // return permission-needed immediately. One whose caller answers
            // (an MCP delegate) asks the caller instead and waits.
            let asks_caller = !self.session_permissions.interactive;
            if asks_caller && !self.session_permissions.ask_caller {
                let permission::PromptKind::ExceedsCeiling { target_mode, reason, .. } = &prompt_kind;
                self.record_denial(t, &target_mode.to_string(), reason.as_deref().unwrap_or("Action exceeds session grants (non-interactive)"));
                let msg = with_gate_reason(self.permission_denied_msg(t), reason.as_deref());
                return self.refuse_saved(t, messages, msg).await;
            }

            let permission::PromptKind::ExceedsCeiling {
                target_mode,
                path,
                directories,
                tool_summary,
                outward,
                reason,
                scoped_read,
                by_rule,
            } = prompt_kind;
            let reason = reason.as_deref();
            let tool_summary = with_gate_reason(tool_summary, reason);
            let denied = |engine: &Self| with_gate_reason(engine.permission_denied_msg(t), reason);
            if last_grant
                .as_ref()
                .is_some_and(|(previous, _)| previous == &path)
            {
                let msg = denied(self);
                return self.refuse_saved(t, messages, msg).await;
            }
            // Declined earlier in this run: refused without asking again.
            if let Some(scope) = self.declined_scopes.covering(&path).map(Path::to_path_buf) {
                self.record_denial(t, &target_mode.to_string(), "Declined earlier in this run");
                return self.refuse_declined(t, messages, &scope, None, reason, true).await;
            }
            let (question, caller_request, wait) = if outward || by_rule {
                // Its effect leaves the machine, or the person's Claude Code
                // settings ask every time: this one action, once or not.
                let question = if outward {
                    permission::build_outward_question(&tool_summary, &target_mode, &path)
                } else {
                    permission::build_rule_question(&tool_summary, &target_mode, &path)
                };
                let request = asks_caller.then(|| {
                    let mut request = self.caller_request(t, target_mode, &path, &[], reason);
                    request.outward = true;
                    request
                });
                let wait = if asks_caller { CALLER_ASK_WAIT } else { PERSON_ASK_WAIT };
                (question, request, wait)
            } else if scoped_read && !asks_caller {
                let question = permission::build_caller_question(&tool_summary, &target_mode, &path, &directories);
                (question, None, PERSON_ASK_WAIT)
            } else if asks_caller {
                let request = self.caller_request(t, target_mode, &path, &directories, reason);
                let question = permission::build_caller_question(
                    &tool_summary,
                    &target_mode,
                    &path,
                    &directories,
                );
                (question, Some(request), CALLER_ASK_WAIT)
            } else {
                let question = permission::build_exceeds_ceiling_question(
                    &tool_summary,
                    &target_mode,
                    &path,
                    &directories,
                );
                (question, None, PERSON_ASK_WAIT)
            };
            let answer = self
                .ask_permission_raw(t.canonical, question, caller_request, wait)
                .await;
            match answer {
                Some(permission::PermissionAction::AllowOnce) if scoped_read => {
                    approved_reads.push(PathBuf::from(&path));
                    continue;
                }
                Some(permission::PermissionAction::AllowOnce) => return None,
                Some(permission::PermissionAction::AllowDirectory { all, mode, path: chosen }) => {
                    // Do not use a session grant when persisting it failed.
                    let mut updated = self.session_permissions.clone();
                    let grant_path = if all { "*" } else { chosen.as_str() };
                    let edit = |perms: &mut permission::SessionPermissions| perms.set_path_mode(grant_path, mode);
                    if let Some(ref sdir) = self.session_dir {
                        match updated.edit_saved(sdir, edit) {
                            Ok(saved) => updated.path_modes = saved.path_modes,
                            Err(error) => {
                                warn!("Permission grant could not be saved: {}", error);
                                let msg = format!(
                                    "Permission denied: unable to save directory grant: {}",
                                    error
                                );
                                return self.refuse_saved(t, messages, msg).await;
                            }
                        }
                    } else {
                        edit(&mut updated);
                    }
                    self.session_permissions = updated;
                    last_grant = Some((path.clone(), mode));
                    // Notify UI so the mode badge updates.
                    if self.tools.get_manager().is_some() {
                        self.emit(crate::engine::agent::AgentEvent::StateUpdated)
                            .await;
                    }
                    if mode < target_mode {
                        let msg = denied(self);
                        return self.refuse_saved(t, messages, msg).await;
                    }
                    continue;
                }
                Some(permission::PermissionAction::AllowSession) => {
                    let msg = denied(self);
                    return self.refuse_saved(t, messages, msg).await;
                }
                Some(permission::PermissionAction::Deny) => {
                    if asks_caller {
                        self.record_denial(t, &target_mode.to_string(), "Denied by the caller");
                    }
                    let scope = self.declined_scopes.decline(&path);
                    return self.refuse_declined(t, messages, &scope, None, reason, false).await;
                }
                Some(permission::PermissionAction::DenyWithMessage(user_msg)) => {
                    let scope = self.declined_scopes.decline(&path);
                    return self
                        .refuse_declined(t, messages, &scope, Some(&user_msg), reason, false)
                        .await;
                }
                None if asks_caller => {
                    // The caller never answered: denied, and the run goes on.
                    let reason = format!(
                        "No answer from the caller within {} min (non-interactive)",
                        CALLER_ASK_WAIT.as_secs() / 60
                    );
                    self.record_denial(t, &target_mode.to_string(), &reason);
                    let msg = denied(self);
                    return self.refuse_saved(t, messages, msg).await;
                }
                None => {
                    let msg = self
                        .prompt_store
                        .render_or_fallback(crate::prompts::keys::PERMISSION_TIMEOUT, &[]);
                    let _ = self.persist_assistant_message(&msg, t.session_id).await;
                    return Some(PreExecOutcome::Blocked(LoopControl::Return(
                        AgentOutcome::None,
                    )));
                }
            }
        }
    }

    /// Keep a refused action where `agent_status` reports it.
    fn record_denial(&self, t: &ToolTurn<'_>, mode: &str, reason: &str) {
        let Some(session) = t.session_id else { return };
        DENIALS.lock().unwrap_or_else(|e| e.into_inner()).record(
            session,
            DeniedAction {
                tool: t.canonical.to_string(),
                summary: self.permission_summary(t),
                mode: mode.to_string(),
                reason: reason.to_string(),
            },
        );
    }

    /// What the caller sees of a permission request: the action, the level
    /// it needs, its target, the narrowest grantable folder, and the
    /// agent's own words for it when the call carries them.
    fn caller_request(
        &self,
        t: &ToolTurn<'_>,
        level: permission::PermissionMode,
        path: &str,
        directories: &[String],
        gate_reason: Option<&str>,
    ) -> tools::CallerPermissionRequest {
        let reason = ["description", "reason"]
            .iter()
            .find_map(|key| t.args.get(*key).and_then(|v| v.as_str()))
            .filter(|text| !text.trim().is_empty())
            .map(str::to_string);
        let reason = gate_reason
            .map(|gate| with_gate_reason(gate.to_string(), reason.as_deref()))
            .or(reason);
        let summary = self.permission_summary(t);
        let command = ["cmd", "command"]
            .iter()
            .find_map(|key| t.args.get(*key).and_then(|v| v.as_str()))
            .filter(|cmd| t.canonical == "Bash" && **cmd != summary)
            .map(str::to_string);
        tools::CallerPermissionRequest {
            tool: t.canonical.to_string(),
            summary,
            command,
            level,
            path: path.to_string(),
            dir: directories.last().cloned(),
            reason,
            outward: false,
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
        if self.tools.get_manager().is_none() {
            return block_id;
        }
        let from = self
            .agent_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let target = self.outbound_target();
        // Emit structured ContentBlockStart for the Web UI.
        let compact_args = serde_json::to_string(safe_args).unwrap_or_else(|_| "{}".to_string());
        self.emit(crate::engine::agent::AgentEvent::ContentBlockStart {
            agent_id: from.clone(),
            block_id: block_id.clone(),
            block_type: "tool_use".to_string(),
            tool: Some(t.canonical.to_string()),
            args: Some(compact_args),
            parent_id: self.parent_agent_id.clone(),
            run_id: self.run_id.clone(),
            parent_run_id: self.parent_run_id.clone(),
        })
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
            self.save_row(
                t.session_id,
                crate::state_fs::sessions::ChatMsg {
                    run_id: None,
                    via: None,
                    via_note: None,
                    agent_id: from.clone(),
                    from_id: from,
                    to_id: target,
                    content: tool_msg,
                    timestamp: crate::util::now_ts_secs(),
                    is_observation: true,
                    client_id: None,
                    failed: false,
                    images: Vec::new(),
                    place: None,
                },
            )
            .await;
        }
        block_id
    }

    /// Number the sources and pictures a result carries in the session's
    /// table (`answer_refs`), so the model can show them by number.
    fn number_answer_refs(&self, result: &mut tools::ToolResult, session_id: Option<&str>) {
        let (Some(manager), Some(sid)) = (self.tools.get_manager(), session_id) else {
            return;
        };
        crate::engine::answer_refs::register::number(&manager.global_sessions, sid, result);
    }

    /// Pictures go to the model only when it can see them: a picture file
    /// read keeps its bytes, an `ImageSearch` gets its contact sheet. For a
    /// model that cannot see, the bytes are dropped and the facts stay.
    async fn settle_pictures(&self, result: &mut tools::ToolResult) {
        let wants_eyes = match result {
            tools::ToolResult::PictureFile { base64, .. } => !base64.is_empty(),
            tools::ToolResult::PictureSearchResults { pictures, .. } => !pictures.is_empty(),
            _ => false,
        };
        if !wants_eyes {
            return;
        }
        let sees = self
            .model_manager
            .has_vision(&self.model_id)
            .await
            .unwrap_or(false);
        match result {
            tools::ToolResult::PictureFile { base64, seen, .. } => {
                *seen = sees;
                if !sees {
                    base64.clear();
                }
            }
            tools::ToolResult::PictureSearchResults {
                pictures, sheet, ..
            } if sees => attach_sheet(pictures, sheet).await,
            _ => {}
        }
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
        read_paths: &mut HashSet<String>,
        empty_search_streak: &mut usize,
        session_id: Option<&str>,
    ) -> LoopControl {
        let ReadyExec {
            canonical_tool,
            sig,
            original_args,
            approved_reads: _,
            tool_done_status,
            tool_failed_status,
            block_id,
            tool_call_id,
        } = exec;

        let failed = self.note_run_fault(&canonical_tool, &result);
        match result {
            Ok(mut result) => {
                // A Read that succeeded, and only that, counts for the
                // write-safety gate's "read before you write".
                if canonical_tool == "Read" {
                    let cwd = self.tools.builtins.cwd();
                    if let Some(path) = normalize_tool_path_arg(&cwd, &original_args) {
                        read_paths.insert(path);
                    }
                }
                self.note_closing_ask(&canonical_tool, &result);
                self.number_answer_refs(&mut result, session_id);
                self.settle_pictures(&mut result).await;
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
                    .persist_observation(&canonical_tool, &rendered_public, failed, session_id)
                    .await;
                if self.tools.get_manager().is_some() {
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
                    self.emit(crate::engine::agent::AgentEvent::ContentBlockUpdate {
                        agent_id: agent_id.clone(),
                        block_id: block_id.clone(),
                        status: Some(tool_call_status(failed).to_string()),
                        summary: Some(if failed { tool_failed_status.clone() } else { tool_done_status.clone() }),
                        is_error: Some(failed),
                        parent_id: self.parent_agent_id.clone(),
                        extra,
                        run_id: self.run_id.clone(),
                        parent_run_id: self.parent_run_id.clone(),
                    })
                    .await;
                    self.emit(crate::engine::agent::AgentEvent::StateUpdated)
                        .await;
                    self.emit(crate::engine::agent::AgentEvent::AgentStatus {
                        agent_id,
                        status: "thinking".to_string(),
                        detail: Some(format!("Thinking ({})", self.model_id)),
                        parent_id: self.parent_agent_id.clone(),
                        run_id: self.run_id.clone(),
                        parent_run_id: self.parent_run_id.clone(),
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
                    let _ = self.persist_assistant_message(&msg, session_id).await;
                }

                let obs_content = self.observation_text("tool", &canonical_tool, &rendered_model);
                let obs_msg = self.tool_result_msg_for(obs_content, &tool_call_id, &canonical_tool);

                self.push_tracked_message(messages, obs_msg);

                // A picture the result shows the model (a screenshot, a
                // picture file read by a model that sees) rides in its own
                // message, this turn only: `messages` is the loop's copy, so
                // a later turn reads the result's text in its place.
                if let Some((note, base64)) = result.attachment() {
                    let img_msg =
                        ChatMessage::new("user", note).with_images(vec![base64.to_string()]);
                    self.push_tracked_message(messages, img_msg);
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
                    .persist_observation(&canonical_tool, &rendered, true, session_id)
                    .await;
                if self.tools.get_manager().is_some() {
                    let agent_id = self
                        .agent_id
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string());
                    // Emit structured ContentBlockUpdate (failed) for the Web UI.
                    let err_summary = format!("{}: {}", tool_failed_status, e);
                    self.emit(crate::engine::agent::AgentEvent::ContentBlockUpdate {
                        agent_id: agent_id.clone(),
                        block_id: block_id.clone(),
                        status: Some(tool_call_status(true).to_string()),
                        summary: Some(err_summary),
                        is_error: Some(true),
                        parent_id: self.parent_agent_id.clone(),
                        extra: None,
                        run_id: self.run_id.clone(),
                        parent_run_id: self.parent_run_id.clone(),
                    })
                    .await;
                    self.emit(crate::engine::agent::AgentEvent::AgentStatus {
                        agent_id,
                        status: "thinking".to_string(),
                        detail: Some(format!("Thinking ({})", self.model_id)),
                        parent_id: self.parent_agent_id.clone(),
                        run_id: self.run_id.clone(),
                        parent_run_id: self.parent_run_id.clone(),
                    })
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

    /// The call's one outcome: true when it failed — an error, or a declared
    /// tool's non-zero exit. Its card, its saved row and the run's fault
    /// count all read this. A failure is kept for whoever records the run
    /// (`run_faults` says what counts; a cancel is not kept). The model has
    /// already been handed the result either way.
    fn note_run_fault(&mut self, tool: &str, result: &anyhow::Result<tools::ToolResult>) -> bool {
        match result {
            Err(e) => {
                if !is_cancel_error(e) {
                    self.run_faults.note_error(tool, &format!("{e:#}"));
                }
                true
            }
            Ok(tools::ToolResult::CommandOutput {
                exit_code, stderr, ..
            }) if self.tools.has_skill_tool(tool) && *exit_code != Some(0) => {
                self.run_faults.note_exit(tool, *exit_code, stderr);
                true
            }
            _ => false,
        }
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

                let preview = write_preview(content);
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

    /// Carry the gate's read decision into the executor for each path it visits.
    pub(crate) fn tools_for_call(&self, exec: &ReadyExec) -> crate::engine::tool_registry::ToolRegistry {
        let mut tools = self.tools.clone();
        if matches!(exec.canonical_tool.as_str(), "Read" | "Glob" | "Grep") {
            let perms = self.session_permissions.clone();
            let ceiling = self.session_spec.ceiling;
            let origin = self.read_origin();
            let coding = self.is_coding_session();
            let declined = self.declined_scopes.clone();
            let cwd = self.tools.builtins.cwd();
            let approved = exec.approved_reads.clone();
            let claude = self.claude_rules.clone();
            tools.builtins.read_filter = Some(Arc::new(move |path| {
                path.to_str().is_some_and(|path| matches!(permission::check_permission_under_rules(
                    "Read", None, Some(path), &cwd, &perms, None, coding, ceiling, origin, Some(&declined), &approved,
                    claude.for_call(None),
                ), permission::PermissionCheckResult::Allowed))
            }));
        }
        tools
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
                self.tools.builtins.parent_allowed_tools = allowed_tools.clone();
                self.tools.builtins.parent_path_modes = self.session_permissions.path_modes.clone();
                self.tools.builtins.parent_interactive = self.session_permissions.interactive;
                self.tools.builtins.parent_read_origin = self.child_read_origin();
                let tools_clone = self.tools_for_call(&exec);
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
                        read_paths,
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
            self.active_skill.clear();
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
/// The error a user cancel hands back to an in-flight call — the run's own
/// status says it, so it is no tool's failure.
fn with_gate_reason(mut text: String, reason: Option<&str>) -> String {
    if let Some(reason) = reason {
        text.push('\n');
        text.push_str(reason);
    }
    text
}

fn is_cancel_error(e: &anyhow::Error) -> bool {
    super::is_stopped(e)
}

fn bash_command_arg(args: &JsonValue) -> Option<String> {
    args.get("cmd")
        .or_else(|| args.get("command"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// What the model reads when a tool fails. `{:#}` keeps the cause chain: `{}`
/// showed only the outer context ("MCP tools/call memory_add") and hid the
/// timeout underneath, so a dream read a slow save as a refused one.
fn tool_error_text(tool: &str, err: &anyhow::Error) -> String {
    format!("tool_error: tool={tool} error={err:#}")
}

/// The found pictures as one contact sheet; a picture that would not load
/// is left out of the result too (the answer could not show it either).
async fn attach_sheet(
    pictures: &mut Vec<crate::engine::image_search::FoundPicture>,
    sheet: &mut String,
) {
    let Some(built) = crate::engine::picture_sheet::build(pictures).await else {
        return;
    };
    let mut on_sheet = built.on_sheet.into_iter();
    pictures.retain(|_| on_sheet.next().unwrap_or(false));
    *sheet = built.base64;
}

/// Why a listed-only tool did not run for an agent that never listed it.
fn listed_only_refusal(tool: &str) -> String {
    if tools::is_pet_scoped(tool) {
        return format!(
            "tool_not_allowed: tool={tool} reason=pet_scoped: only the pet agent may drive the avatar; ask Yinyue via agent_chat instead"
        );
    }
    format!("tool_not_allowed: tool={tool} reason=listed_only: only an agent that lists this tool has it")
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

#[cfg(test)]
mod declined_gate_tests {
    use super::*;
    use crate::engine::permission::{PermissionMode, SessionPermissions};

    type Pending = Arc<tokio::sync::Mutex<HashMap<String, tools::PendingAskUser>>>;

    fn engine(root: &std::path::Path, pending: &Pending, ask_caller: bool) -> AgentEngine {
        let cfg = crate::engine::EngineConfig::from_app_config(
            &crate::config::Config::default(),
            root.into(),
            crate::engine::InterfaceMode::Web,
        );
        let models = Arc::new(crate::provider::models::ModelManager::new_with_credentials(
            vec![],
            &crate::credentials::Credentials::default(),
        ));
        let mut engine = AgentEngine::new(cfg, models, "m".into(), crate::engine::AgentRole::Lead).unwrap();
        let mut perms = SessionPermissions { interactive: !ask_caller, ask_caller, ..Default::default() };
        perms.set_path_mode(&root.to_string_lossy(), PermissionMode::Read);
        engine.session_permissions = perms;
        engine.tools.set_ask_user_bridge(Arc::new(tools::AskUserBridge {
            events_tx: crate::engine::event_bus::EventBus::new(16),
            pending: pending.clone(),
            session_id: None,
        }));
        engine
    }

    /// Answer every prompt with Deny; count how many were asked.
    fn deny_all(pending: &Pending) -> Arc<std::sync::atomic::AtomicUsize> {
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (pending, count) = (pending.clone(), asked.clone());
        tokio::spawn(async move {
            loop {
                let next = pending.lock().await.keys().next().cloned();
                if let Some(id) = next {
                    let entry = pending.lock().await.remove(&id).unwrap();
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let answer = tools::AskUserAnswer { question_index: 0, selected: vec!["Deny".into()], custom_text: None };
                    let _ = entry.sender.send(vec![answer]);
                }
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
        });
        asked
    }

    async fn gate(engine: &mut AgentEngine, tool: &str, args: JsonValue) -> String {
        let id = Some("c1".to_string());
        let t = ToolTurn { tool, canonical: tool, args: &args, session_id: None, tool_call_id: &id };
        let mut messages = Vec::new();
        assert!(engine.gate_permission(&t, &mut messages, &mut Vec::new()).await.is_some(), "{tool} was let through");
        messages.last().map(|m| m.content.clone()).unwrap_or_default()
    }

    #[tokio::test]
    async fn a_coding_bash_request_and_denial_carry_the_same_gate_reason() {
        let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let pending: Pending = Default::default();
        let seen = answer_first(&pending, "Deny");
        let mut engine = engine(work.path(), &pending, true);
        crate::engine::session_spec::coding_caller_fixture().apply(&mut engine);
        let denial = gate(&mut engine, "Bash", serde_json::json!({"cmd": "echo $FLAGS"})).await;
        let (_, request) = tokio::time::timeout(std::time::Duration::from_secs(5), seen)
            .await.unwrap().unwrap();
        let reason = request.unwrap().reason.unwrap();
        assert!(reason.contains("words are decided at run time"), "{reason}");
        assert!(denial.contains(&reason), "{denial}");
        let blocked = gate(&mut engine, "Bash", serde_json::json!({"cmd": "sudo ls"})).await;
        assert!(blocked.contains("never run here"), "{blocked}");
    }

    #[tokio::test]
    async fn a_denial_closes_the_repo_to_every_tool_for_the_run() {
        let home = dirs::home_dir().unwrap();
        let (work, repo, other) = (tempfile::tempdir_in(&home).unwrap(), tempfile::tempdir_in(&home).unwrap(), tempfile::tempdir_in(&home).unwrap());
        std::fs::create_dir_all(repo.path().join(".git")).unwrap();
        let pending: Pending = Default::default();
        let asked = deny_all(&pending);
        let mut engine = engine(work.path(), &pending, false);
        let repo_dir = repo.path().to_string_lossy().to_string();
        let first = gate(&mut engine, "Bash", serde_json::json!({"command": format!("cd {repo_dir} && git log -1")})).await;
        assert!(first.contains("declined access to") && first.contains("Do not try it again"), "{first}");
        let count = || asked.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(count(), 1);
        // Another tool on the same repo: refused, no prompt.
        let head = repo.path().join(".git/HEAD").to_string_lossy().to_string();
        let again = gate(&mut engine, "Read", serde_json::json!({"path": head})).await;
        assert!(again.contains("declined access to"), "{again}");
        assert_eq!(count(), 1, "the same repo is not asked twice in a run");
        // A path outside the declined repo still asks.
        let elsewhere = other.path().join("f.txt").to_string_lossy().to_string();
        gate(&mut engine, "Read", serde_json::json!({"path": elsewhere})).await;
        assert_eq!(count(), 2);
        // The next run asks again.
        engine.declined_scopes.clear();
        gate(&mut engine, "Read", serde_json::json!({"path": head})).await;
        assert_eq!(count(), 3);
    }

    #[tokio::test]
    async fn a_callers_deny_also_closes_the_repo_for_the_run() {
        let home = dirs::home_dir().unwrap();
        let (work, repo) = (tempfile::tempdir_in(&home).unwrap(), tempfile::tempdir_in(&home).unwrap());
        std::fs::create_dir_all(repo.path().join(".git")).unwrap();
        let pending: Pending = Default::default();
        let asked = deny_all(&pending);
        let mut engine = engine(work.path(), &pending, true);
        let file = repo.path().join("a.txt").to_string_lossy().to_string();
        gate(&mut engine, "Read", serde_json::json!({"path": file})).await;
        let head = repo.path().join(".git/HEAD").to_string_lossy().to_string();
        let again = gate(&mut engine, "Read", serde_json::json!({"path": head})).await;
        assert!(again.contains("declined access to"), "{again}");
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// Answer the first prompt with `answer`; what it offered and whether
    /// it went to the caller.
    fn answer_first(
        pending: &Pending,
        answer: &'static str,
    ) -> tokio::sync::oneshot::Receiver<(Vec<String>, Option<tools::CallerPermissionRequest>)> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let pending = pending.clone();
        tokio::spawn(async move {
            loop {
                let next = pending.lock().await.keys().next().cloned();
                if let Some(id) = next {
                    let entry = pending.lock().await.remove(&id).unwrap();
                    let labels = entry.questions[0].options.iter().map(|o| o.label.clone()).collect();
                    let _ = tx.send((labels, entry.caller_request.clone()));
                    let reply = tools::AskUserAnswer { question_index: 0, selected: vec![answer.into()], custom_text: None };
                    let _ = entry.sender.send(vec![reply]);
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
        });
        rx
    }

    fn admin_engine(root: &std::path::Path, pending: &Pending, interactive: bool, ask_caller: bool) -> AgentEngine {
        let mut engine = engine(root, pending, ask_caller);
        engine.session_permissions = SessionPermissions { interactive, ask_caller, ..Default::default() };
        engine.session_permissions.set_path_mode("*", PermissionMode::Admin);
        engine.session_permissions.set_path_mode(&root.to_string_lossy(), PermissionMode::Admin);
        engine
    }

    #[tokio::test]
    async fn read_policy_local_coding_caller_reads_without_a_grant() {
        let home = dirs::home_dir().unwrap();
        let work = tempfile::tempdir_in(&home).unwrap();
        let source = tempfile::tempdir_in(&home).unwrap();
        let path = source.path().join("notes.txt");
        std::fs::write(&path, "fixture").unwrap();
        let pending: Pending = Default::default();
        let asked = deny_all(&pending);
        let mut engine = engine(work.path(), &pending, true);
        crate::engine::session_spec::coding_caller_fixture().apply(&mut engine);
        let args = serde_json::json!({"path": path});
        let id = Some("c1".to_string());
        let t = ToolTurn { tool: "Read", canonical: "Read", args: &args, session_id: None, tool_call_id: &id };
        assert!(engine.gate_permission(&t, &mut Vec::new(), &mut Vec::new()).await.is_none());
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn read_policy_origin_matrix() {
        use crate::engine::permission::{PermissionCheckResult, ReadOrigin, ReadStart};
        let world = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let target = world.path().join("notes.txt");
        std::fs::write(&target, "fixture").unwrap();
        let command = format!("cat {}", target.display());
        let grants = SessionPermissions::default();
        for (label, origin, coding, ceiling, allowed) in [
            ("local coding chat", ReadOrigin::LocalOwner, true, None, true),
            ("local coding delegate", ReadOrigin::LocalDelegate, true, None, true),
            ("coding Task", ReadStart::TaskChild(ReadOrigin::LocalDelegate).standing(), true, None, true),
            ("ordinary chat", ReadOrigin::LocalOwner, false, None, false),
            ("handoff", ReadOrigin::Other, true, None, false),
            ("app", ReadOrigin::Other, false, None, false),
            ("private place", ReadOrigin::Other, true, None, false),
            ("daily", ReadOrigin::Other, false, None, false),
            ("companion", ReadOrigin::Other, false, None, false),
            ("mission", ReadOrigin::Other, false, None, false),
            ("room guest", ReadOrigin::Other, false, None, false),
            ("remote person", ReadOrigin::Other, true, None, false),
            ("paired admin", ReadOrigin::Other, true, Some(PermissionMode::Admin), false),
            ("remote Task", ReadStart::TaskChild(ReadOrigin::Other).standing(), true, None, false),
        ] {
            for tool in ["Read", "Glob", "Grep", "Bash"] {
                let result = permission::check_permission_with_read_approvals(
                    tool, (tool == "Bash").then_some(command.as_str()), target.to_str(),
                    world.path(), &grants, None, coding, ceiling, origin, None, &[],
                );
                assert_eq!(matches!(result, PermissionCheckResult::Allowed), allowed, "{label}: {tool}: {result:?}");
            }
        }
    }

    #[tokio::test]
    async fn read_policy_search_prunes_protected_entries_under_a_broad_grant() {
        let root = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        std::fs::write(root.path().join("public.txt"), "fixture-needle").unwrap();
        let hidden = root.path().join(".ssh");
        std::fs::create_dir(&hidden).unwrap();
        std::fs::write(hidden.join("private.txt"), "fixture-needle").unwrap();
        let pending: Pending = Default::default();
        let engine = engine(root.path(), &pending, false);
        let exec = ReadyExec {
            canonical_tool: "Glob".into(), sig: String::new(), original_args: JsonValue::Null,
            approved_reads: Vec::new(), tool_done_status: String::new(), tool_failed_status: String::new(),
            block_id: String::new(), tool_call_id: None,
        };
        let call = tools::ToolCall {
            tool: "Glob".into(),
            args: serde_json::json!({"globs": [format!("{}/**/*.txt", root.path().display())]}),
            block_id: None,
        };
        let result = crate::engine::render::render_tool_result(&engine.tools_for_call(&exec).execute(call).await.unwrap());
        assert!(result.contains("public.txt"), "{result}");
        assert!(!result.contains("private.txt"), "{result}");
    }

    #[tokio::test]
    async fn read_policy_recursive_glob_requires_grant_for_its_root() {
        let home = dirs::home_dir().unwrap();
        let work = tempfile::tempdir_in(&home).unwrap();
        let private = tempfile::tempdir_in(crate::paths::linggen_home()).unwrap();
        let pending: Pending = Default::default();
        let mut engine = admin_engine(work.path(), &pending, false, false);
        let args = serde_json::json!({"globs": [format!("{}/**/*", private.path().display())]});
        gate(&mut engine, "Glob", args).await;
    }

    #[tokio::test]
    async fn read_policy_protected_search_root_is_refused_unattended() {
        let private = tempfile::tempdir_in(crate::paths::linggen_home()).unwrap();
        let pending: Pending = Default::default();
        let mut engine = engine(private.path(), &pending, false);
        engine.session_permissions.interactive = false;
        engine.session_permissions.set_path_mode("*", PermissionMode::Read);
        gate(&mut engine, "Grep", serde_json::json!({"query": "fixture"})).await;
    }

    #[tokio::test]
    async fn read_policy_secrets_never_reach_the_approval_path() {
        use std::sync::atomic::Ordering::SeqCst;
        let home = crate::paths::linggen_home();
        let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let pending: Pending = Default::default();
        let asked = deny_all(&pending);
        let mut engine = admin_engine(work.path(), &pending, true, false);
        for name in ["account.toml", "credentials.json", "codex_auth.json", "paired-devices.json", "push/state.json"] {
            let file = home.join(name);
            engine.session_permissions.set_path_mode(file.to_str().unwrap(), PermissionMode::Admin);
            gate(&mut engine, "Read", serde_json::json!({"path": file})).await;
        }
        assert_eq!(asked.load(SeqCst), 0);
    }

    #[tokio::test]
    async fn read_policy_local_owner_can_approve_one_secret_read() {
        let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let pending: Pending = Default::default();
        let seen = answer_first(&pending, crate::engine::permission::ALLOW_ONCE);
        let mut engine = admin_engine(work.path(), &pending, true, false);
        engine.respec(|spec| spec.read_origin = permission::ReadOrigin::LocalOwner);
        let path = crate::paths::linggen_home().join("account.toml");
        let args = serde_json::json!({"path": path});
        let id = None;
        let t = ToolTurn { tool: "Read", canonical: "Read", args: &args, session_id: None, tool_call_id: &id };
        let mut approved = Vec::new();
        assert!(engine.gate_permission(&t, &mut Vec::new(), &mut approved).await.is_none());
        let (labels, request) = seen.await.unwrap();
        assert!(labels.iter().any(|label| label == permission::ALLOW_ONCE));
        assert!(request.is_none());
        assert_eq!(approved.len(), 1);
        let allowed = |approved: &[PathBuf]| permission::check_permission_with_read_approvals(
            "Read", None, path.to_str(), work.path(), &engine.session_permissions,
            None, false, None, permission::ReadOrigin::LocalOwner, None, approved,
        );
        assert!(matches!(allowed(&approved), permission::PermissionCheckResult::Allowed));
        assert!(matches!(allowed(&[]), permission::PermissionCheckResult::NeedsPrompt(_)));
    }

    #[tokio::test]
    async fn an_outward_command_is_refused_unattended_without_asking() {
        let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let pending: Pending = Default::default();
        let asked = deny_all(&pending);
        let mut engine = admin_engine(work.path(), &pending, false, false);
        gate(&mut engine, "Bash", serde_json::json!({"command": "cargo publish"})).await;
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 0, "nobody is asked");
        // The same session runs local work at admin.
        let args = serde_json::json!({"command": "cargo build"});
        let id = Some("c2".to_string());
        let t = ToolTurn { tool: "Bash", canonical: "Bash", args: &args, session_id: None, tool_call_id: &id };
        assert!(engine.gate_permission(&t, &mut Vec::new(), &mut Vec::new()).await.is_none());
    }

    #[tokio::test]
    async fn an_outward_command_asks_the_caller_once_with_no_session_grant() {
        let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let pending: Pending = Default::default();
        let seen = answer_first(&pending, crate::engine::permission::ALLOW_ONCE);
        let mut engine = admin_engine(work.path(), &pending, false, true);
        let args = serde_json::json!({"command": "git push --force origin feature"});
        let id = Some("c1".to_string());
        let t = ToolTurn { tool: "Bash", canonical: "Bash", args: &args, session_id: None, tool_call_id: &id };
        assert!(engine.gate_permission(&t, &mut Vec::new(), &mut Vec::new()).await.is_none(), "allowed once");
        let wait = std::time::Duration::from_secs(5);
        let (labels, request) = tokio::time::timeout(wait, seen).await.expect("asked").unwrap();
        assert_eq!(labels, [crate::engine::permission::ALLOW_ONCE, "Deny"]);
        let request = request.expect("the caller is asked");
        assert!(request.outward && request.dir.is_none(), "{request:?}");
    }

    #[tokio::test]
    async fn an_outward_command_asks_the_person_once_even_with_admin_everywhere() {
        let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let pending: Pending = Default::default();
        let seen = answer_first(&pending, "Deny");
        let mut engine = admin_engine(work.path(), &pending, true, false);
        gate(&mut engine, "Bash", serde_json::json!({"command": "ssh host uptime"})).await;
        let wait = std::time::Duration::from_secs(5);
        let (labels, request) = tokio::time::timeout(wait, seen).await.expect("asked").unwrap();
        assert_eq!(labels, [crate::engine::permission::ALLOW_ONCE, "Deny"]);
        assert!(request.is_none(), "the person, not a caller");
    }

    /// Strict write safety: only a Read that succeeded lets a Write change
    /// an existing file.
    #[tokio::test]
    async fn a_read_that_failed_is_not_a_read_before_a_write() {
        use std::os::unix::fs::PermissionsExt;
        let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let file = work.path().join("f.txt");
        std::fs::write(&file, "kept").unwrap();
        let pending: Pending = Default::default();
        let mut engine = admin_engine(work.path(), &pending, false, false);
        engine.cfg.write_safety_mode = crate::config::WriteSafetyMode::Strict;
        let mut read_paths = HashSet::new();
        async fn read(engine: &mut AgentEngine, read_paths: &mut HashSet<String>, path: &str) -> String {
            let (mut messages, mut cache) = (Vec::new(), HashMap::new());
            let (mut sig, mut streak, mut empty) = (String::new(), 0, 0);
            let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let args = serde_json::json!({ "path": path });
            engine
                .handle_tool_action(
                    "Read".into(), args, &None, &mut messages, &mut cache, read_paths,
                    &mut sig, &mut streak, &mut empty, &mut rx, None, Some("r1".into()),
                )
                .await;
            messages.last().map(|m| m.content.clone()).unwrap_or_default()
        }
        let path = file.to_string_lossy().to_string();
        let write = serde_json::json!({ "path": file.to_string_lossy(), "content": "new" });
        let id = Some("w1".to_string());
        let t = ToolTurn { tool: "Write", canonical: "Write", args: &write, session_id: None, tool_call_id: &id };

        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
        let failed = read(&mut engine, &mut read_paths, &path).await;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!failed.contains("kept"), "the Read failed: {failed}");
        assert!(engine.gate_write_safety(&t, &read_paths, &mut Vec::new()).await.is_some(), "a failed Read let the Write through");

        let read_ok = read(&mut engine, &mut read_paths, &path).await;
        assert!(read_ok.contains("kept"), "{read_ok}");
        assert!(engine.gate_write_safety(&t, &read_paths, &mut Vec::new()).await.is_none());
    }

    #[test]
    fn the_chat_line_follows_the_language() {
        assert_eq!(declined_notice("~/r", "zh-Hans"), "本轮已拒绝访问 ~/r");
        assert_eq!(declined_notice("~/r", "en"), "Access to ~/r declined for this turn");
    }

    /// A coding engine at admin everywhere whose project's Claude Code
    /// settings hold `permissions`; the person (`interactive`) or the
    /// caller answers.
    fn claude_rules_engine(work: &std::path::Path, permissions: JsonValue, pending: &Pending, interactive: bool) -> AgentEngine {
        let home = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        claude_rules_engine_at(home.path(), work, permissions, pending, interactive)
    }

    /// The same, with the person's home given (a temporary folder, never the real one).
    fn claude_rules_engine_at(home: &std::path::Path, work: &std::path::Path, permissions: JsonValue, pending: &Pending, interactive: bool) -> AgentEngine {
        std::fs::create_dir_all(work.join(".claude")).unwrap();
        std::fs::write(work.join(".claude/settings.json"), serde_json::json!({"permissions": permissions}).to_string()).unwrap();
        let mut engine = admin_engine(work, pending, interactive, !interactive);
        let admin = engine.session_permissions.clone();
        crate::engine::session_spec::coding_caller_fixture().apply(&mut engine);
        engine.session_permissions = admin;
        engine.claude_rules = Arc::new(permission::ClaudeRules::load(Some(home), work));
        engine
    }

    #[tokio::test]
    async fn a_claude_deny_rule_refuses_a_read_and_says_which_file_refused_it() {
        let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let pending: Pending = Default::default();
        let asked = deny_all(&pending);
        let mut engine = claude_rules_engine(work.path(), serde_json::json!({"deny": ["Read(./.env)"]}), &pending, true);
        let refused = gate(&mut engine, "Read", serde_json::json!({"path": work.path().join(".env")})).await;
        assert!(refused.contains("`Read(./.env)`") && refused.contains(".claude/settings.json"), "{refused}");
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 0, "a deny rule is never asked");
    }

    #[tokio::test]
    async fn a_claude_ask_rule_asks_the_person_once_with_no_grant_offered() {
        let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let pending: Pending = Default::default();
        let seen = answer_first(&pending, crate::engine::permission::ALLOW_ONCE);
        let mut engine = claude_rules_engine(work.path(), serde_json::json!({"ask": ["Read(./notes/**)"]}), &pending, true);
        let args = serde_json::json!({"path": work.path().join("notes/a.md")});
        let id = Some("c1".to_string());
        let t = ToolTurn { tool: "Read", canonical: "Read", args: &args, session_id: None, tool_call_id: &id };
        let mut approved = Vec::new();
        assert!(engine.gate_permission(&t, &mut Vec::new(), &mut approved).await.is_none(), "allowed once");
        let (labels, request) = tokio::time::timeout(std::time::Duration::from_secs(5), seen).await.expect("asked").unwrap();
        assert_eq!(labels, [crate::engine::permission::ALLOW_ONCE, "Deny"]);
        assert!(request.is_none(), "the person, not a caller");
        assert_eq!(approved.len(), 1, "the read itself is allowed once");
    }

    #[tokio::test]
    async fn a_claude_ask_rule_asks_the_caller_for_this_one_action() {
        let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let pending: Pending = Default::default();
        let seen = answer_first(&pending, "Deny");
        let mut engine = claude_rules_engine(work.path(), serde_json::json!({"ask": ["Bash(cargo build:*)"]}), &pending, false);
        let refused = gate(&mut engine, "Bash", serde_json::json!({"command": "cargo build --release"})).await;
        let (labels, request) = tokio::time::timeout(std::time::Duration::from_secs(5), seen).await.expect("asked").unwrap();
        assert_eq!(labels, [crate::engine::permission::ALLOW_ONCE, "Deny"]);
        let request = request.expect("the caller is asked");
        assert!(request.outward && request.dir.is_none(), "only allow_once or deny answers it: {request:?}");
        assert!(request.reason.as_deref().is_some_and(|r| r.contains("`Bash(cargo build:*)`")), "{request:?}");
        assert!(refused.contains("`Bash(cargo build:*)`"), "{refused}");
    }

    /// The calls that change `file`, as the model would make them.
    fn changes_of(file: &std::path::Path) -> Vec<(&'static str, JsonValue)> {
        vec![
            ("Write", serde_json::json!({"path": file, "content": "{}"})),
            ("Edit", serde_json::json!({"path": file, "old_string": "a", "new_string": "b"})),
            ("Bash", serde_json::json!({"command": format!("rm {}", file.display())})),
        ]
    }

    #[tokio::test]
    async fn a_change_to_a_settings_file_asks_the_person_once_and_only_once_per_change() {
        let home = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let pending: Pending = Default::default();
        for file in [work.path().join(".claude/settings.json"), work.path().join(".claude/settings.local.json"), home.path().join(".claude/settings.json")] {
            for (tool, args) in changes_of(&file) {
                // A denial closes the project for the rest of a run: one run each.
                let mut engine = claude_rules_engine_at(home.path(), work.path(), serde_json::json!({}), &pending, true);
                let seen = answer_first(&pending, crate::engine::permission::ALLOW_ONCE);
                let id = Some("c1".to_string());
                let t = ToolTurn { tool, canonical: tool, args: &args, session_id: None, tool_call_id: &id };
                assert!(engine.gate_permission(&t, &mut Vec::new(), &mut Vec::new()).await.is_none(), "{tool} {}: allowed once", file.display());
                let (labels, request) = tokio::time::timeout(std::time::Duration::from_secs(5), seen).await.expect("asked").unwrap();
                assert_eq!(labels, [crate::engine::permission::ALLOW_ONCE, "Deny"], "{tool}");
                assert!(request.is_none(), "the person, not a caller");
                // Allowing one change is not a grant: the next asks again.
                let seen = answer_first(&pending, "Deny");
                let refused = gate(&mut engine, tool, args.clone()).await;
                tokio::time::timeout(std::time::Duration::from_secs(5), seen).await.expect("asked again").unwrap();
                assert!(refused.contains("settings file"), "{tool}: {refused}");
            }
        }
    }

    #[tokio::test]
    async fn a_change_to_a_settings_file_asks_the_caller_with_no_grant_to_give() {
        let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let pending: Pending = Default::default();
        for (tool, args) in changes_of(&work.path().join(".claude/settings.local.json")) {
            let mut engine = claude_rules_engine(work.path(), serde_json::json!({}), &pending, false);
            let seen = answer_first(&pending, "Deny");
            let refused = gate(&mut engine, tool, args).await;
            let (labels, request) = tokio::time::timeout(std::time::Duration::from_secs(5), seen).await.expect("asked").unwrap();
            assert_eq!(labels, [crate::engine::permission::ALLOW_ONCE, "Deny"], "{tool}");
            let request = request.expect("the caller is asked");
            assert!(request.outward && request.dir.is_none(), "{tool}: only allow_once or deny answers it: {request:?}");
            assert!(refused.contains("settings file"), "{tool}: {refused}");
        }
    }

    #[tokio::test]
    async fn an_unattended_session_that_is_not_coding_cannot_change_a_settings_file() {
        let home = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let work = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let pending: Pending = Default::default();
        let asked = deny_all(&pending);
        // At admin everywhere, not a coding session, nobody to ask.
        let mut engine = admin_engine(work.path(), &pending, false, false);
        engine.claude_rules = Arc::new(permission::ClaudeRules::load(Some(home.path()), work.path()));
        let refused = gate(&mut engine, "Write", serde_json::json!({"path": work.path().join(".claude/settings.json"), "content": "{}"})).await;
        assert!(refused.contains("settings file"), "{refused}");
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 0, "nobody to ask");
    }
}

#[cfg(test)]
mod run_fault_tests {
    use super::*;
    use crate::engine::run_faults::FailureKind;

    fn engine_with_declared(tool: &str) -> AgentEngine {
        let root = std::env::temp_dir();
        let cfg = crate::engine::EngineConfig::from_app_config(
            &crate::config::Config::default(),
            root,
            crate::engine::InterfaceMode::Web,
        );
        let models = Arc::new(crate::provider::models::ModelManager::new_with_credentials(
            vec![],
            &crate::credentials::Credentials::default(),
        ));
        let mut engine =
            AgentEngine::new(cfg, models, "m".into(), crate::engine::AgentRole::Lead).unwrap();
        let def: crate::engine::skill_tool::SkillToolDef = serde_json::from_value(
            serde_json::json!({ "name": tool, "description": "d", "cmd": "sh fetch.sh" }),
        )
        .unwrap();
        engine.tools.skill_tools.insert(tool.to_string(), def);
        engine
    }

    fn output(exit: Option<i32>, stderr: &str) -> anyhow::Result<tools::ToolResult> {
        Ok(tools::ToolResult::CommandOutput {
            exit_code: exit,
            stdout: "private rows the model read".into(),
            stderr: stderr.into(),
        })
    }

    #[test]
    fn a_declared_tool_that_exits_non_zero_is_kept_with_its_stderr_tail_only() {
        let mut engine = engine_with_declared("fetch");
        engine.note_run_fault("fetch", &output(Some(3), "one\ntwo\nhost unreachable\n"));
        let f = &engine.run_faults.failures[0];
        assert_eq!((f.kind, f.exit), (FailureKind::Exit, Some(3)));
        assert_eq!(f.tail.as_deref(), Some("two / host unreachable"));
        assert!(!format!("{f:?}").contains("private rows"), "stdout leaked");
    }

    #[test]
    fn a_clean_declared_tool_and_a_bash_exit_are_not_failures() {
        let mut engine = engine_with_declared("fetch");
        engine.note_run_fault("fetch", &output(Some(0), ""));
        engine.note_run_fault("Bash", &output(Some(1), "no match"));
        engine.note_run_fault(
            "Grep",
            &Ok(tools::ToolResult::Success("(no matches)".into())),
        );
        assert!(engine.run_faults.is_empty(), "{:?}", engine.run_faults);
    }

    #[tokio::test]
    async fn a_refused_call_is_kept_with_no_tail() {
        let mut engine = engine_with_declared("fetch");
        let (args, id) = (serde_json::json!({}), Some("c1".to_string()));
        let t = ToolTurn {
            tool: "Write",
            canonical: "Write",
            args: &args,
            session_id: None,
            tool_call_id: &id,
        };
        let scope = Some(HashSet::from(["Read".to_string()]));
        let mut messages = Vec::new();
        let stop = engine.gate_allowed_list(&t, &scope, &mut messages).await;
        assert!(stop.is_some(), "Write was let through");
        let f = &engine.run_faults.failures[0];
        assert_eq!((f.tool.as_str(), f.kind), ("Write", FailureKind::Refused));
        assert_eq!(f.tail, None, "the engine's refusal text became a tail");
    }

    /// A declared tool that exits non-zero is one failed call: its card,
    /// its saved row and the run's fault count say so alike.
    #[tokio::test]
    async fn a_non_zero_exit_is_failed_on_the_card_the_row_and_the_count() {
        let mut engine = engine_with_declared("fetch");
        engine.agent_id = Some("ling".into());
        let (manager, mut events) = crate::engine::agent::AgentManager::new(
            crate::config::Config::default(),
            None,
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            crate::engine::InterfaceMode::Web,
        );
        engine.set_manager_context(manager.clone());
        let sid = format!("sess-1-exit-{}", uuid::Uuid::new_v4().simple());
        let exec = ReadyExec {
            canonical_tool: "fetch".into(),
            sig: "fetch{}".into(),
            original_args: serde_json::json!({}),
            approved_reads: Vec::new(),
            tool_done_status: "Fetched".into(),
            tool_failed_status: "Fetch failed".into(),
            block_id: "b1".into(),
            tool_call_id: Some("c1".into()),
        };
        let (mut messages, mut cache, mut streak) = (Vec::new(), HashMap::new(), 0);
        engine
            .post_execute_tool(exec, output(Some(3), "host unreachable\n"), &mut messages, &mut cache, &mut HashSet::new(), &mut streak, Some(&sid))
            .await;
        assert_eq!(engine.run_faults.count, 1);
        let card = std::iter::from_fn(|| events.try_recv().ok())
            .find_map(|(event, _)| match event {
                crate::engine::agent::AgentEvent::ContentBlockUpdate { block_id, is_error, .. } if block_id == "b1" => is_error,
                _ => None,
            });
        assert_eq!(card, Some(true), "the card");
        let rows = manager.global_sessions.get_chat_history(&sid).unwrap();
        let row = rows.iter().find(|r| r.content.starts_with("Tool fetch:")).expect("saved");
        assert!(row.failed, "the saved row");
    }

    #[test]
    fn a_tool_error_is_a_failure_and_a_cancel_is_not() {
        let mut engine = engine_with_declared("fetch");
        engine.note_run_fault("fetch", &Err(anyhow::anyhow!("run cancelled")));
        assert!(engine.run_faults.is_empty());
        engine.note_run_fault(
            "Read",
            &Err(anyhow::anyhow!("timeout: Read did not finish")),
        );
        assert_eq!(engine.run_faults.failures[0].kind, FailureKind::Error);
        assert_eq!(engine.run_faults.count, 1);
    }
}

/// A written file's text as the chat's diff shows it: the first 10,000 bytes,
/// cut at a character boundary — a fixed byte index can fall inside a
/// multi-byte character (any CJK text), and slicing there stops the turn.
fn write_preview(content: &str) -> String {
    const SHOWN: usize = 10_000;
    if content.len() <= SHOWN {
        return content.to_string();
    }
    let cut = content.floor_char_boundary(SHOWN);
    format!("{}…\n(truncated, {} total chars)", &content[..cut], content.len())
}

#[cfg(test)]
mod write_preview_tests {
    use super::write_preview;

    #[test]
    fn a_long_cjk_file_is_cut_between_characters() {
        // 9,999 ASCII bytes, then three-byte characters: byte 10,000 falls
        // inside the first of them.
        let content = format!("{}{}", "a".repeat(9_999), "、".repeat(50));
        let preview = write_preview(&content);
        assert!(preview.starts_with(&"a".repeat(9_999)));
        assert!(preview.contains("(truncated, 10149 total chars)"));
        assert!(!preview[..preview.find('…').unwrap()].contains('、'), "cut before the character it would split");
    }

    #[test]
    fn a_short_file_is_shown_whole() {
        assert_eq!(write_preview("你好"), "你好");
    }
}
