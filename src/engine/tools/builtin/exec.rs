//! Bash.

use super::super::search_exec::RunCommandArgs;
use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

pub struct BashTool;
#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &'static str {
        "Bash"
    }
    // The user asked for this command; builds and downloads run long, and the
    // shell layer bounds itself.
    fn max_duration(&self) -> Option<Duration> {
        None
    }
    fn description(&self) -> &'static str {
        "Run a shell command via sh -c. Working directory persists across calls (cd is remembered). Use for build, test, git, and other commands that require shell execution. Prefer dedicated tools (Read, Glob, Grep) over Bash equivalents."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Admin
    }
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
