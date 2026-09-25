//! `Tool` trait + built-in tool registry.
//!
//! Each built-in tool is a unit struct that implements [`Tool`] — name,
//! aliases, tier, description, schema, and an async execute body. The
//! registry ([`registry`]) is a `Vec<Arc<dyn Tool>>` constructed once on
//! first access.
//!
//! Adding a new built-in tool: write one `impl Tool` block (name +
//! description + tier + schemas + execute) and append `Arc::new(YourTool)`
//! to the registry constructor. No edits to dispatcher/tier-table/
//! schema-table required.

mod agent_chat;
mod answer_prompt;
mod app_tool;
mod ask_user;
mod delegate;
mod exec;
mod fs;
mod image;
mod pet;
mod sense;
mod web;
mod write;

pub use agent_chat::*;
pub use answer_prompt::*;
pub use app_tool::*;
pub use ask_user::*;
pub use delegate::*;
pub use exec::*;
pub use fs::*;
pub use image::*;
pub use pet::*;
pub use sense::*;
pub use web::*;
pub use write::*;

use super::browser_tool::{
    BrowserClickTool, BrowserKeyTool, BrowserNavigateTool, BrowserReadConsoleTool,
    BrowserReadPageTool, BrowserScreenshotTool, BrowserScrollTool, BrowserTabsTool,
    BrowserTypeTool, BrowserWaitTool,
};
use super::{ToolCall, ToolResult, Tools};
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

#[async_trait]
pub trait Tool: Send + Sync {
    /// Canonical tool name as it appears in the model's tool list.
    fn name(&self) -> &'static str;

    /// Alternate names the model might emit (case + snake_case variants).
    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }

    /// Description shown to the model.
    fn description(&self) -> &'static str;

    /// Permission tier the agent must hold on the target path before this
    /// tool can run.
    fn tier(&self) -> PermissionMode;

    /// JSON Schema for the tool's arguments — for the native
    /// function-calling `tools` API parameter.
    fn args_schema(&self) -> Value;

    /// Legacy short-form schema for the system-prompt JSON-action
    /// embedding. Shape: `{"name", "args":{k: "type"}, "returns", "notes"?}`.
    fn legacy_schema_entry(&self) -> Value;

    /// True when this tool should appear in the model's advertised tool
    /// list. Internal tools (lock_paths, unlock_paths) are dispatched
    /// when called but never listed to the model.
    fn model_facing(&self) -> bool {
        true
    }

    /// The local-model lane this tool runs on, if any. A tool that names
    /// one is offered only on a machine the lane gate accepts.
    fn lane(&self) -> Option<&'static crate::runtime::Lane> {
        None
    }

    /// True when an identical later call may be served from the per-run
    /// tool cache and counted toward the redundant-loop nudge. Opt-in, for
    /// pure reads of the workspace only: a cached answer is a call that never
    /// ran, so anything that acts (writes, asks the user, delegates, drives a
    /// browser or an avatar) or reads live state (the web, a browser page,
    /// the user's presence) must run every time.
    fn cacheable(&self) -> bool {
        false
    }

    /// Wall-clock ceiling for one call. A backstop against hangs, not a
    /// latency policy — hence generous by default.
    ///
    /// Without one, a tool that blocks owns the run indefinitely: the turn
    /// never ends, the session stays busy, and cancellation cannot help
    /// because the block is inside a syscall that does not accept it. That is
    /// not hypothetical — a `**` glob rooted at a home directory descended
    /// into a network-backed cloud folder and held a run for 19 minutes.
    ///
    /// `None` = unbounded, for work that is open-ended by nature: waiting on
    /// the user, on a delegated subagent, or on a shell command the user
    /// asked for.
    fn max_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs(300))
    }

    /// Run the tool.
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult>;
}

/// Static registry of built-in tools. Constructed once on first access.
pub(super) fn registry() -> &'static [Arc<dyn Tool>] {
    static REGISTRY: LazyLock<Vec<Arc<dyn Tool>>> = LazyLock::new(|| {
        vec![
            Arc::new(GlobTool),
            Arc::new(ReadTool),
            Arc::new(GrepTool),
            Arc::new(BashTool),
            Arc::new(GenerateImageTool),
            Arc::new(WriteTool),
            Arc::new(EditTool),
            Arc::new(LockPathsTool),
            Arc::new(UnlockPathsTool),
            Arc::new(TaskTool),
            Arc::new(SkillTool),
            Arc::new(RunAppTool),
            Arc::new(WebSearchTool),
            Arc::new(WebFetchTool),
            Arc::new(ExpressTool),
            Arc::new(VoiceTool),
            Arc::new(SenseTool),
            Arc::new(RecentActivityTool),
            Arc::new(AnswerPromptTool),
            Arc::new(AgentChatTool),
            Arc::new(AppTool),
            Arc::new(AskUserTool),
            // Memory is NOT here. ling-mem is an MCP server and the model
            // uses its tools directly (`mcp__memory__memory_*`), discovered
            // at runtime rather than compiled in — so there is one memory
            // surface for Linggen, Claude Code, and Codex alike instead of
            // this engine's private restatement of it.
            // Browser_* — browser control over the bridge to the
            // linggen-browser extension (browser-control-spec.md). Mutating
            // actions are gated by the extension's own permission prompt.
            Arc::new(BrowserNavigateTool),
            Arc::new(BrowserReadPageTool),
            Arc::new(BrowserScreenshotTool),
            Arc::new(BrowserClickTool),
            Arc::new(BrowserTypeTool),
            Arc::new(BrowserKeyTool),
            Arc::new(BrowserScrollTool),
            Arc::new(BrowserWaitTool),
            Arc::new(BrowserReadConsoleTool),
            Arc::new(BrowserTabsTool),
        ]
    });
    &REGISTRY
}

