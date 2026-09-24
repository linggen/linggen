//! Handing work on: Task, Skill, RunApp.

use super::super::delegation::{RunAppArgs, SkillArgs, TaskArgs};
use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

pub struct TaskTool;
#[async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &'static str {
        "Task"
    }
    // A delegated subagent run — open-ended by nature.
    fn max_duration(&self) -> Option<Duration> {
        None
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["delegate_to_agent"]
    }
    fn description(&self) -> &'static str {
        "Delegate a task to another agent. Send a specific task description with clear scope and expected output."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Admin
    }
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
    fn name(&self) -> &'static str {
        "Skill"
    }
    // A skill defines its own work, and plenty of it runs for minutes.
    fn max_duration(&self) -> Option<Duration> {
        None
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["skill"]
    }
    fn description(&self) -> &'static str {
        "Invoke a skill by name. Returns the skill's full instructions. Use to discover and run installed skills."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Admin
    }
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
    fn name(&self) -> &'static str {
        "RunApp"
    }
    // Hands off to an app; the run does not own its lifetime.
    fn max_duration(&self) -> Option<Duration> {
        None
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["run_app"]
    }
    fn description(&self) -> &'static str {
        "Launch an app-enabled skill. The skill must have an 'app' config with a launcher (web/bash/url). For web apps, returns the URL to open in the UI."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Admin
    }
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
