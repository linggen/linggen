use crate::agent_manager::models::ModelManager;
use crate::agent_manager::AgentManager;
use crate::config::AgentSpec;
use crate::engine::permission;
use crate::engine::tool_registry::ToolRegistry;
use crate::engine::tools;
use crate::ollama::ChatMessage;
use crate::skills::Skill;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Plan mode data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanItem {
    pub id: String,
    pub title: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub summary: String,
    pub status: PlanStatus,
    /// Free-form markdown plan text.
    #[serde(default)]
    pub plan_text: String,
    /// Structured todo items from UpdatePlan.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<PlanItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Planned,
    Approved,
    Executing,
    Completed,
    Rejected,
}


#[derive(Debug, Clone)]
pub enum ThinkingEvent {
    /// Internal reasoning token (hidden from user, shows "Thinking..." indicator).
    Token(String),
    /// Visible content token (streamed to user in real-time).
    ContentToken(String),
    /// Thinking stream finished (legacy path — triggers "thinking done" in UI).
    Done,
    /// Content stream finished (native tool calling — does NOT re-enable thinking flag).
    ContentDone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentRole {
    #[serde(rename = "lead")]
    Lead,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceMode {
    Web,
}

impl std::fmt::Display for InterfaceMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InterfaceMode::Web => write!(f, "Web UI"),
        }
    }
}

pub struct EngineConfig {
    pub ws_root: PathBuf,
    pub max_iters: usize,
    pub write_safety_mode: crate::config::WriteSafetyMode,
    /// Legacy field — kept for backward compat during migration. Use `permission_mode`.
    pub tool_permission_mode: crate::config::ToolPermissionMode,
    /// New permission mode (chat/read/edit/admin). See permission-spec.md.
    pub permission_mode: crate::engine::permission::PermissionMode,
    /// Config deny rules from `[permissions]` in linggen.toml.
    pub deny_rules: Vec<String>,
    /// Config ask rules from `[permissions]` in linggen.toml.
    pub ask_rules: Vec<String>,
    pub prompt_loop_breaker: Option<String>,
    pub interface_mode: InterfaceMode,
    /// When set, Bash commands must match one of these prefixes.
    /// Used by mission "standard" tier to restrict commands to build/test/git-read.
    pub bash_allow_prefixes: Option<Vec<String>>,
    /// When set, restricts available tools to this set (mission permission tiers).
    /// This is applied at engine level, before the agent spec tool list.
    pub mission_allowed_tools: Option<std::collections::HashSet<String>>,
    /// When set, restricts available tools for proxy room consumers.
    /// Owner configures which tools consumers can use via room_config.toml.
    pub consumer_allowed_tools: Option<std::collections::HashSet<String>>,
    /// When set, restricts available skills for proxy room consumers.
    pub consumer_allowed_skills: Option<std::collections::HashSet<String>>,
    /// Default session policy preset for user sessions (`interactive` /
    /// `strict` / `trusted` / `sandbox`). Set from `[agent] default_policy`
    /// in linggen.toml. When unset, sessions default to `interactive`.
    pub default_policy: Option<String>,
    /// Every N user messages, inject a hidden memory self-review nudge into
    /// the turn's message list. `0` disables. Default 6.
    pub memory_nudge_interval: usize,
}

impl EngineConfig {
    /// Compute the cascading intersection of mission + consumer tool restrictions.
    /// Returns None if no restrictions apply (all tools allowed from config perspective).
    pub fn effective_tool_restrictions(&self) -> Option<std::collections::HashSet<String>> {
        match (&self.mission_allowed_tools, &self.consumer_allowed_tools) {
            (None, None) => None,
            (Some(m), None) => Some(m.clone()),
            (None, Some(c)) => Some(c.clone()),
            (Some(m), Some(c)) => Some(m.intersection(c).cloned().collect()),
        }
    }

