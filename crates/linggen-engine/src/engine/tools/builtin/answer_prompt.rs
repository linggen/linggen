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
        assert_eq!(relay_selection(Some(&question), "  dENY ", None).unwrap(), ["Deny"]);
        assert!(relay_selection(Some(&question), "grant:at:read", None).is_err());
        assert!(relay_selection(Some(&question), "grant:all:read", None).is_err());
        question.permission = None;
        assert_eq!(relay_selection(Some(&question), "I said Deny", None).unwrap(), ["Deny"]);
    }

    /// A permission question with the labels "Allow once" and "Deny" (the
    /// caller's, the outward command's and a rule's have them; the person's
    /// own folder grant has "Deny" and the grants).
    fn permission_question() -> crate::engine::tools::AskUserQuestion {
        crate::engine::permission::build_caller_question(
            "Read", &PermissionMode::Read, "/project/file", &["/project".into()],
        )
    }

    /// Rule 6: on a turn the person spoke, an answer is the words heard, and
    /// they must be one option's label whole — after trimming, case and
    /// full-/half-width folding, never by a substring, never as free text.
    #[test]
    fn a_spoken_answer_must_be_one_option_whole() {
        let question = permission_question();
        let relayed = |heard: &str| relay_selection(Some(&question), "Allow once", Some(heard));
        for heard in ["Allow once", "allow once", "  ALLOW ONCE  ", "Allow once.", "Allow once!", "\u{201c}Allow once\u{201d}", "Allow  once", "Ａｌｌｏｗ　ｏｎｃｅ"] {
            assert_eq!(relayed(heard).unwrap(), ["Allow once"], "{heard:?}");
        }
        for heard in ["Deny", "deny.", "DENY。", "Ｄｅｎｙ"] {
            assert_eq!(relay_selection(Some(&question), "Deny", Some(heard)).unwrap(), ["Deny"], "{heard:?}");
        }
        // Near misses never grant: a hedge, a stock phrase the model invents from
        // a noisy room, an option inside a sentence or the sentence inside an
        // option, another word of the same stem, silence, and what is not a word.
        for heard in [
            "allow it I guess", "Thank you for watching.", "字幕由某某提供", "I said Deny", "Deny the request",
            "please allow once", "allow once or deny", "denied", "allow", "once", "yes", "", "   ", "。",
            "Allow once Deny", "允许一次",
        ] {
            assert!(relayed(heard).is_err(), "{heard:?} must not relay");
        }
    }

    /// The model's rendering of what it heard is not the words: a hedge that
    /// the model turned into an exact label still does not relay.
    #[test]
    fn a_spoken_answer_is_weighed_by_the_words_heard_not_the_models_argument() {
        let question = permission_question();
        assert!(relay_selection(Some(&question), "Allow once", Some("allow it I guess")).is_err());
        assert!(relay_selection(Some(&question), "Allow once", Some("")).is_err(), "no words, no relay");
        // And the other way: the argument cannot take back what was said exactly.
        assert_eq!(relay_selection(Some(&question), "something else", Some("Deny")).unwrap(), ["Deny"]);
    }

    /// A grant for every folder is answered on screen, spoken or typed; and a
    /// spoken answer is one option, never free text, on a question that is not a permission too.
    #[test]
    fn a_spoken_answer_never_grants_everywhere_and_is_never_free_text() {
        let grants = crate::engine::permission::build_exceeds_ceiling_question(
            "Read", &PermissionMode::Read, "/project/file", &["/project".into()],
        );
        let all = grants.options.iter().find(|o| o.label.starts_with("grant:all:")).expect("the grant question offers it");
        assert!(relay_selection(Some(&grants), "", Some(&all.label)).is_err());
        assert_eq!(relay_selection(Some(&grants), "", Some("deny")).unwrap(), ["Deny"], "its Deny can be spoken");
        let mut open = permission_question();
        open.permission = None;
        assert_eq!(relay_selection(Some(&open), "x", Some("deny")).unwrap(), ["Deny"]);
        assert!(relay_selection(Some(&open), "x", Some("I said Deny")).is_err(), "typed may; spoken may not");
        assert!(relay_selection(Some(&open), "x", Some("whatever the person said")).is_err(), "no free-text answer by voice");
        assert!(relay_selection(None, "x", Some("Deny")).is_err(), "no question to name an option of");
        // Typed, with no question on record, is as it was: the words go through.
        assert_eq!(relay_selection(None, "anything", None).unwrap(), Vec::<String>::new());
    }

    /// Through the tool itself: a turn the person spoke relays the exact
    /// option and consumes the waiting question; a near miss relays nothing
    /// and leaves the question waiting, with the reason for her to word.
    #[tokio::test]
    async fn the_tool_relays_the_heard_option_and_leaves_a_near_miss_waiting() {
        use std::sync::Arc;
        let pending = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let mut tools = Tools::new(std::env::temp_dir()).unwrap();
        tools.person_spoke = true;
        tools.set_ask_user_bridge(Arc::new(crate::engine::tools::AskUserBridge {
            events_tx: crate::engine::event_bus::EventBus::new(16),
            pending: pending.clone(),
            session_id: None,
        }));
        let wait = || {
            let (sender, receiver) = tokio::sync::oneshot::channel();
            let entry = crate::engine::tools::PendingAskUser {
                agent_id: "ling".into(),
                questions: vec![permission_question()],
                sender,
                session_id: None,
                run_id: None,
                caller_request: None,
            };
            (entry, receiver)
        };
        let relay = |tools: &Tools, answer: &str| {
            let call = ToolCall { tool: "answer_prompt".into(), args: json!({ "answer": answer, "question_id": "q1" }), block_id: None };
            let tools = tools.clone();
            async move {
                let ToolResult::Success(said) = AnswerPromptTool.execute(&tools, call).await.unwrap() else { panic!("a text result") };
                said
            }
        };

        let (entry, mut receiver) = wait();
        pending.lock().await.insert("q1".into(), entry);
        tools.heard_aloud = Some("allow it I guess".into());
        let said = relay(&tools, "Allow once").await;
        assert!(said.starts_with("refused:"), "{said}");
        assert!(pending.lock().await.contains_key("q1"), "the question still waits");
        assert!(receiver.try_recv().is_err(), "nothing was relayed");

        tools.heard_aloud = Some("Allow once.".into());
        let said = relay(&tools, "anything the model made of it").await;
        assert!(said.contains("Allow once"), "{said}");
        let answers = receiver.await.unwrap();
        assert_eq!((answers[0].selected.clone(), answers[0].custom_text.clone()), (vec!["Allow once".to_string()], None));
        assert!(!pending.lock().await.contains_key("q1"));
    }

    /// The words are folded in one place: trimmed of edge marks and space,
    /// full-width made half-width, cased down, inner spaces made one.
    #[test]
    fn heard_words_are_folded_in_one_place() {
        assert_eq!(folded("  Ａllow\u{3000}ONCE。 "), "allow once");
        assert_eq!(folded("\u{201c}Deny!\u{201d}"), "deny");
        assert_eq!(folded("允许一次，"), "允许一次");
        assert_eq!(folded(" . "), "");
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

/// The one place that decides which option of a waiting question an answer
/// relays — and whether it relays at all.
///
/// `heard`: the words the person SPOKE to start this turn, when they did.
/// A spoken answer is weighed by those words and by nothing the model made of
/// them: they must be exactly one option's label (see [`folded`]) — never part
/// of a longer sentence, never free text, never a grant for every folder — or
/// nothing is relayed and she says what she heard. A typed answer is as it
/// always was: a permission question wants one exact label, any other
/// question takes a label by substring or the words as custom text.
fn relay_selection(
    question: Option<&super::super::AskUserQuestion>,
    answer: &str,
    heard: Option<&str>,
) -> std::result::Result<Vec<String>, &'static str> {
    if let Some(words) = heard {
        return relay_heard(question, words);
    }
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

/// A spoken answer: the heard `words` name exactly one option, or nothing is relayed.
fn relay_heard(
    question: Option<&super::super::AskUserQuestion>,
    words: &str,
) -> std::result::Result<Vec<String>, &'static str> {
    const NOT_EXACT: &str = "refused: what was heard is not exactly one of the options. Tell the person what you heard \
                             and that this answer must be typed or tapped on screen; relay nothing.";
    let heard = folded(words);
    let named: Vec<&str> = question
        .into_iter()
        .flat_map(|q| &q.options)
        .map(|option| option.label.as_str())
        .filter(|label| !heard.is_empty() && folded(label) == heard)
        .collect();
    match named.as_slice() {
        [label] if label.starts_with("grant:all:") => {
            Err("refused: grants for every folder must be answered on screen.")
        }
        [label] => Ok(vec![(*label).to_string()]),
        _ => Err(NOT_EXACT),
    }
}

/// Words as they are weighed against a label: edge spaces and marks trimmed
/// (a clip's text ends "Allow once." or "Deny。"), full-width letters made
/// half-width, lower case, inner runs of space made one. Edges only: a mark
/// inside the words stays, so "allow, once" is not "allow once".
fn folded(text: &str) -> String {
    let half_width: String = text
        .chars()
        .map(|c| match c {
            '\u{3000}' => ' ',
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            other => other,
        })
        .collect();
    half_width
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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
            let selected = match relay_selection(pending.questions.first(), &args.answer, tools.heard_aloud.as_deref()) {
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
