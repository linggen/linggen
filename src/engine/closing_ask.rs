//! The closing question (`closing-ask` in SKILL.md): a skill whose tools know
//! what the player must choose next hands that question back in every result
//! as `ask`. The model is told to ask it, and usually does — when it ends a
//! turn on words alone, the engine asks it instead, through AskUser itself,
//! so a turn of such a skill never ends without a way forward. The engine
//! reads only the shape; it never learns what the question means.
//! See `doc/skill-spec.md` § Closing question.

use super::tools::{AskUserAnswer, ToolResult};
use super::types::{AgentEngine, LoopState};
use super::{AgentOutcome, ModelAction};
use crate::message::{ChatMessage, ToolCallFunction, ToolCallMessage};
use anyhow::Result;
use serde_json::{json, Value as JsonValue};
use tracing::info;

/// What asking the closing question came to.
pub(crate) enum ClosingAsk {
    /// Nothing waited to be asked — the turn ends as it would have.
    NotNeeded,
    /// Asked and answered: the loop goes on, the answer is the model's next input.
    Answered,
    /// Asked, never answered (skipped, timed out): the run ends here.
    Unanswered,
    /// The ask itself ended the run (a cancel on the way).
    Exit(AgentOutcome),
}

/// The AskUser arguments a tool's output hands the turn, when it carries a
/// top-level `ask` = `{header, question, options: [{label, …}]}`. Only the
/// labels travel, and only a question AskUser can take (2–6 options).
pub(crate) fn ask_from_output(output: &str) -> Option<JsonValue> {
    let trimmed = output.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    let value: JsonValue = serde_json::from_str(trimmed).ok()?;
    let ask = value.get("ask")?;
    let question = ask.get("question")?.as_str()?.trim();
    if question.is_empty() {
        return None;
    }
    let header = ask.get("header").and_then(JsonValue::as_str).unwrap_or("");
    let labels: Vec<&str> = ask
        .get("options")?
        .as_array()?
        .iter()
        .filter_map(|o| o.get("label").and_then(JsonValue::as_str))
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if !(2..=6).contains(&labels.len()) {
        return None;
    }
    Some(json!({
        "questions": [{
            "header": header,
            "question": question,
            "options": labels.iter().map(|l| json!({ "label": l })).collect::<Vec<_>>(),
        }]
    }))
}

/// A tool's result as text the closing question can be read from.
fn output_text(result: &ToolResult) -> Option<&str> {
    match result {
        ToolResult::CommandOutput { stdout, .. } => Some(stdout),
        ToolResult::Success(text) => Some(text),
        _ => None,
    }
}

/// True when AskUser came back with a choice or typed words — not a skip,
/// a timeout or a cancel.
pub(crate) fn answered(result: &ToolResult) -> bool {
    let ToolResult::AskUserResponse { answers } = result else {
        return false;
    };
    answers.iter().any(answer_given)
}

fn answer_given(a: &AskUserAnswer) -> bool {
    a.selected.iter().any(|s| !s.trim().is_empty())
        || a.custom_text
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty())
}

impl AgentEngine {
    /// After every tool: an AskUser settles the question (and records whether
    /// it was answered); a tool of a `closing-ask` skill that hands back an
    /// `ask` makes it the turn's question.
    pub(crate) fn note_closing_ask(&mut self, tool: &str, result: &ToolResult) {
        if tool == "AskUser" {
            self.closing_ask = None;
            self.last_ask_answered = answered(result);
            return;
        }
        let declared = self
            .active_skill
            .as_ref()
            .is_some_and(|s| s.closing_ask && s.tool_defs.iter().any(|t| t.name == tool));
        if !declared {
            return;
        }
        if let Some(ask) = output_text(result).and_then(ask_from_output) {
            self.closing_ask = Some(ask);
        }
    }

