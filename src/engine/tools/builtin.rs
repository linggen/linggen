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

use super::browser_tool::{
    BrowserClickTool, BrowserKeyTool, BrowserNavigateTool, BrowserReadConsoleTool,
    BrowserReadPageTool, BrowserScreenshotTool, BrowserScrollTool, BrowserTabsTool,
    BrowserTypeTool, BrowserWaitTool,
};
use super::delegation::{RunAppArgs, SkillArgs, TaskArgs, WebFetchArgs, WebSearchArgs};
use super::file_tools::{CaptureScreenshotArgs, ListFilesArgs, ReadFileArgs};
use super::memory_tool::{MemoryQueryTool, MemoryWriteTool};
use super::search_exec::{RunCommandArgs, SearchArgs};
use super::write_tools::{EditFileArgs, LockPathsArgs, UnlockPathsArgs, WriteFileArgs};
use super::{ToolCall, ToolResult, Tools};
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Timelike;
use serde_json::{json, Value};
use std::sync::{Arc, LazyLock};

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

    /// True when an identical later call may be served from the per-run
    /// tool cache and counted toward the redundant-loop nudge. Tools
    /// that read live mutable state outside the session (the memory
    /// store is shared across sessions and hosts) return false — an
    /// identical call can legitimately return new data.
    fn cacheable(&self) -> bool {
        true
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
            Arc::new(ExpressTool),
            Arc::new(SenseTool),
            Arc::new(AnswerPromptTool),
            Arc::new(AgentChatTool),
            Arc::new(AskUserTool),
            // Memory_query / Memory_write — engine-built-in (HTTP to
            // `ling-mem`). Previously routed through the now-defunct
            // `memory` capability abstraction.
            Arc::new(MemoryQueryTool),
            Arc::new(MemoryWriteTool),
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
    registry().iter().find(|t| t.name() == name || t.aliases().contains(&name))
}

/// Public tier lookup used by `engine::permission::tool_action_tier`.
pub fn builtin_tier(name: &str) -> Option<PermissionMode> {
    lookup(name).map(|t| t.tier())
}

/// Cache/redundancy-gate participation, used by `engine::tool_exec`.
/// Unknown (custom / skill) tools default to cacheable.
pub fn tool_cacheable(name: &str) -> bool {
    lookup(name).map(|t| t.cacheable()).unwrap_or(true)
}

