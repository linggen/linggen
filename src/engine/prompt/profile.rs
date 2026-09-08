//! Prompt profile — declares which system prompt sections to include.
//!
//! Set once by `SessionPolicy::apply()`, read by the prompt builder.
//! No scattered if/else — the profile is the single decision point.

/// Which prompt sections to include for this session.
#[derive(Debug, Clone)]
pub struct PromptProfile {
    /// Environment block: ws_root, shell, platform.
    pub include_environment: bool,
    /// Project context files: CLAUDE.md, AGENTS.md, .cursorrules.
    pub include_project_context: bool,
    /// Memory: project MEMORY.md + global MEMORY.md.
    pub include_memory: bool,
    /// Frame the turn as an autonomous task: wrap the task text in the
    /// bootstrap message (workspace listing + step-by-step coda). Off for a
    /// person's session — their message reaches the model verbatim, the way
    /// a chat turn does in Claude Code. On for missions, delegated subagents
    /// and headless agent runs, where the input really is a task.
    pub task_bootstrap: bool,
    /// Available agents for Task delegation.
    pub include_delegation: bool,
    /// Consumer-specific frame: explains constraints to the model.
    pub consumer_frame: bool,
    /// Scoped per-app memory: when set (from the bound skill's
    /// `memory-context`), the turn auto-recalls ONLY this namespace —
    /// independent of `include_memory` (which stays off for skill sessions, so
    /// the core block / full biography is NOT injected). The two `recall_*`
    /// fields tune that scoped recall; `None` → engine defaults.
    pub memory_context: Option<String>,
    pub memory_recall_min_score: Option<f32>,
    pub memory_recall_count: Option<usize>,
}

impl PromptProfile {
    /// Owner — a person's session: full prompt, all sections, turns verbatim.
    pub fn owner() -> Self {
        Self {
            include_environment: true,
            include_project_context: true,
            include_memory: true,
            task_bootstrap: false,
            include_delegation: true,
            consumer_frame: false,
            memory_context: None,
            memory_recall_min_score: None,
            memory_recall_count: None,
        }
    }

    /// Consumer — restricted prompt, no owner-private sections.
    pub fn consumer() -> Self {
        Self {
            include_environment: false,
            include_project_context: false,
            include_memory: false,
            task_bootstrap: false,
            include_delegation: false,
            consumer_frame: true,
            memory_context: None,
            memory_recall_min_score: None,
            memory_recall_count: None,
        }
    }

    /// Autonomous run — a mission, a delegated subagent, a headless agent
    /// run: the owner's sections, and the task framed as a task.
    pub fn autonomous() -> Self {
        Self {
            task_bootstrap: true,
            ..Self::owner()
        }
    }
}

/// An engine that no session policy has claimed is running a task, not
/// talking to a person: missions and headless runs set `task` directly and
/// never pass through `SessionPolicy::apply()`.
impl Default for PromptProfile {
    fn default() -> Self {
        Self::autonomous()
    }
}
