//! TaskOutput and TaskStop: a native agent's hold on the background tasks
//! it started (doc/background-tasks-spec.md). Only this session's tasks.

use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::background;
use crate::engine::permission::PermissionMode;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

fn task_id(call: &ToolCall) -> Result<String> {
    call.args["task_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .context("task_id is required")
}

fn scope(tools: &Tools) -> Result<(std::sync::Arc<crate::engine::agent::AgentManager>, String)> {
    let manager = tools.get_manager().context("No background tasks here")?;
    let session = tools.session_id.clone().context("No background tasks outside a session")?;
    Ok((manager, session))
}

pub struct TaskListTool;
#[async_trait]
impl Tool for TaskListTool {
    fn name(&self) -> &'static str { "TaskList" }
    fn description(&self) -> &'static str { "List background tasks you started in this session." }
    fn tier(&self) -> PermissionMode { PermissionMode::Read }
    fn args_schema(&self) -> Value { json!({"type":"object", "properties":{}}) }
    fn legacy_schema_entry(&self) -> Value { json!({"name":"TaskList", "args":{}, "returns":"string"}) }
    async fn execute(&self, tools: &Tools, _call: ToolCall) -> Result<ToolResult> {
        let (manager, session) = scope(tools)?;
        let tasks = manager.background.list_owned(&session, tools.task_owner());
        Ok(ToolResult::Success(serde_json::to_string(&tasks)?))
    }
}

pub struct TaskOutputTool;
#[async_trait]
impl Tool for TaskOutputTool {
    fn name(&self) -> &'static str {
        "TaskOutput"
    }
    fn description(&self) -> &'static str {
        "Check one of this session's background tasks: its status, how it ended, and the last lines of its output (a shell's)."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string", "description": "The task id Bash or Task returned"},
                "lines": {"type": "integer", "description": "Output lines to show (default 50)"}
            },
            "required": ["task_id"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "TaskOutput",
            "args": {"task_id": "string", "lines": "number?"},
            "returns": "string",
            "notes": "Status and output tail of a background task of this session."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let id = task_id(&call)?;
        let (manager, session) = scope(tools)?;
        let (task, result) = manager.background.get_owned(&session, tools.task_owner(), &id).context("No such task in your scope")?;
        let lines = call.args["lines"].as_u64().unwrap_or(50).clamp(1, 500) as usize;
        let mut text = format!("Task {} ({}): {}", task.id, task.kind, task.status);
        if let Some(summary) = &task.summary {
            text.push_str(&format!(" — {summary}"));
        }
        text.push_str(&format!("\n{}", task.title));
        if let Some(path) = background::output_of(&task) {
            let tail = background::tail(&path, lines);
            text.push_str(&format!("\nOutput ({}):\n{}", path.display(), if tail.is_empty() { "(none yet)" } else { &tail }));
        }
        if let Some(result) = result { text.push_str(&format!("\nResult:\n{result}")); }
        Ok(ToolResult::Success(text))
    }
}

pub struct TaskStopTool;
#[async_trait]
impl Tool for TaskStopTool {
    fn name(&self) -> &'static str {
        "TaskStop"
    }
    fn description(&self) -> &'static str {
        "Stop one of this session's background tasks. You are told in a new turn once it has ended."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"task_id": {"type": "string", "description": "The task id Bash or Task returned"}},
            "required": ["task_id"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "TaskStop",
            "args": {"task_id": "string"},
            "returns": "string",
            "notes": "Stops a background task of this session."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let id = task_id(&call)?;
        let (manager, session) = scope(tools)?;
        manager.background.stop_owned(&session, tools.task_owner(), &id)?;
        Ok(ToolResult::Success(format!("Stopping task {id}; you will be told once it has ended.")))
    }
}
