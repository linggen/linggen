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

/// The private app a question waiting in `session` is kept to, when this
/// call is not made from that session (`turn_place::question_kept_to`).
async fn kept_elsewhere(tools: &Tools, session: Option<&str>) -> Option<String> {
    let manager = tools.get_manager()?;
    if session.is_some() && tools.session_id.as_deref() == session {
        return None;
    }
    crate::engine::turn_place::question_kept_to(&manager, session).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_permission_relay_needs_one_exact_non_global_label() {
        let mut question = crate::engine::permission::build_exceeds_ceiling_question(
            "Read", &PermissionMode::Read, "/project/file", &["/project".into()],
        );
        assert_eq!(relay_selection(Some(&question), "  dENY ").unwrap(), ["Deny"]);
        assert!(relay_selection(Some(&question), "grant:at:read").is_err());
        assert!(relay_selection(Some(&question), "grant:all:read").is_err());
        question.permission = None;
        assert_eq!(relay_selection(Some(&question), "I said Deny").unwrap(), ["Deny"]);
    }

    #[tokio::test]
    async fn relay_requires_a_persons_turn_and_an_explicit_listing() {
        let tools = Tools::new(std::env::temp_dir()).unwrap();
        let call = ToolCall {
            tool: "answer_prompt".into(),
            args: json!({"answer": "Deny"}),
            block_id: None,
        };
        let ToolResult::Success(reason) = AnswerPromptTool.execute(&tools, call).await.unwrap() else {
            panic!("expected a relay refusal");
        };
        assert!(reason.contains("person"), "{reason}");
        assert!(super::super::super::is_listed_only("answer_prompt"));
    }
}

fn relay_selection(
    question: Option<&super::super::AskUserQuestion>,
    answer: &str,
) -> std::result::Result<Vec<String>, &'static str> {
    let Some(question) = question else { return Ok(Vec::new()) };
    let permission = question.permission.is_some();
    let lower = answer.to_lowercase();
    let matches: Vec<String> = question.options.iter().filter(|option| {
        let label = option.label.to_lowercase();
        if permission {
            label.trim() == lower.trim()
        } else {
            label == lower || lower.contains(&label) || label.contains(&lower)
        }
    }).map(|option| option.label.clone()).collect();
    if permission {
        if matches.len() != 1 {
            return Err("refused: give the exact permission option label or answer on screen.");
        }
        if matches[0].starts_with("grant:all:") {
            return Err("refused: grants for every folder must be answered on screen.");
        }
    }
    Ok(if matches.len() == 1 { matches } else { Vec::new() })
}

pub struct AnswerPromptTool;
#[async_trait]
impl Tool for AnswerPromptTool {
    fn place_reach(&self) -> crate::engine::tools::PlaceReach {
        crate::engine::tools::PlaceReach::CarriesOut
    }
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
        if !tools.person_spoke || tools.delegated_session || tools.delegation_depth() > 0 {
            return Ok(ToolResult::Success(
                "refused: only a turn started by a person can relay an answer.".into(),
            ));
        }
        let args: AnswerPromptArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for answer_prompt: {}", e))?;
        let Some(bridge) = tools.ask_user_bridge() else {
            return Ok(ToolResult::Success(
                "there's no open prompt to answer.".to_string(),
            ));
        };

        // Resolve the target: an explicit id, else the single open prompt that
        // isn't one of Yinyue's own asks.
        let target = {
            let pending = bridge.pending.lock().await;
            match args.question_id {
                Some(id) => pending.get(&id).map(|p| (id.clone(), p.session_id.clone())),
                None => {
                    let mut others: Vec<(String, Option<String>)> = pending
                        .iter()
                        .filter(|(_, p)| p.agent_id != crate::engine::agent::COMPANION_AGENT_ID)
                        .map(|(k, p)| (k.clone(), p.session_id.clone()))
                        .collect();
                    if others.len() == 1 {
                        others.pop()
                    } else {
                        None
                    }
                }
            }
        };
        if let Some(app) = kept_elsewhere(tools, target.as_ref().and_then(|t| t.1.as_deref())).await {
            return Ok(ToolResult::Success(format!(
                "refused: that question is kept to {app}; it is answered in {app}'s own chat."
            )));
        }
        // Validate the answer before consuming the waiting prompt.
        let (qid, pending, selected) = {
            let mut waiting = bridge.pending.lock().await;
            let Some((qid, pending)) = target.and_then(|(id, _)| waiting.get(&id).map(|p| (id, p))) else {
                return Ok(ToolResult::Success(
                    "no single open prompt to answer — nothing relayed.".into(),
                ));
            };
            let selected = match relay_selection(pending.questions.first(), &args.answer) {
                Ok(selected) => selected,
                Err(reason) => return Ok(ToolResult::Success(reason.into())),
            };
            let pending = waiting.remove(&qid).expect("prompt held under lock");
            (qid, pending, selected)
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
