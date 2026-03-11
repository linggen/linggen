use super::types::*;
use crate::config::AgentPolicyCapability;
use crate::engine::patch::validate_unified_diff;
use crate::ollama::ChatMessage;
use tracing::{info, warn};

impl AgentEngine {
    pub(crate) async fn handle_patch_action(
        &mut self,
        diff: String,
        messages: &mut Vec<ChatMessage>,
    ) -> LoopControl {
        info!("Patch proposed");
        if !self.agent_allows_policy(AgentPolicyCapability::Patch) {
            warn!("Patch blocked: agent lacks Patch policy");
            self.push_context_record(
                ContextType::Error,
                Some("patch_not_allowed".to_string()),
                self.agent_id.clone(),
                None,
                "Agent policy does not allow Patch.".to_string(),
                serde_json::json!({
                    "required_policy": "Patch",
                    "agent": self.agent_id.clone(),
                }),
            );
            messages.push(self.tool_result_msg(
                self.prompt_store.render_or_fallback(
                    crate::prompts::keys::PATCH_NOT_ALLOWED,
                    &[],
                ),
            ));
            return LoopControl::Continue;
        }
        let errs = validate_unified_diff(&diff);
        if !errs.is_empty() {
            warn!("Patch invalid: {} errors", errs.len());
            self.push_context_record(
                ContextType::Error,
                Some("patch_validation".to_string()),
                self.agent_id.clone(),
                None,
                errs.join("\n"),
                serde_json::json!({ "error_count": errs.len() }),
            );
            messages.push(self.tool_result_msg(
                self.prompt_store.render_or_fallback(
                    crate::prompts::keys::PATCH_VALIDATION_FAILED,
                    &[("errors", &errs.join("\n"))],
                ),
            ));
            return LoopControl::Continue;
        }

        info!("Patch validated OK");

        self.active_skill = None;
        LoopControl::Return(AgentOutcome::Patch(diff))
    }

    /// Called when the model signals plan completion (via ExitPlanMode tool or
    /// fallback: done in plan_mode). Emits a PlanUpdate SSE event so the
    /// PlanBlock renders in the UI, and returns `AgentOutcome::Plan` for the
    /// server to store as pending. The user reviews and approves via PlanBlock
    /// buttons (CC-aligned — no modal AskUser dialog).
    pub(crate) async fn finalize_plan_mode(&mut self, plan_text: String) -> AgentOutcome {
        let summary = Self::extract_plan_summary(&plan_text);
        let plan = Plan {
            summary,
            status: PlanStatus::Planned,
            plan_text,
            items: Vec::new(),
        };
        self.persist_and_emit_plan(plan.clone()).await;
        AgentOutcome::Plan(plan)
    }

    /// Store the plan in memory and emit a PlanUpdate SSE event.
    pub(crate) async fn persist_and_emit_plan(&mut self, plan: Plan) {
        self.plan = Some(plan);

        if let Some(manager) = self.tools.get_manager() {
            let agent_id = self
                .agent_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let plan = self.plan.clone().unwrap();
            manager
                .send_event(crate::agent_manager::AgentEvent::PlanUpdate {
                    agent_id,
                    plan,
                })
                .await;
        }
    }

    /// Extract a summary from the plan text (first heading or first non-empty line).
    pub(crate) fn extract_plan_summary(text: &str) -> String {
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("# ") {
                return trimmed.strip_prefix("# ").unwrap_or(trimmed).to_string();
            }
            if !trimmed.is_empty() {
                return trimmed.chars().take(80).collect();
            }
        }
        "Plan".to_string()
    }

    pub(crate) async fn handle_finalize_action(
        &mut self,
        packet: TaskPacket,
        _messages: &mut Vec<ChatMessage>,
        session_id: Option<&str>,
    ) -> LoopControl {
        info!("Task finalized: {}", packet.title);
        // Persist the structured final answer to session files for the UI.
        let msg = serde_json::json!({ "type": "finalize_task", "packet": packet }).to_string();
        let _ = self
            .persist_assistant_message(&msg, session_id)
            .await;
        self.chat_history.push(ChatMessage::new("assistant", msg.clone()));
        self.push_context_record(
            ContextType::AssistantReply,
            Some("finalize_task".to_string()),
            self.agent_id.clone(),
            Some("user".to_string()),
            msg,
            serde_json::json!({ "kind": "finalize_task" }),
        );
        self.active_skill = None;
        LoopControl::Return(AgentOutcome::Task(packet))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_plan_summary_examples() {
        assert_eq!(
            AgentEngine::extract_plan_summary("# My Plan\n\nSome details"),
            "My Plan"
        );
        assert_eq!(
            AgentEngine::extract_plan_summary("First line without heading"),
            "First line without heading"
        );
        assert_eq!(AgentEngine::extract_plan_summary(""), "Plan");
        assert_eq!(AgentEngine::extract_plan_summary("   \n  "), "Plan");
    }
}
