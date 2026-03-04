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
pub struct Plan {
    pub summary: String,
    pub status: PlanStatus,
    /// Free-form markdown plan text.
    #[serde(default)]
    pub plan_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Planned,
    Approved,
    Executing,
    Completed,
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
    #[serde(rename = "coder")]
    Coder,
    #[serde(rename = "operator")]
    Operator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPacket {
    pub title: String,
    pub user_stories: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub mermaid_wireframe: Option<String>,
}

pub struct EngineConfig {
    pub ws_root: PathBuf,
    pub max_iters: usize,
    pub write_safety_mode: crate::config::WriteSafetyMode,
    pub tool_permission_mode: crate::config::ToolPermissionMode,
    pub prompt_loop_breaker: Option<String>,
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
    pub parent_agent_id: Option<String>,
    pub run_id: Option<String>,
    pub thinking_tx: Option<mpsc::UnboundedSender<ThinkingEvent>>,
    /// Receiver for user interrupt messages injected while the agent loop is running.
    pub interrupt_rx: Option<mpsc::UnboundedReceiver<String>>,
    // Plan mode
    pub plan_mode: bool,
    pub plan: Option<Plan>,
    /// Path to the plan file in `~/.linggen/plans/`.
    pub plan_file_path: Option<PathBuf>,
    /// Base64-encoded images to attach to the next user message.
    pub pending_images: Vec<String>,
    /// Tool permission store (session + project scoped allows).
    pub permission_store: permission::PermissionStore,
    /// Ordered list of default model IDs from routing config (for fallback chain).
    pub default_models: Vec<String>,
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
    /// Set by `Done` dispatch; used by delegation callers to surface the sub-agent's response.
    pub(crate) last_assistant_text: Option<String>,
    /// When true, tool result messages use `role: "tool"` instead of `role: "user"`.
    /// Required by Ollama native tool calling — Ollama expects tool results
    /// after an assistant message with tool_calls.
    pub(crate) native_tool_mode: bool,
    /// Tool call signatures the user has denied permission for.
    /// Lives on AgentEngine (not LoopState) so denials persist across runs within a session.
    pub(crate) denied_tool_sigs: HashSet<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum AgentOutcome {
    #[serde(rename = "task")]
    Task(TaskPacket),
    #[serde(rename = "patch")]
    Patch(String),
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
        // Load project-scoped permissions from {workspace}/.linggen/permissions.json
        let perm_store = {
            let linggen_dir = cfg.ws_root.join(".linggen");
            permission::PermissionStore::load(&linggen_dir)
        };
        let prompt_store = {
            let override_dir = crate::prompts::PromptStore::default_override_dir();
            std::sync::Arc::new(crate::prompts::PromptStore::load(Some(override_dir.as_path())))
        };
        builtins.set_prompt_store(std::sync::Arc::clone(&prompt_store));
        let tools = ToolRegistry::new(builtins);
        Ok(Self {
            cfg,
            model_manager,
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
            parent_agent_id: None,
            run_id: None,
            thinking_tx: None,
            interrupt_rx: None,
            plan_mode: false,
            plan: None,
            plan_file_path: None,
            pending_images: Vec::new(),
            permission_store: perm_store,
            default_models: Vec::new(),
            context_window_tokens: None,
            last_token_usage: None,
            cached_system_prompt: None,
            message_importance: Vec::new(),
            accumulated_token_estimate: 0,
            last_assistant_text: None,
            native_tool_mode: false,
            denied_tool_sigs: HashSet::new(),
        })
    }

    pub fn set_spec(&mut self, agent_id: String, spec: AgentSpec, system_prompt: String) {
        let policy = spec.policy.clone();
        self.agent_id = Some(agent_id);
        self.spec = Some(spec);
        self.spec_system_prompt = Some(system_prompt);
        self.tools.set_policy(Some(policy));
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

    pub fn set_delegation_depth(&mut self, depth: usize, max_depth: usize) {
        self.tools.builtins.set_delegation_depth(depth);
        self.tools.builtins.set_max_delegation_depth(max_depth);
    }

    pub fn set_run_id(&mut self, run_id: Option<String>) {
        self.run_id = run_id.clone();
        self.tools.set_run_id(run_id);
    }

    pub fn set_memory_dir(&mut self, dir: PathBuf) {
        self.tools.set_memory_dir(dir);
    }

    pub fn get_task(&self) -> Option<String> {
        self.task.clone()
    }

    /// Load skill-defined tools into the registry based on the agent spec's skills list.
    /// Skills with `disable_model_invocation == true` are skipped (their tools are not
    /// registered in the model-facing tool schema).
    pub async fn load_skill_tools(&mut self, skill_manager: &crate::skills::SkillManager) {
        let Some(spec) = &self.spec else { return };
        let skill_names = spec.skills.clone();
        for skill_name in &skill_names {
            if let Some(skill) = skill_manager.get_skill(skill_name).await {
                if skill.disable_model_invocation {
                    continue;
                }
                for tool_def in skill.tool_defs {
                    self.tools.register_skill_tool(tool_def);
                }
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