/// JSON-Schema entries for the model-facing built-in tools. Used by
/// `engine::tools::json_schema::oai_tool_definitions`.
pub(super) fn model_facing_args_schemas() -> Vec<(String, String, Value)> {
    registry()
        .iter()
        .filter(|t| t.model_facing())
        .map(|t| (t.name().to_string(), t.description().to_string(), t.args_schema()))
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

// ---------------------------------------------------------------------------
// File tools
// ---------------------------------------------------------------------------

pub struct GlobTool;
#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &'static str { "Glob" }
    fn description(&self) -> &'static str {
        "Find files by glob pattern. Returns matching file paths sorted by modification time."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "globs": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Glob patterns to match (e.g. [\"**/*.rs\", \"src/**/*.ts\"])"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return"
                }
            },
            "required": ["globs"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Glob",
            "args": {"globs": "string[]?", "max_results": "number?"},
            "returns": "string[]",
            "notes": "Glob pattern aliases accepted: globs, pattern, glob."
        })
    }
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
    fn description(&self) -> &'static str {
        "Read a file's contents. Path can be relative (resolved from workspace root) or absolute. Always read a file before modifying it."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path to read (relative to workspace root, or absolute)"
                },
                "max_bytes": {
                    "type": "integer",
                    "description": "Maximum bytes to read (default: entire file)"
                },
                "line_range": {
                    "type": "array",
                    "items": {"type": "integer"},
                    "minItems": 2,
                    "maxItems": 2,
                    "description": "Line range [start, end] (1-based, inclusive)"
                }
            },
            "required": ["path"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Read",
            "args": {"path": "string", "max_bytes": "number?", "line_range": "[number,number]?"},
            "returns": "{path,content,truncated}",
            "notes": "Path aliases accepted: path, file, filepath."
        })
    }
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
    fn description(&self) -> &'static str {
        "Search file contents using regex. Returns matching lines with file path, line number, and snippet."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "globs": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "File glob patterns to search within (e.g. [\"**/*.rs\"])"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of matches to return"
                }
            },
            "required": ["query"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Grep",
            "args": {"query": "string", "globs": "string[]?", "max_results": "number?"},
            "returns": "{matches:[{path,line,snippet}]}",
            "notes": "Query aliases accepted: query, path, file, filepath."
        })
    }
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
    fn description(&self) -> &'static str { "Capture a screenshot of a URL." }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "URL to capture"},
                "delay_ms": {"type": "integer", "description": "Delay before capture in milliseconds"}
            },
            "required": ["url"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "capture_screenshot",
            "args": {"url": "string", "delay_ms": "number?"},
            "returns": "{url,base64}"
        })
    }
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
    fn description(&self) -> &'static str {
        "Run a shell command via sh -c. Working directory persists across calls (cd is remembered). Use for build, test, git, and other commands that require shell execution. Prefer dedicated tools (Read, Glob, Grep) over Bash equivalents."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Admin }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "cmd": {"type": "string", "description": "Shell command to execute"},
                "timeout_ms": {"type": "integer", "description": "Timeout in milliseconds (default: 30000)"}
            },
            "required": ["cmd"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Bash",
            "args": {"cmd": "string", "timeout_ms": "number?"},
            "returns": "{exit_code,stdout,stderr}",
            "notes": "Runs shell commands via sh -c. Permission required in ask mode. Command alias accepted: command."
        })
    }
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
    fn description(&self) -> &'static str {
        "Write content to a file (creates or overwrites). Prefer Edit for existing files. Path is relative to workspace root."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Edit }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path to write (relative to workspace root)"},
                "content": {"type": "string", "description": "Content to write to the file"}
            },
            "required": ["path", "content"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Write",
            "args": {"path": "string", "content": "string"},
            "returns": "success",
            "notes": "Path aliases accepted: path, file, filepath."
        })
    }
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
    fn description(&self) -> &'static str {
        "Apply an exact string replacement in a file. Prefer this over Write for existing files. Read the file first."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Edit }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path to edit (relative to workspace root)"},
                "old_string": {"type": "string", "description": "Exact string to find and replace"},
                "new_string": {"type": "string", "description": "Replacement string"},
                "replace_all": {"type": "boolean", "description": "Replace all occurrences (default: false, replaces first only)"}
            },
            "required": ["path", "old_string", "new_string"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Edit",
            "args": {"path": "string", "old_string": "string", "new_string": "string", "replace_all": "boolean?"},
            "returns": "success",
            "notes": "Applies an exact string replacement. Path aliases accepted: path, file, filepath."
        })
    }
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
    fn description(&self) -> &'static str {
        "Acquire exclusive write locks on a set of glob patterns to prevent races with sibling agents."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Admin }
    fn args_schema(&self) -> Value { json!({"type": "object"}) }
    fn legacy_schema_entry(&self) -> Value { json!({"name": "lock_paths"}) }
    fn model_facing(&self) -> bool { false }
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
    fn description(&self) -> &'static str { "Release locks acquired via lock_paths." }
    fn tier(&self) -> PermissionMode { PermissionMode::Admin }
    fn args_schema(&self) -> Value { json!({"type": "object"}) }
    fn legacy_schema_entry(&self) -> Value { json!({"name": "unlock_paths"}) }
    fn model_facing(&self) -> bool { false }
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
    fn description(&self) -> &'static str {
        "Delegate a task to another agent. Send a specific task description with clear scope and expected output."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Admin }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "target_agent_id": {"type": "string", "description": "ID of the agent to delegate to"},
                "task": {"type": "string", "description": "Task description for the target agent"}
            },
            "required": ["target_agent_id", "task"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Task",
            "args": {"target_agent_id": "string", "task": "string"},
            "returns": "{agent_outcome}",
            "notes": "Delegates a task to another agent. Subject to max delegation depth."
        })
    }
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
    fn description(&self) -> &'static str {
        "Invoke a skill by name. Returns the skill's full instructions. Use to discover and run installed skills."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Admin }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill": {"type": "string", "description": "Skill name to invoke"},
                "args": {"type": "string", "description": "Optional arguments for the skill"}
            },
            "required": ["skill"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Skill",
            "args": {"skill": "string", "args": "string?"},
            "returns": "string",
            "notes": "Invoke a skill by name. Returns the skill's full instructions. Pass optional args for the skill."
        })
    }
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
    fn description(&self) -> &'static str {
        "Launch an app-enabled skill. The skill must have an 'app' config with a launcher (web/bash/url). For web apps, returns the URL to open in the UI."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Admin }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill": {"type": "string", "description": "Name of the skill to launch"},
                "args": {"type": "string", "description": "Optional arguments for the skill"}
            },
            "required": ["skill"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "RunApp",
            "args": {"skill": "string", "args": "string?"},
            "returns": "{skill,launcher,url}",
            "notes": "Launch an app-enabled skill. The skill must have an 'app' config with a launcher (web/bash/url). For web apps, returns the URL to open."
        })
    }
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
    fn description(&self) -> &'static str {
        "Search the web via DuckDuckGo. Returns titles, URLs, and snippets."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"},
                "max_results": {"type": "integer", "description": "Maximum results (default: 5, max: 10)"}
            },
            "required": ["query"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "WebSearch",
            "args": {"query": "string", "max_results": "number?"},
            "returns": "{results:[{title,url,snippet}]}",
            "notes": "Search the web via DuckDuckGo. Default 5 results, max 10."
        })
    }
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
    fn description(&self) -> &'static str {
        "Fetch a URL and return its content as text. HTML tags are stripped. Default max 100KB."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "URL to fetch"},
                "max_bytes": {"type": "integer", "description": "Maximum bytes to return (default: 100000)"}
            },
            "required": ["url"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "WebFetch",
            "args": {"url": "string", "max_bytes": "number?"},
            "returns": "{url,content,content_type,truncated}",
            "notes": "Fetch a URL and return its content as text. HTML is stripped of tags. Default max 100KB."
        })
    }
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

