//! `Tool` trait + built-in tool registry.
//!
//! Each built-in tool is a unit struct that implements [`Tool`] — name,
//! aliases, and an async `execute` body that parses its args and delegates
//! to the matching method on [`Tools`]. The registry ([`builtin_tools`])
//! is a `Vec<Arc<dyn Tool>>` constructed once at startup.
//!
//! This replaces the 12-arm match in `Tools::execute`. Adding a new
//! built-in tool = one more `impl Tool` and one more `Arc::new(...)` in
//! the registry.

use super::delegation::{RunAppArgs, SkillArgs, TaskArgs, WebFetchArgs, WebSearchArgs};
use super::file_tools::{CaptureScreenshotArgs, ListFilesArgs, ReadFileArgs};
use super::search_exec::{RunCommandArgs, SearchArgs};
use super::write_tools::{EditFileArgs, LockPathsArgs, UnlockPathsArgs, WriteFileArgs};
use super::{ToolCall, ToolResult, Tools};
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::{Arc, LazyLock};

#[async_trait]
pub trait Tool: Send + Sync {
    /// Canonical tool name as it appears in the model's tool list.
    fn name(&self) -> &'static str;

    /// Alternate names the model might emit (case + snake_case variants).
    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }

    /// Permission tier the agent must hold on the target path before this
    /// tool can run. Drives the prompt-on-exceed behavior in
    /// `engine::permission::check_permission`.
    fn tier(&self) -> PermissionMode;

    /// Run the tool. The implementor parses `call.args` into its typed
    /// form and dispatches to the matching method on `tools`.
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
            Arc::new(CaptureScreenshotTool),
            Arc::new(WriteTool),
            Arc::new(EditTool),
            Arc::new(LockPathsTool),
            Arc::new(UnlockPathsTool),
            Arc::new(TaskTool),
            Arc::new(SkillTool),
            Arc::new(RunAppTool),
            Arc::new(WebSearchTool),
            Arc::new(WebFetchTool),
            Arc::new(AskUserTool),
        ]
    });
    &REGISTRY
}

/// Look up a tool by canonical name or alias. `None` if no built-in
/// tool matches.
pub(super) fn lookup(name: &str) -> Option<&'static Arc<dyn Tool>> {
    registry().iter().find(|t| t.name() == name || t.aliases().contains(&name))
}

/// Public tier lookup used by `engine::permission::tool_action_tier`.
pub fn builtin_tier(name: &str) -> Option<crate::engine::permission::PermissionMode> {
    lookup(name).map(|t| t.tier())
}

// ---------------------------------------------------------------------------
// File tools
// ---------------------------------------------------------------------------

pub struct GlobTool;
#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &'static str { "Glob" }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: ListFilesArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for Glob: {}", e))?;
        tools.list_files(args).await
    }
}

pub struct ReadTool;
#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &'static str { "Read" }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: ReadFileArgs = serde_json::from_value(call.args).map_err(|e| {
            anyhow::anyhow!(
                "invalid args for Read: {}. Expected keys: path|max_bytes|line_range",
                e
            )
        })?;
        tools.read_file(args).await
    }
}

pub struct GrepTool;
#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &'static str { "Grep" }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: SearchArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for Grep: {}", e))?;
        tools.search_rg(args).await
    }
}

pub struct CaptureScreenshotTool;
#[async_trait]
impl Tool for CaptureScreenshotTool {
    fn name(&self) -> &'static str { "capture_screenshot" }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: CaptureScreenshotArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for capture_screenshot: {}", e))?;
        tools.capture_screenshot(args).await
    }
}

// ---------------------------------------------------------------------------
// Bash
// ---------------------------------------------------------------------------

pub struct BashTool;
#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &'static str { "Bash" }
    fn tier(&self) -> PermissionMode { PermissionMode::Admin }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let mut args: RunCommandArgs = serde_json::from_value(call.args).map_err(|e| {
            anyhow::anyhow!(
                "invalid args for Bash: {}. Expected keys: cmd|timeout_ms",
                e
            )
        })?;
        // Bash is the only tool with mid-execution cancellation: register a
        // cancel flag against the block_id so an in-flight `kill` from the
        // UI can interrupt the child process.
        if let (Some(bid), Some(mgr)) = (&call.block_id, &tools.manager) {
            args.cancel_flag = Some(mgr.register_tool_cancel_flag(bid));
        }
        let result = tools.run_command(args).await;
        if let (Some(bid), Some(mgr)) = (&call.block_id, &tools.manager) {
            mgr.clear_tool_cancel_flag(bid);
        }
        result
    }
}

