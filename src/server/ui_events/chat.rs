//! A conversation's own events: messages, streamed tokens, content blocks,
//! questions and hints — everything drawn inside one session's chat.

use super::{
    Ui, UI_KIND_CONTENT_BLOCK, UI_KIND_MESSAGE, UI_KIND_TEXT_SEGMENT, UI_KIND_TOKEN,
    UI_KIND_TURN_COMPLETE, UI_PHASE_DONE,
};
use crate::engine::events::{ServerEvent, UiEvent};
use serde_json::json;

pub(super) fn map(event: ServerEvent, ui: Ui) -> Option<UiEvent> {
    let seq = ui.seq;
    match event {
        ServerEvent::Message {
            from,
            to,
            content,
            session_id,
            run_id,
            parent_agent_id,
        } => {
            let cleaned = crate::engine::tool_render::sanitize_message_for_ui(&from, &content)?;
            Some(
                ui.event(format!("msg-{seq}"), UI_KIND_MESSAGE)
                    .text(cleaned)
                    .agent(from.clone())
                    .session(session_id)
                    .data(json!({
                        "from": from,
                        "to": to,
                        "role": if from == "user" { "user" } else { "assistant" },
                        // Subagent routing keys — present only when emitted
                        // from a delegated engine. handleMessage in the UI
                        // uses these to route the bubble into SubagentPane
                        // instead of the parent's main chat (the gap that
                        // was leaking "ENCODED encoded=0" into chat).
                        "run_id": run_id,
                        "parent_agent_id": parent_agent_id,
                    })),
            )
        }
        ServerEvent::Token {
            agent_id,
            token,
            done,
            thinking,
            session_id,
        } => {
            let mut e = ui
                .event(format!("token-{agent_id}-{seq}"), UI_KIND_TOKEN)
                .text(token)
                .agent(agent_id)
                .session(session_id);
            if done {
                e = e.phase(UI_PHASE_DONE);
            }
            if thinking {
                e = e.data(json!({ "thinking": true }));
            }
            Some(e)
        }
        ServerEvent::TextSegment {
            agent_id,
            text,
            parent_id,
            session_id,
        } => Some(
            ui.event(format!("text-seg-{agent_id}-{seq}"), UI_KIND_TEXT_SEGMENT)
                .text(text)
                .agent(agent_id)
                .session(session_id)
                .data(json!({ "parent_id": parent_id })),
        ),
        ServerEvent::ContentBlockStart {
            agent_id,
            block_id,
            block_type,
            tool,
            args,
            parent_id,
            session_id,
            run_id,
            parent_run_id,
        } => Some(
            ui.event(format!("cb-start-{block_id}"), UI_KIND_CONTENT_BLOCK)
                .phase("start")
                .agent(agent_id)
                .session(session_id)
                .data(json!({
                    "block_id": block_id,
                    "block_type": block_type,
                    "tool": tool,
                    "args": args,
                    "parent_id": parent_id,
                    "run_id": run_id,
                    "parent_run_id": parent_run_id,
                })),
        ),
        ServerEvent::ContentBlockUpdate {
            agent_id,
            block_id,
            status,
            summary,
            is_error,
            parent_id,
            extra,
            session_id,
            run_id,
            parent_run_id,
        } => {
            let mut data = json!({
                "block_id": block_id,
                "status": status,
                "summary": summary,
                "is_error": is_error,
                "parent_id": parent_id,
                "run_id": run_id,
                "parent_run_id": parent_run_id,
            });
            // Merge extra fields into the data object so the frontend receives them flat.
            if let (Some(base), Some(ext)) = (
                data.as_object_mut(),
                extra.as_ref().and_then(|e| e.as_object()),
            ) {
                for (k, v) in ext {
                    base.insert(k.clone(), v.clone());
                }
            }
            let mut e = ui
                .event(format!("cb-update-{block_id}-{seq}"), UI_KIND_CONTENT_BLOCK)
                .phase("update")
                .agent(agent_id)
                .session(session_id)
                .data(data);
            e.text = summary;
            Some(e)
        }
        ServerEvent::TurnComplete {
            agent_id,
            duration_ms,
            context_tokens,
            parent_id,
            session_id,
            run_id,
            parent_run_id,
        } => Some(
            ui.event(
                format!("turn-complete-{agent_id}-{seq}"),
                UI_KIND_TURN_COMPLETE,
            )
            .agent(agent_id)
            .session(session_id)
            .data(json!({
                "duration_ms": duration_ms,
                "context_tokens": context_tokens,
                "parent_id": parent_id,
                "run_id": run_id,
                "parent_run_id": parent_run_id,
            })),
        ),
        ServerEvent::AskUser {
            agent_id,
            question_id,
            questions,
            session_id,
        } => Some(
            ui.event(format!("ask-user-{question_id}"), "ask_user")
                .agent(agent_id)
                .session(session_id)
                .data(json!({
                    "question_id": question_id,
                    "questions": questions,
                })),
        ),
        ServerEvent::WidgetResolved {
            widget_id,
            session_id,
        } => Some(
            ui.event(format!("resolved-{widget_id}"), "widget_resolved")
                .session(session_id)
                .data(json!({ "widget_id": widget_id })),
        ),
        ServerEvent::Followups {
            agent_id,
            items,
            run_id,
            session_id,
        } => Some(
            ui.event(format!("followups-{agent_id}-{seq}"), "followups")
                .agent(agent_id)
                .session(session_id)
                .data(json!({ "items": items, "run_id": run_id })),
        ),
        ServerEvent::ModelFallback {
            agent_id,
            preferred_model,
            actual_model,
            reason,
            session_id,
        } => Some(
            ui.event(format!("model-fallback-{agent_id}-{seq}"), "model_fallback")
                .text(format!(
                    "Using {} model ({} unavailable: {})",
                    actual_model, preferred_model, reason
                ))
                .agent(agent_id)
                .session(session_id)
                .data(json!({
                    "preferred_model": preferred_model,
                    "actual_model": actual_model,
                    "reason": reason,
                })),
        ),
        ServerEvent::ToolProgress {
            agent_id,
            tool,
            line,
            stream,
            session_id,
        } => Some(
            ui.event(format!("tool-progress-{agent_id}-{seq}"), "tool_progress")
                .text(line.clone())
                .agent(agent_id)
                .session(session_id)
                .data(json!({
                    "tool": tool,
                    "line": line,
                    "stream": stream,
                })),
        ),
        _ => None,
    }
}
