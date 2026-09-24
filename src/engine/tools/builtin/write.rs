//! File writes: Write, Edit, and path locks.

use super::super::write_tools::{EditFileArgs, LockPathsArgs, UnlockPathsArgs, WriteFileArgs};
use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct WriteTool;
#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &'static str {
        "Write"
    }
    fn description(&self) -> &'static str {
        "Write content to a file (creates or overwrites). Prefer Edit for existing files. Path is relative to workspace root."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Edit
    }
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
    fn name(&self) -> &'static str {
        "Edit"
    }
    fn description(&self) -> &'static str {
        "Apply an exact string replacement in a file. Prefer this over Write for existing files. Read the file first."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Edit
    }
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
    fn name(&self) -> &'static str {
        "lock_paths"
    }
    fn description(&self) -> &'static str {
        "Acquire exclusive write locks on a set of glob patterns to prevent races with sibling agents."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Admin
    }
    fn args_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({"name": "lock_paths"})
    }
    fn model_facing(&self) -> bool {
        false
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: LockPathsArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for lock_paths: {}", e))?;
        tools.lock_paths(args).await
    }
}

pub struct UnlockPathsTool;
#[async_trait]
impl Tool for UnlockPathsTool {
    fn name(&self) -> &'static str {
        "unlock_paths"
    }
    fn description(&self) -> &'static str {
        "Release locks acquired via lock_paths."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Admin
    }
    fn args_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({"name": "unlock_paths"})
    }
    fn model_facing(&self) -> bool {
        false
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: UnlockPathsArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for unlock_paths: {}", e))?;
        tools.unlock_paths(args).await
    }
}
