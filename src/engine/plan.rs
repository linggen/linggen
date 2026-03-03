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
    /// fallback: done in plan_mode). Extracts the plan text, persists it, and
    /// returns the Plan outcome for user approval.
    pub(crate) async fn finalize_plan_mode(&mut self, plan_text: String) -> AgentOutcome {
        let summary = Self::extract_plan_summary(&plan_text);
        let plan = Plan {
            summary,
            status: PlanStatus::Planned,
            plan_text,
        };
        self.persist_and_emit_plan(plan).await;
        AgentOutcome::Plan(self.plan.clone().unwrap())
    }

    /// Persist the plan to self.plan + plan file, and emit a PlanUpdate SSE event.
    pub(crate) async fn persist_and_emit_plan(&mut self, plan: Plan) {
        self.write_plan_file(&plan);
        self.plan = Some(plan);

        if let Some(manager) = self.tools.get_manager() {
            let agent_id = self
                .agent_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            // Clone from self.plan for the event — avoids double clone.
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

    // -----------------------------------------------------------------------
    // Per-session plan file persistence
    // -----------------------------------------------------------------------

    pub(crate) fn write_plan_file(&self, plan: &Plan) {
        let Some(dir) = &self.session_plan_dir else { return };
        let _ = std::fs::create_dir_all(dir);
        let path = dir.join("plan.md");
        let _ = std::fs::write(&path, &plan.plan_text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_test_engine(tmp: &std::path::Path) -> AgentEngine {
        let model_manager = Arc::new(
            crate::agent_manager::models::ModelManager::new(vec![]),
        );
        AgentEngine::new(
            EngineConfig {
                ws_root: tmp.to_path_buf(),
                max_iters: 1,
                write_safety_mode: crate::config::WriteSafetyMode::Off,
                tool_permission_mode: crate::config::ToolPermissionMode::Auto,
                prompt_loop_breaker: None,
            },
            model_manager,
            "test".to_string(),
            AgentRole::Coder,
        )
        .unwrap()
    }

    #[test]
    fn write_plan_file_free_form() {
        let tmp = tempfile::tempdir().unwrap();
        let mut engine = make_test_engine(tmp.path());
        let session_dir = tmp.path().join("sessions").join("s2");
        engine.session_plan_dir = Some(session_dir.clone());

        let plan_text = "# Add avatar upload\n\n1. Add endpoint\n2. Add model\n";
        let plan = Plan {
            summary: "Add avatar upload".to_string(),
            status: PlanStatus::Planned,
            plan_text: plan_text.to_string(),
        };

        engine.write_plan_file(&plan);

        let plan_path = session_dir.join("plan.md");
        assert!(plan_path.exists());
        let content = std::fs::read_to_string(&plan_path).unwrap();
        assert_eq!(content, plan_text);
    }

    #[test]
    fn write_plan_file_no_dir_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let engine = make_test_engine(tmp.path());
        // session_plan_dir is None — should not panic or write anything

        let plan = Plan {
            summary: "Test".to_string(),
            status: PlanStatus::Executing,
            plan_text: String::new(),
        };

        engine.write_plan_file(&plan);
        // No assertion needed — just verify no panic
    }

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