#[derive(serde::Deserialize)]
struct ExpressArgs {
    #[serde(default)]
    emotion: Option<String>,
    #[serde(default)]
    action: Option<String>,
    /// An ordered list of gestures to play back-to-back as one routine.
    /// Takes precedence over `action` when present.
    #[serde(default)]
    sequence: Option<Vec<String>>,
}

/// Cap on how many gestures one Express call may chain.
const MAX_SEQUENCE: usize = 8;

/// One entry of the pet animation manifest. Only the fields the engine needs
/// to build the `Express` tool schema are deserialized; the renderer-side
/// fields (`render`, `proc`, `clips`, `visible`, `type`, …) are ignored here
/// and consumed by the web UI instead.
#[derive(serde::Deserialize)]
struct PetIntent {
    name: String,
    use_when: String,
}

/// The pet's `Express` vocabulary — the single source of truth shared with the
/// web renderer. Baked in at compile time from the UI's manifest so the tool
/// schema and the avatar can never drift; a malformed manifest fails the build.
static PET_INTENTS: LazyLock<Vec<PetIntent>> = LazyLock::new(|| {
    #[derive(serde::Deserialize)]
    struct Manifest {
        intents: Vec<PetIntent>,
    }
    let raw = include_str!("../../../ui/public/anim/actions.json");
    serde_json::from_str::<Manifest>(raw)
        .expect("ui/public/anim/actions.json must be valid")
        .intents
});

