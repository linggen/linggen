//! One agent's one-way message to another.

use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

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
    fn name(&self) -> &'static str {
        "agent_chat"
    }
    // Waits on another agent to answer.
    fn max_duration(&self) -> Option<Duration> {
        None
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["AgentChat"]
    }
    fn description(&self) -> &'static str {
        "Send a brief one-way message to another agent (e.g. tell Yinyue something \
         worth surfacing to the user). Fire-and-forget — if you need a reply, use Task \
         instead. You can't send if you were yourself reached via agent_chat."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
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
            return Ok(ToolResult::Success(
                "no recipient given — nothing sent.".to_string(),
            ));
        }
        if to == from {
            return Ok(ToolResult::Success(
                "you can't message yourself.".to_string(),
            ));
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
