//! SendMessage: a running subagent gets a message from the agent that
//! started it — or from it, its parent — at its next step, as Claude Code's
//! SendMessage reaches a subagent by name. The session's own agent reaches
//! its background subagents from any later turn: they are its by name.
use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct SendMessageTool;

#[async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &'static str {
        "SendMessage"
    }

    fn description(&self) -> &'static str {
        "Send a message to a running subagent you started (by its task id from Task or TaskList, or its run id), or to your own parent run. It reads it at its next step; this does not wait for a reply."
    }

    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }

    fn args_schema(&self) -> Value {
        json!({"type":"object", "properties":{
            "to":{"type":"string", "description":"Task id or run id of the running subagent, or your parent's run id"},
            "message":{"type":"string"}
        }, "required":["to","message"]})
    }

    fn legacy_schema_entry(&self) -> Value {
        json!({"name":"SendMessage", "args":{"to":"string", "message":"string"}, "returns":"string"})
    }

    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let to = call.args["to"]
            .as_str()
            .or_else(|| call.args["run_id"].as_str())
            .filter(|s| !s.is_empty())
            .context("to is required")?;
        let message = call.args["message"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .context("message is required")?;
        let manager = tools.get_manager().context("No agent manager")?;
        let session = tools.session_id.as_deref().context("No session")?;
        // A background subagent of this caller, named by its task or its run.
        let owned = manager
            .background
            .list_owned(session, tools.task_owner())
            .into_iter()
            .find(|t| t.kind == "subagent" && (t.id == to || t.subagent_id.as_deref() == Some(to)));
        if let Some(task) = owned {
            let running = task.is_running();
            let run = task
                .subagent_id
                .filter(|_| running)
                .with_context(|| format!("{to} is not running"))?;
            let from = tools.agent_id.as_deref().unwrap_or("your parent");
            if !manager.queue_run_notice(&run, format!("Message from {from}: {message}")) {
                anyhow::bail!("{to} has finished taking messages");
            }
            return Ok(ToolResult::Success(format!("Message queued for {to}.")));
        }
        let sender = tools.run_id.as_deref().context("No running sender")?;
        manager.send_run_message(session, sender, to, message)?;
        Ok(ToolResult::Success(format!("Message queued for {to}.")))
    }
}