/// The mascot's body control — she shows a mood and/or a gesture on her avatar.
/// Fire-and-forget: emits a `PetExpress` event to every surface and returns
/// immediately. Carries no speech (her spoken line is just her reply text).
pub struct ExpressTool;
#[async_trait]
impl Tool for ExpressTool {
    fn name(&self) -> &'static str { "Express" }
    fn aliases(&self) -> &'static [&'static str] { &["express"] }
    fn description(&self) -> &'static str {
        "Show feeling on your avatar body: a sustained mood and/or a one-shot \
         gesture (no speech). Use sparingly and naturally — never narrate it."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn args_schema(&self) -> Value {
        let names: Vec<Value> = PET_INTENTS
            .iter()
            .map(|i| Value::String(i.name.clone()))
            .collect();
        let menu = PET_INTENTS
            .iter()
            .map(|i| format!("• {} — {}", i.name, i.use_when))
            .collect::<Vec<_>>()
            .join("\n");
        json!({
            "type": "object",
            "properties": {
                "emotion": {
                    "type": "string",
                    "enum": ["neutral", "happy", "sad", "angry", "relaxed"],
                    "description": "Sustained mood to hold until you change it."
                },
                "action": {
                    "type": "string",
                    "enum": names.clone(),
                    "description": format!(
                        "A gesture, pose, or movement. Choose by what fits the moment:\n{menu}"
                    )
                },
                "sequence": {
                    "type": "array",
                    "items": { "type": "string", "enum": names },
                    "description": "Several gestures to play back-to-back as one little routine, \
                        in order (e.g. [\"wave\", \"tilt_head\", \"shrug\"]). Use instead of `action` \
                        when one beat isn't enough. Max 8."
                }
            }
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        let names = PET_INTENTS
            .iter()
            .map(|i| i.name.as_str())
            .collect::<Vec<_>>()
            .join("|");
        json!({
            "name": "Express",
            "args": {"emotion": "string?", "action": "string?", "sequence": "string[]?"},
            "returns": "ok",
            "notes": format!(
                "Show feeling on your avatar. emotion (sustained): neutral|happy|sad|angry|relaxed. \
                 action: {names}. sequence: an ordered list of those to chain (max 8). \
                 At least one of emotion/action/sequence. Use sparingly; never narrate it."
            )
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: ExpressArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for Express: {}", e))?;

        // `sequence` (an ordered routine) takes precedence over a single `action`.
        let intents: Vec<String> = match args.sequence {
            Some(seq) if !seq.is_empty() => seq,
            _ => args.action.into_iter().collect(),
        };
        if args.emotion.is_none() && intents.is_empty() {
            anyhow::bail!("Express needs at least one of: emotion, action, sequence");
        }
        if intents.len() > MAX_SEQUENCE {
            anyhow::bail!("Express sequence too long (max {MAX_SEQUENCE})");
        }
        for name in &intents {
            if !PET_INTENTS.iter().any(|i| &i.name == name) {
                anyhow::bail!("Express: unknown action '{name}' (not in the avatar vocabulary)");
            }
        }
        // Transport the ordered intents as one comma-joined string so the
        // existing PetExpress event + spine stay unchanged; the UI splits + queues.
        let action = (!intents.is_empty()).then(|| intents.join(","));
        if let Some(manager) = tools.get_manager() {
            manager
                .send_event(
                    crate::engine::agent::AgentEvent::PetExpress {
                        emotion: args.emotion,
                        action,
                    },
                    tools.session_id.clone(),
                )
                .await;
        }
        Ok(ToolResult::Success("ok".to_string()))
    }
}

// ---------------------------------------------------------------------------
// sense — Yinyue's perception of the room (presence + work + tempo)
// ---------------------------------------------------------------------------

pub struct SenseTool;
#[async_trait]
impl Tool for SenseTool {
    fn name(&self) -> &'static str { "sense" }
    fn aliases(&self) -> &'static [&'static str] { &["Sense"] }
    fn description(&self) -> &'static str {
        "Glance at the room before you react: whether the user is here (typing), \
         present but reading, or away; how busy the day is; the hour. Your \
         perception — read it to decide whether and how to respond. Never read it aloud."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn args_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "sense",
            "args": {},
            "returns": "presence + work + tempo snapshot (JSON)",
            "notes": "Glance at the room: presence (typing|present_reading|away), how busy \
                      the day is, the hour. Your perception — decide from it; never read it aloud."
        })
    }
    async fn execute(&self, tools: &Tools, _call: ToolCall) -> Result<ToolResult> {
        let Some(manager) = tools.get_manager() else {
            return Ok(ToolResult::Success(
                json!({ "error": "no environment to sense" }).to_string(),
            ));
        };
        let now = crate::util::now_ts_secs();

        // Presence — the three-state read from the latest client beat.
        let p = manager.presence_snapshot();
        let beat_age = now.saturating_sub(p.updated_at);
        let idle = now.saturating_sub(p.last_input_at);
        let state = p.state(now);

        // Work — what the *other* agents are doing (exclude her own session so
        // she doesn't count her own glance/herald turns as the day's work).
        let own_session = tools.session_id.as_deref();
        let runs: Vec<_> = manager
            .run_store
            .list_runs(None)
            .into_iter()
            .filter(|r| Some(r.session_id.as_str()) != own_session)
            .collect();
        let active_runs = runs
            .iter()
            .filter(|r| matches!(r.status, crate::engine::agent::AgentRunStatus::Running))
            .count();
        let lt = chrono::Local::now();
        let secs_since_midnight =
            lt.hour() as u64 * 3600 + lt.minute() as u64 * 60 + lt.second() as u64;
        let today_start = now.saturating_sub(secs_since_midnight);
        let runs_today = runs.iter().filter(|r| r.started_at >= today_start).count();

        // Tempo — the hour, in human terms.
        let hour = lt.hour();
        let part_of_day = match hour {
            5..=11 => "morning",
            12..=16 => "afternoon",
            17..=21 => "evening",
            _ => "night",
        };

        let snapshot = json!({
            "presence": {
                "state": state,
                "focused": p.focused,
                "typing": p.typing,
                "idle_seconds": idle,
                "beat_age_seconds": beat_age,
            },
            "work": { "active_runs": active_runs, "runs_today": runs_today },
            "tempo": {
                "local_time": lt.format("%H:%M").to_string(),
                "hour": hour,
                "part_of_day": part_of_day,
            },
        });
        Ok(ToolResult::Success(snapshot.to_string()))
    }
}