// ---------------------------------------------------------------------------
// Write tools
// ---------------------------------------------------------------------------

pub struct WriteTool;
#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &'static str { "Write" }
    fn tier(&self) -> PermissionMode { PermissionMode::Edit }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: WriteFileArgs = serde_json::from_value(call.args).map_err(|e| {
            anyhow::anyhow!("invalid args for Write: {}. Expected keys: path|content", e)
        })?;
        tools.write_file(args).await
    }
}

pub struct EditTool;
#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &'static str { "Edit" }
    fn tier(&self) -> PermissionMode { PermissionMode::Edit }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: EditFileArgs = serde_json::from_value(call.args).map_err(|e| {
            anyhow::anyhow!(
                "invalid args for Edit: {}. Expected keys: path|old_string|new_string|replace_all?",
                e
            )
        })?;
        tools.edit_file(args).await
    }
}

pub struct LockPathsTool;
#[async_trait]
impl Tool for LockPathsTool {
    fn name(&self) -> &'static str { "lock_paths" }
    fn tier(&self) -> PermissionMode { PermissionMode::Admin }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: LockPathsArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for lock_paths: {}", e))?;
        tools.lock_paths(args).await
    }
}

pub struct UnlockPathsTool;
#[async_trait]
impl Tool for UnlockPathsTool {
    fn name(&self) -> &'static str { "unlock_paths" }
    fn tier(&self) -> PermissionMode { PermissionMode::Admin }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: UnlockPathsArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for unlock_paths: {}", e))?;
        tools.unlock_paths(args).await
    }
}

// ---------------------------------------------------------------------------
// Delegation, skill, app
// ---------------------------------------------------------------------------

pub struct TaskTool;
#[async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &'static str { "Task" }
    fn aliases(&self) -> &'static [&'static str] { &["delegate_to_agent"] }
    fn tier(&self) -> PermissionMode { PermissionMode::Admin }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: TaskArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for Task: {}", e))?;
        tools.task(args).await
    }
}

pub struct SkillTool;
#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &'static str { "Skill" }
    fn aliases(&self) -> &'static [&'static str] { &["skill"] }
    fn tier(&self) -> PermissionMode { PermissionMode::Admin }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: SkillArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for Skill: {}", e))?;
        tools.invoke_skill(args).await
    }
}

pub struct RunAppTool;
#[async_trait]
impl Tool for RunAppTool {
    fn name(&self) -> &'static str { "RunApp" }
    fn aliases(&self) -> &'static [&'static str] { &["run_app"] }
    fn tier(&self) -> PermissionMode { PermissionMode::Admin }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: RunAppArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for RunApp: {}", e))?;
        tools.run_app(args).await
    }
}

// ---------------------------------------------------------------------------
// Web tools (genuinely async — no spawn_blocking inside)
// ---------------------------------------------------------------------------

pub struct WebSearchTool;
#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &'static str { "WebSearch" }
    fn aliases(&self) -> &'static [&'static str] { &["web_search"] }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    async fn execute(&self, _tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: WebSearchArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for WebSearch: {}", e))?;
        let max = args.max_results.unwrap_or(5).min(10);
        let results = crate::engine::web_search::web_search(&args.query, max).await?;
        Ok(ToolResult::WebSearchResults {
            query: args.query,
            results,
        })
    }
}

pub struct WebFetchTool;
#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &'static str { "WebFetch" }
    fn aliases(&self) -> &'static [&'static str] { &["web_fetch"] }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    async fn execute(&self, _tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: WebFetchArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for WebFetch: {}", e))?;
        let result = crate::engine::web_fetch::fetch_url(&args.url, args.max_bytes).await?;
        Ok(ToolResult::WebFetchContent {
            url: result.url,
            content: result.content,
            content_type: result.content_type,
            truncated: result.truncated,
        })
    }
}

// ---------------------------------------------------------------------------
// AskUser
// ---------------------------------------------------------------------------

pub struct AskUserTool;
#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &'static str { "AskUser" }
    fn aliases(&self) -> &'static [&'static str] { &["ask_user"] }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        tools.ask_user(call.args).await
    }
}
