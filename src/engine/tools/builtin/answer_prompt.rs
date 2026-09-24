//! Answering for someone: answer_prompt.

use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

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
    fn name(&self) -> &'static str {
        "answer_prompt"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["AnswerPrompt"]
    }
    fn description(&self) -> &'static str {
        "Relay the user's answer to a question or permission prompt another agent is \
         blocked on. Carry only what the user actually told you — never decide for them. \
         Omit question_id to answer the one open prompt."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
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
            return Ok(ToolResult::Success(
                "there's no open prompt to answer.".to_string(),
            ));
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
                        .filter(|(_, p)| p.agent_id != crate::engine::agent::COMPANION_AGENT_ID)
                        .map(|(k, _)| k.clone())
                        .collect();
                    if others.len() == 1 {
                        others.pop()
                    } else {
                        None
                    }
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
        let selected = if matches.len() == 1 {
            matches
        } else {
            Vec::new()
        };
        let custom_text = if selected.is_empty() {
            Some(args.answer.clone())
        } else {
            None
        };

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
        let _ = bridge
            .events_tx
            .send(crate::engine::events::ServerEvent::WidgetResolved {
                widget_id: qid,
                session_id,
            });
        let what = if selected.is_empty() {
            args.answer
        } else {
            selected.join(", ")
        };
        Ok(ToolResult::Success(format!(
            "relayed the user's answer: {what}"
        )))
    }
}