    /// Check if a specific tool is allowed by config-level restrictions.
    pub fn is_tool_allowed(&self, tool: &str) -> bool {
        match self.effective_tool_restrictions() {
            None => true,
            Some(ref allowed) => allowed.contains(tool),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextType {
    System,
    UserInput,
    AssistantReply,
    ToolCall,
    ToolResult,
    Observation,
    Status,
    Error,
    Summary,
}

#[derive(Debug, Clone)]
pub struct ObservationRecord {
    pub observation_type: String,
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextRecord {
    pub id: u64,
    pub ts: u64,
    pub context_type: ContextType,
    pub name: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub content: String,
    pub meta: JsonValue,
}

pub struct AgentEngine {
    pub cfg: EngineConfig,
    pub model_manager: Arc<ModelManager>,
    pub model_id: String,
    /// The originally-configured model ID (before any fallback).
    /// Used to reset model_id at the start of each turn when no session
    /// override is active, so fallback state doesn't persist.
    pub default_model_id: String,
    pub tools: ToolRegistry,
    pub role: AgentRole,
    pub task: Option<String>,
    /// Prompt template store (loaded from ~/.linggen/prompts/ with embedded fallbacks).
    pub prompt_store: std::sync::Arc<crate::prompts::PromptStore>,
    // Agent spec and runtime context
    pub spec: Option<AgentSpec>,
    pub spec_system_prompt: Option<String>,
    pub agent_id: Option<String>,
    // Rolling observations with metadata that we feed back to the model.
    pub observations: Vec<ObservationRecord>,
    pub context_records: Vec<ContextRecord>,
    pub next_context_id: u64,
    // Conversational history for chat.
    pub chat_history: Vec<ChatMessage>,
    // Active skill if any
    pub active_skill: Option<Skill>,
    /// Metadata for skills available to the model via the Skill tool: (name, description).
    pub available_skills_metadata: Vec<(String, String)>,
    /// Metadata for agents available for delegation via the Task tool: (id, description).
    pub available_agents_metadata: Vec<(String, String)>,
    pub parent_agent_id: Option<String>,
    /// Unique run_id of the parent agent when this is a subagent.
    /// Populated by `run_delegation` so subagents can tag emitted events
    /// with the parent's run_id, letting the UI route their activity to
    /// the correct subagent tree entry even when multiple instances share
    /// the same `agent_id` (e.g. several "ling" subagents in parallel).
    pub parent_run_id: Option<String>,
    pub run_id: Option<String>,
    /// Session this engine is running in — threaded into every emitted event
    /// so the server can route events without a shared mutable map.
    pub session_id: Option<String>,
    pub thinking_tx: Option<mpsc::UnboundedSender<ThinkingEvent>>,
    /// Receiver for user interrupt messages injected while the agent loop is running.
    pub interrupt_rx: Option<mpsc::UnboundedReceiver<String>>,
    // Plan mode
    pub plan_mode: bool,
    pub plan: Option<Plan>,
    /// Base64-encoded images to attach to the next user message.
    pub pending_images: Vec<String>,
    /// Session-scoped permissions (path modes, allows, denied sigs). See permission-spec.md.
    pub session_permissions: permission::SessionPermissions,
    /// Prompt profile — which system prompt sections to include (owner vs consumer).
    pub prompt_profile: super::prompt_profile::PromptProfile,
    /// Directory for the current session (for persisting permission.json).
    pub session_dir: Option<PathBuf>,
    /// Ordered list of default model IDs from routing config (for fallback chain).
    pub default_models: Vec<String>,
    /// Whether to automatically try fallback models on transient errors.
    pub auto_fallback: bool,
    /// Cached context window size (in tokens) for the active model.
    /// Queried once at loop start and used to adapt compaction thresholds.
    pub context_window_tokens: Option<usize>,
    /// Token usage from the most recent API response.
    pub last_token_usage: Option<crate::agent_manager::models::TokenUsage>,
    /// Cached stable portion of the system prompt.
    pub(crate) cached_system_prompt: Option<CachedSystemPrompt>,
    /// Importance tags for each message in the current messages vec,
    /// kept in sync with the messages vec during the agent loop.
    pub(crate) message_importance: Vec<MessageImportance>,
    /// Running token estimate accumulated incrementally during the loop.
    /// Reset after compaction. Avoids re-scanning all messages each iteration.
    pub(crate) accumulated_token_estimate: usize,
    /// Last assistant message text emitted during the agent loop.
    /// Set when the loop ends with a text-only response or ExitPlanMode;
    /// used by delegation callers to surface the sub-agent's response.
    pub(crate) last_assistant_text: Option<String>,
    /// When true, tool result messages use `role: "tool"` instead of `role: "user"`.
    /// Required by Ollama native tool calling — Ollama expects tool results
    /// after an assistant message with tool_calls.
    pub(crate) native_tool_mode: bool,
    // denied_tool_sigs moved to session_permissions.denied_sigs (persisted per session).
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum AgentOutcome {
    #[serde(rename = "plan")]
    Plan(Plan),
    /// User approved the plan inline via AskUser — ready for immediate execution.
    #[serde(rename = "plan_approved")]
    PlanApproved(Plan),
    #[serde(rename = "plan_mode_requested")]
    PlanModeRequested {
        #[serde(default)]
        reason: Option<String>,
    },
    #[serde(rename = "none")]
    None,
}

// ---------------------------------------------------------------------------
// Internal types shared across engine submodules
// ---------------------------------------------------------------------------

/// Control flow returned by extracted loop helpers.
pub(crate) enum LoopControl {
    /// Continue to the next iteration of the agent loop.
    Continue,
    /// Exit the loop and return this outcome.
    Return(AgentOutcome),
}

/// Result of pre-execution validation for a tool call.
pub(crate) enum PreExecOutcome {
    /// The tool call was blocked (permission denied, cached, redundant, etc.)
    Blocked(LoopControl),
    /// Ready to execute: returns the ToolCall and metadata for post-processing.
    Ready(tools::ToolCall, ReadyExec),
}

/// Metadata captured during pre-execution, needed for post-processing.
pub(crate) struct ReadyExec {
    pub canonical_tool: String,
    pub sig: String,
    pub original_args: JsonValue,
    pub tool_done_status: String,
    pub tool_failed_status: String,
    /// Unique ID for the content block (used by Web UI content-block events).
    pub block_id: String,
    /// The tool_call_id from native function calling (for threading results back).
    pub tool_call_id: Option<String>,
}

/// A fully parsed tool call from native function calling.
#[derive(Debug, Clone)]
pub(crate) struct ParsedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    /// Gemini thought signature (must be echoed back in conversation history).
    pub thought_signature: Option<String>,
}

/// Result of streaming model output, including early-detected first action.
pub(crate) struct StreamResult {
    pub full_text: String,
    pub token_usage: Option<crate::agent_manager::models::TokenUsage>,
    /// First action detected mid-stream (avoids re-parsing it later).
    pub first_action: Option<(super::actions::ModelAction, usize)>,
    /// Tool calls from native function calling (empty in legacy mode).
    pub tool_calls: Vec<ParsedToolCall>,
}

/// Cached system prompt with hash for quick staleness checks.
pub(crate) struct CachedSystemPrompt {
    pub input_hash: u64,
    pub content: String,
}

/// Importance level assigned to each message in the conversation history.
/// Used by the compaction algorithm to preserve high-value messages longer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MessageImportance {
    Low = 0,      // empty search results, routine status
    Normal = 1,   // standard tool results
    High = 2,     // errors, write/edit results, user messages
    Critical = 3, // system prompt, user task
}

#[derive(Clone)]
pub(crate) struct CachedToolObs {
    pub model: String,
}

/// Mutable state carried through the agent loop iterations.
/// Extracted to allow helper methods to accept it as a single `&mut LoopState`.
pub(crate) struct LoopState {
    pub messages: Vec<ChatMessage>,
    pub allowed_tools: Option<HashSet<String>>,
    pub read_paths: HashSet<String>,
    pub tool_cache: HashMap<String, CachedToolObs>,
    pub empty_search_streak: usize,
    pub redundant_tool_streak: usize,
    pub last_tool_sig: String,
    pub invalid_json_streak: usize,
    pub last_assistant_response: String,
    pub identical_response_streak: usize,
    pub loop_nudge_count: usize,
    pub empty_response_streak: usize,
    pub progress_rx: mpsc::UnboundedReceiver<(String, String, String)>,
}

// ---------------------------------------------------------------------------
// AgentEngine constructor + simple setters/getters
// ---------------------------------------------------------------------------

impl AgentEngine {
    pub fn new(
        cfg: EngineConfig,
        model_manager: Arc<ModelManager>,
        model_id: String,
        role: AgentRole,
    ) -> Result<Self> {
        let mut builtins = tools::Tools::new(cfg.ws_root.clone())?;
        let prompt_store = {
            let override_dir = crate::prompts::PromptStore::default_override_dir();
            std::sync::Arc::new(crate::prompts::PromptStore::load(Some(override_dir.as_path())))
        };
        builtins.set_prompt_store(std::sync::Arc::clone(&prompt_store));
        let tools = ToolRegistry::new(builtins);
        Ok(Self {
            cfg,
            model_manager,
            default_model_id: model_id.clone(),
            model_id,
            tools,
            role,
            task: None,
            prompt_store,
            spec: None,
            spec_system_prompt: None,
            agent_id: None,
            observations: Vec::new(),
            context_records: Vec::new(),
            next_context_id: 1,
            chat_history: Vec::new(),
            active_skill: None,
            available_skills_metadata: Vec::new(),
            available_agents_metadata: Vec::new(),
            parent_agent_id: None,
            parent_run_id: None,
            run_id: None,
            session_id: None,
            thinking_tx: None,
            interrupt_rx: None,
            plan_mode: false,
            plan: None,
            pending_images: Vec::new(),
            session_permissions: permission::SessionPermissions::default(),
            prompt_profile: super::prompt_profile::PromptProfile::default(),
            session_dir: None,
            default_models: Vec::new(),
            auto_fallback: true,
            context_window_tokens: None,
            last_token_usage: None,
            cached_system_prompt: None,
            message_importance: Vec::new(),
            accumulated_token_estimate: 0,
            last_assistant_text: None,
            native_tool_mode: false,
        })
    }

    /// Check if the agent's cwd has entered/left a git project and update
    /// ws_root + invalidate the cached system prompt accordingly.
    pub fn check_working_folder_change(&mut self) {
        use crate::engine::tools::search_exec_find_git_root;
        let cwd = self.tools.builtins.cwd();
        // Canonicalize to resolve symlinks (e.g. /tmp → /private/tmp on macOS)
        let cwd = cwd.canonicalize().unwrap_or(cwd);
        let git_root = search_exec_find_git_root(&cwd);
        // If inside a git repo, use the git root. Otherwise use the cwd itself
        // so Read/Glob/Grep resolve relative paths from where the agent actually is.
        let new_ws_root = git_root.unwrap_or(cwd);
        if new_ws_root != self.cfg.ws_root {
            tracing::info!(
                "Working folder changed: {} → {}",
                self.cfg.ws_root.display(),
                new_ws_root.display()
            );
            self.cfg.ws_root = new_ws_root.clone();
            // Update the tools root so Read/Write/Edit/Glob/Grep resolve
            // relative paths from the new workspace root.
            self.tools.builtins.set_workspace_root(new_ws_root.clone());
            self.cached_system_prompt = None; // Force rebuild with new CLAUDE.md
        }
    }

    pub fn set_spec(&mut self, agent_id: String, spec: AgentSpec, system_prompt: String) {
        self.agent_id = Some(agent_id);
        self.spec = Some(spec);
        self.spec_system_prompt = Some(system_prompt);
    }

    pub fn set_manager_context(&mut self, manager: Arc<AgentManager>) {
        if let Some(agent_id) = &self.agent_id {
            self.tools.set_context(manager, agent_id.clone());
        }
    }

    pub fn set_task(&mut self, task: String) {
        self.task = Some(task);
        self.observations.clear();
        self.context_records.clear();
        self.next_context_id = 1;
        self.chat_history.clear();
    }

    pub fn set_parent_agent(&mut self, parent_agent_id: Option<String>) {
        self.parent_agent_id = parent_agent_id;
    }

    pub fn set_parent_run_id(&mut self, parent_run_id: Option<String>) {
        self.parent_run_id = parent_run_id;
    }

    pub fn set_delegation_depth(&mut self, depth: usize, max_depth: usize) {
        self.tools.builtins.set_delegation_depth(depth);
        self.tools.builtins.set_max_delegation_depth(max_depth);
    }

    pub fn set_run_id(&mut self, run_id: Option<String>) {
        self.run_id = run_id.clone();
        self.tools.set_run_id(run_id);
    }

    pub fn get_task(&self) -> Option<String> {
        self.task.clone()
    }

    /// Load skill-defined tools into the registry based on the agent spec's skills list.
    /// Skills with `disable_model_invocation == true` are skipped (their tools are not
    /// registered in the model-facing tool schema).
    pub async fn load_skill_tools(&mut self, skill_manager: &crate::skills::SkillManager) {
        if self.spec.is_none() { return };

        // Phase 1: register skill-unique tool defs (shell, HTTP, data).
        // Capability tools (Memory_*, etc.) are NOT listed here — their
        // schemas live in `engine::capabilities`.
        for skill in skill_manager.list_skills().await {
            if skill.disable_model_invocation {
                continue;
            }
            for mut tool_def in skill.tool_defs {
                // Stamp the owning skill's name on each tool so HTTP dispatch
                // can find the skill's daemon without a second lookup by
                // walking the whole registry.
                tool_def.skill_name = Some(skill.name.clone());
                if tool_def.skill_dir.is_none() {
                    tool_def.skill_dir = skill.skill_dir.clone();
                }
                self.tools.register_skill_tool(tool_def);
            }
        }

        // Phase 2: cache which capabilities have a live provider. Drives
        // tool-schema injection and permission-tier lookup. We iterate the
        // engine's capability registry once and ask the skill manager for
        // each name.
        self.tools.active_capabilities.clear();
        for cap in super::capabilities::CAPABILITIES.iter() {
            if skill_manager.active_provider(&cap.name).await.is_some() {
                self.tools.active_capabilities.insert(cap.name.clone());
            }
        }
    }

    /// Populate `available_skills_metadata` with (name, description) pairs
    /// for all locally-installed skills that are not `disable_model_invocation`.
    pub async fn load_available_skills_metadata(
        &mut self,
        skill_manager: &crate::skills::SkillManager,
    ) {
        let all_skills = skill_manager.list_skills().await;
        self.available_skills_metadata = all_skills
            .into_iter()
            .filter(|s| !s.disable_model_invocation)
            .map(|s| (s.name, s.description))
            .collect();
    }

    pub(crate) async fn is_cancelled(&self) -> bool {
        let Some(run_id) = &self.run_id else {
            return false;
        };
        let Some(manager) = self.tools.get_manager() else {
            return false;
        };
        manager.is_run_cancelled(run_id).await
    }

    pub(crate) fn outbound_target(&self) -> String {
        self.parent_agent_id
            .clone()
            .unwrap_or_else(|| "user".to_string())
    }
}