    /// The model ended its turn on `reply` without asking the question its
    /// skill's tool handed it: ask it now, as the model would have — the call
    /// joins the transcript, the widget and its answer take the usual road.
    /// Each question is asked at most once. `done_ids` are the filtered
    /// `Done` calls of a Done-only turn, which get their results first.
    pub(crate) async fn try_closing_ask(
        &mut self,
        state: &mut LoopState,
        session_id: Option<&str>,
        reply: &str,
        native: bool,
        done_ids: &[String],
    ) -> Result<ClosingAsk> {
        let Some(args) = self.closing_ask.take() else {
            return Ok(ClosingAsk::NotNeeded);
        };
        if self.tools.builtins.ask_user_bridge().is_none() {
            return Ok(ClosingAsk::NotNeeded);
        }
        if let Some(allowed) = &state.allowed_tools {
            if !self.is_tool_allowed(allowed, "AskUser") {
                return Ok(ClosingAsk::NotNeeded);
            }
        }
        info!(
            "[{}] closing-ask: the turn ended without its question — the engine asks it",
            self.run_id.as_deref().unwrap_or("root")
        );

        let call_id = format!("call_{}", uuid::Uuid::new_v4().simple());
        let tc_ids = if native {
            for id in done_ids {
                let done = ChatMessage::tool_result_named(id.clone(), "Done", "ok".to_string());
                self.push_tracked_message(&mut state.messages, done);
            }
            let call = ToolCallMessage {
                id: call_id.clone(),
                call_type: "function".to_string(),
                function: ToolCallFunction {
                    name: "AskUser".to_string(),
                    arguments: args.clone(),
                },
                thought_signature: None,
            };
            let mut msg = ChatMessage::assistant_with_tool_calls(vec![call]);
            if done_ids.is_empty() {
                msg.content = reply.to_string();
            }
            self.push_tracked_message(&mut state.messages, msg);
            vec![call_id]
        } else {
            let msg = ChatMessage::new("assistant", reply.to_string());
            self.push_tracked_message(&mut state.messages, msg);
            Vec::new()
        };

        self.last_ask_answered = false;
        let action = ModelAction {
            tool: "AskUser".to_string(),
            args,
        };
        if let Some(outcome) = self
            .execute_action_loop(vec![action], state, session_id, &tc_ids)
            .await?
        {
            return Ok(ClosingAsk::Exit(outcome));
        }
        Ok(if self.last_ask_answered {
            ClosingAsk::Answered
        } else {
            ClosingAsk::Unanswered
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_hands_over_its_question_labels_only() {
        let out = r#"{"then":"…","ask":{"header":"流波山 · 山根","question":"何去何从？","options":[{"label":"读封 · 填东方","exit":"seal"},{"label":"问问银月","ask":true}]},"ok":true}"#;
        let args = ask_from_output(&format!("\n{out}\n")).unwrap();
        let q = &args["questions"][0];
        assert_eq!(q["header"], "流波山 · 山根");
        assert_eq!(q["question"], "何去何从？");
        assert_eq!(
            q["options"],
            json!([{ "label": "读封 · 填东方" }, { "label": "问问银月" }])
        );
    }

    #[test]
    fn no_question_askuser_cannot_take() {
        assert!(ask_from_output("ok").is_none());
        assert!(ask_from_output(r#"{"ok":true}"#).is_none());
        assert!(ask_from_output(
            r#"{"ask":{"question":"","options":[{"label":"a"},{"label":"b"}]}}"#
        )
        .is_none());
        assert!(ask_from_output(r#"{"ask":{"question":"Q","options":[{"label":"a"}]}}"#).is_none());
        let seven: Vec<_> = (0..7)
            .map(|i| json!({ "label": format!("o{i}") }))
            .collect();
        let out = json!({ "ask": { "question": "Q", "options": seven } }).to_string();
        assert!(ask_from_output(&out).is_none());
        // No header is still a question; the widget shows none.
        let bare =
            ask_from_output(r#"{"ask":{"question":"Q","options":[{"label":"a"},{"label":"b"}]}}"#);
        assert_eq!(bare.unwrap()["questions"][0]["header"], "");
    }

    #[test]
    fn only_a_choice_or_words_count_as_an_answer() {
        let with = |selected: Vec<&str>, custom: Option<&str>| ToolResult::AskUserResponse {
            answers: vec![AskUserAnswer {
                question_index: 0,
                selected: selected.into_iter().map(String::from).collect(),
                custom_text: custom.map(String::from),
            }],
        };
        assert!(answered(&with(vec!["离"], None)));
        assert!(answered(&with(vec![], Some("说说今日卦象"))));
        assert!(!answered(&with(vec![], None)));
        assert!(!answered(&with(vec![], Some("  "))));
        assert!(!answered(&ToolResult::Success("timed out".into())));
    }
}