// ---------------------------------------------------------------------------
// answer_prompt — relay the user's answer to a prompt another agent is blocked on
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct AnswerPromptArgs {
    /// The user's answer, in their own words ("approve", "deny", "the second one").
    answer: String,
    /// Which open prompt to answer; omit to answer the single open one.
    #[serde(default)]
    question_id: Option<String>,
}

pub struct AnswerPromptTool;
#[async_trait]
impl Tool for AnswerPromptTool {
    fn name(&self) -> &'static str { "answer_prompt" }
    fn aliases(&self) -> &'static [&'static str] { &["AnswerPrompt"] }
    fn description(&self) -> &'static str {
        "Relay the user's answer to a question or permission prompt another agent is \
         blocked on. Carry only what the user actually told you — never decide for them. \
         Omit question_id to answer the one open prompt."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "answer": { "type": "string", "description": "The user's answer, in their words." },
                "question_id": { "type": "string", "description": "Which open prompt; omit for the single open one." }
            },
            "required": ["answer"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "answer_prompt",
            "args": { "answer": "string", "question_id": "string?" },
            "returns": "what was relayed",
            "notes": "Relay the USER's answer to an agent's pending question/permission prompt — \
                      only the user's word, never your own decision. Omit question_id for the one open prompt."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: AnswerPromptArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for answer_prompt: {}", e))?;
        let Some(bridge) = tools.ask_user_bridge() else {
            return Ok(ToolResult::Success("there's no open prompt to answer.".to_string()));
        };

        // Resolve the target: an explicit id, else the single open prompt that
        // isn't one of Yinyue's own asks. Remove it (consuming the oneshot).
        let entry = {
            let mut pending = bridge.pending.lock().await;
            let qid = match args.question_id {
                Some(id) if pending.contains_key(&id) => Some(id),
                Some(_) => None, // stale / unknown id
                None => {
                    let mut others: Vec<String> = pending
                        .iter()
                        .filter(|(_, p)| p.agent_id != "yinyue")
                        .map(|(k, _)| k.clone())
                        .collect();
                    if others.len() == 1 { others.pop() } else { None }
                }
            };
            qid.and_then(|id| pending.remove(&id).map(|p| (id, p)))
        };
        let Some((qid, pending)) = entry else {
            return Ok(ToolResult::Success(
                "no single open prompt to answer — nothing relayed.".to_string(),
            ));
        };

        // Map the user's words onto an option of the first question when it
        // matches a label cleanly (a single match); otherwise pass it as free
        // text. Permission prompts are single-question approve/deny — a label
        // match is the norm.
        let lower = args.answer.to_lowercase();
        let matches: Vec<String> = pending
            .questions
            .first()
            .map(|q| {
                q.options
                    .iter()
                    .filter(|o| {
                        let l = o.label.to_lowercase();
                        l == lower || lower.contains(&l) || l.contains(&lower)
                    })
                    .map(|o| o.label.clone())
                    .collect()
            })
            .unwrap_or_default();
        let selected = if matches.len() == 1 { matches } else { Vec::new() };
        let custom_text = if selected.is_empty() { Some(args.answer.clone()) } else { None };

        let answers = vec![crate::engine::tools::AskUserAnswer {
            question_index: 0,
            selected: selected.clone(),
            custom_text,
        }];

        let session_id = pending.session_id.clone();
        if pending.sender.send(answers).is_err() {
            return Ok(ToolResult::Success(
                "that prompt just expired — nothing to relay.".to_string(),
            ));
        }
        // Dismiss the widget on every surface, like the normal answer path.
        let _ = bridge.events_tx.send(crate::server::ServerEvent::WidgetResolved {
            widget_id: qid,
            session_id,
        });
        let what = if selected.is_empty() { args.answer } else { selected.join(", ") };
        Ok(ToolResult::Success(format!("relayed the user's answer: {what}")))
    }
}

