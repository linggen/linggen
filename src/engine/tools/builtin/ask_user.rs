//! AskUser.

use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

pub struct AskUserTool;
#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &'static str {
        "AskUser"
    }
    // Waits on a person. A deadline here would cancel every prompt the user
    // has not answered yet, which is the opposite of what it is for.
    fn max_duration(&self) -> Option<Duration> {
        None
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["ask_user"]
    }
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