/// Look up a tool by canonical name or alias. `None` if no built-in
/// tool matches.
pub(super) fn lookup(name: &str) -> Option<&'static Arc<dyn Tool>> {
    registry()
        .iter()
        .find(|t| t.name() == name || t.aliases().contains(&name))
}

/// Public tier lookup used by `engine::permission::tool_action_tier`.
pub fn builtin_tier(name: &str) -> Option<PermissionMode> {
    lookup(name).map(|t| t.tier())
}

/// Cache/redundancy-gate participation, used by `engine::tool_exec`.
///
/// Only a built-in that opts in (a pure workspace read) is cacheable. Unknown
/// tools — a skill's, an MCP server's — never are: the engine has no basis for
/// claiming an identical call returns an identical answer or did nothing, the
/// same reason `tool_max_duration` leaves them unbounded. A skill tool served
/// from cache is an action that never happened (2026-09-17: three identical
/// `Trade … buy` calls answered "paid" from cache after one real purchase),
/// and memory's store is live state shared across sessions and hosts.
pub fn tool_cacheable(name: &str) -> bool {
    lookup(name).is_some_and(|t| t.cacheable())
}

/// Wall-clock ceiling for a tool call, used by `engine::tool_exec`.
///
/// Unknown (custom / skill) tools are unbounded: a skill declares its own
/// work, and plenty of it legitimately runs for minutes (fetching media,
/// driving a long script). The engine has no basis for guessing their cost,
/// and a wrong guess here would abort real work.
pub fn tool_max_duration(name: &str) -> Option<Duration> {
    lookup(name).and_then(|t| t.max_duration())
}

/// JSON-Schema entries for the model-facing built-in tools. Used by
/// `engine::tools::json_schema::oai_tool_definitions`.
pub(super) fn model_facing_args_schemas() -> Vec<(String, String, Value)> {
    registry()
        .iter()
        .filter(|t| t.model_facing())
        .map(|t| {
            (
                t.name().to_string(),
                t.description().to_string(),
                t.args_schema(),
            )
        })
        .collect()
}

/// Legacy short-form schema entries for the system-prompt JSON-action
/// embedding. Used by `engine::tools::tool_helpers::full_tool_schema_entries`.
pub(super) fn model_facing_legacy_entries() -> Vec<Value> {
    registry()
        .iter()
        .filter(|t| t.model_facing())
        .map(|t| t.legacy_schema_entry())
        .collect()
}

#[cfg(test)]
mod max_duration_tests {
    use super::*;

    #[test]
    fn tools_that_wait_on_something_else_are_unbounded() {
        // A ceiling here would abort the user's own prompt, a subagent mid-run,
        // or a build they asked for.
        for name in ["AskUser", "Task", "Bash", "Skill", "RunApp", "agent_chat"] {
            assert_eq!(tool_max_duration(name), None, "{name} must stay unbounded");
        }
    }

    #[test]
    fn tree_walks_are_bounded_tighter_than_the_default() {
        let walk = tool_max_duration("Glob").expect("Glob must be bounded");
        let default = tool_max_duration("Read").expect("Read must be bounded");
        assert!(
            walk < default,
            "a walk should give up sooner than the default"
        );
        assert_eq!(tool_max_duration("Grep"), Some(walk));
    }

    #[test]
    fn unknown_tools_are_unbounded() {
        // Skill-provided tools declare their own work; the engine cannot guess.
        assert_eq!(tool_max_duration("SomeSkillProvidedTool"), None);
    }

    #[test]
    fn only_pure_workspace_reads_are_served_from_cache() {
        for name in ["Read", "Grep", "Glob"] {
            assert!(tool_cacheable(name), "{name} is a pure read");
        }
        // A skill's tool may act (a purchase); an MCP tool may read live state.
        assert!(!tool_cacheable("Trade"));
        assert!(!tool_cacheable("mcp__memory__memory_search"));
        for name in [
            "AskUser",
            "Bash",
            "Write",
            "Edit",
            "Task",
            "Skill",
            "RunApp",
            "Express",
            "Voice",
            "GenerateImage",
            "WebSearch",
            "WebFetch",
            "sense",
            "recent_activity",
            "answer_prompt",
            "agent_chat",
            "lock_paths",
            "unlock_paths",
            "Browser_navigate",
            "Browser_readPage",
        ] {
            assert!(!tool_cacheable(name), "{name} must run every time");
        }
    }
}