// ---------------------------------------------------------------------------
// agent_chat — one agent sends a one-way message to another
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct AgentChatArgs {
    /// The recipient agent's id (e.g. "yinyue").
    to: String,
    /// The message to deliver.
    message: String,
    /// Optional target app/skill (e.g. "dj"): deliver into that app's session so
    /// the recipient runs with that app's tools. Omit to use the focused session.
    #[serde(default)]
    app: Option<String>,
}

pub struct AgentChatTool;
#[async_trait]
impl Tool for AgentChatTool {
    fn name(&self) -> &'static str { "agent_chat" }
    fn aliases(&self) -> &'static [&'static str] { &["AgentChat"] }
    fn description(&self) -> &'static str {
        "Send a brief one-way message to another agent (e.g. tell Yinyue something \
         worth surfacing to the user). Fire-and-forget — if you need a reply, use Task \
         instead. You can't send if you were yourself reached via agent_chat."
    }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "to": { "type": "string", "description": "Recipient agent id, e.g. \"yinyue\" or \"ling\"." },
                "message": { "type": "string", "description": "The message to deliver." },
                "app": { "type": "string", "description": "Optional app/skill to deliver into (e.g. \"dj\"), so the recipient acts with that app's tools." }
            },
            "required": ["to", "message"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "agent_chat",
            "args": { "to": "string", "message": "string", "app": "string?" },
            "returns": "ok / why not",
            "notes": "One-way message to another agent (fire-and-forget; use Task for a reply). \
                      Refused if you were reached via agent_chat — one hop, the user re-arms it."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: AgentChatArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for agent_chat: {}", e))?;
        let from = tools.agent_id().unwrap_or("agent").to_string();
        let to = args.to.trim().to_string();
        if to.is_empty() {
            return Ok(ToolResult::Success("no recipient given — nothing sent.".to_string()));
        }
        if to == from {
            return Ok(ToolResult::Success("you can't message yourself.".to_string()));
        }
        // Loop-break: a turn that was itself woken by an agent_chat can't relay
        // onward — the chain stops at one hop; a fresh user message re-arms it.
        if let (Some(m), Some(sid)) = (tools.get_manager(), tools.session_id.as_deref()) {
            if m.is_agent_chat_session(sid) {
                return Ok(ToolResult::Success(
                    "you were reached via agent_chat — you can't pass it on without the user in \
                     the loop. If they need to act, ask them directly."
                        .to_string(),
                ));
            }
        }
        if let Some(manager) = tools.get_manager() {
            manager
                .send_event(
                    crate::engine::agent::AgentEvent::AgentChat {
                        from,
                        to: to.clone(),
                        message: args.message,
                        app: args.app.filter(|s| !s.trim().is_empty()),
                    },
                    tools.session_id.clone(),
                )
                .await;
        }
        Ok(ToolResult::Success(format!("sent to {to}.")))
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
    fn description(&self) -> &'static str {
        "Ask the user 1-4 structured questions with 2-6 options each. User can always type custom text. Blocks until response (5 min timeout)."
    }
    fn tier(&self) -> PermissionMode {
        // Pure conversation — asks act on nothing (no fs/exec/network), so
        // they sit at Chat like the Memory tools. At Read tier, a session
        // without path grants (e.g. an attended mission) hit the permission
        // ceiling and the ask was silently denied before the tool ran.
        PermissionMode::Chat
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": {"type": "string"},
                            "header": {"type": "string"},
                            "options": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": {"type": "string"},
                                        "description": {"type": "string"}
                                    },
                                    "required": ["label"]
                                }
                            },
                            "multi_select": {"type": "boolean"}
                        },
                        "required": ["question", "header", "options"]
                    }
                }
            },
            "required": ["questions"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "AskUser",
            "args": {
                "questions": "[{question: string, header: string, options: [{label: string, description?: string, preview?: string}], multi_select?: boolean}]"
            },
            "returns": "{answers: [{question_index: number, selected: string[], custom_text?: string}]}",
            "notes": "Ask user 1-4 structured questions with 2-6 options each. User can always type custom text via 'Other'. Blocks until response (5 min timeout). Not available in sub-agents."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        tools.ask_user(call.args).await
    }
}

#[cfg(test)]
mod express_tests {
    use super::*;

    /// The `Express` vocabulary is built from `ui/public/anim/actions.json` at
    /// runtime — this proves the baked-in manifest parses and every intent
    /// reaches the model-facing schema (the engine/renderer contract).
    #[test]
    fn express_vocab_loads_from_manifest() {
        let schema = ExpressTool.args_schema();
        let actions = schema["properties"]["action"]["enum"]
            .as_array()
            .expect("action enum array");
        assert_eq!(actions.len(), 43, "expected 43 intents from actions.json");

        let names: Vec<&str> = actions.iter().filter_map(|v| v.as_str()).collect();
        for expected in [
            "nod", "wave", "dance", "appear", "disappear", "walk", "run", "think", "spin", "pose",
        ] {
            assert!(names.contains(&expected), "missing intent '{expected}'");
        }

        // `sequence` chains the same vocabulary.
        let seq = schema["properties"]["sequence"]["items"]["enum"]
            .as_array()
            .expect("sequence items enum array");
        assert_eq!(seq.len(), actions.len(), "sequence vocab must match action vocab");
    }
}
