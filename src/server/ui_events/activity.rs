//! What an agent is doing and how its run is going: status lines, the
//! queue, context usage, subagents, plans, outcomes, resyncs.

use super::{
    Ui, UI_KIND_ACTIVITY, UI_KIND_QUEUE, UI_KIND_RUN, UI_PHASE_CONTEXT_USAGE, UI_PHASE_DOING,
    UI_PHASE_DONE, UI_PHASE_OUTCOME, UI_PHASE_PLAN_UPDATE, UI_PHASE_RESYNC,
    UI_PHASE_SUBAGENT_RESULT, UI_PHASE_SUBAGENT_SPAWNED, UI_PHASE_SYNC,
};
use crate::engine::events::{AgentStatusKind, ServerEvent, UiEvent};
use super::data::{
    ActivityData, ContextUsageData, OutcomeData, PlanUpdateData, QueueData, ResyncData,
    SubagentResultData, SubagentSpawnedData,
};

fn default_status_text(status: AgentStatusKind) -> String {
    match status {
        AgentStatusKind::ModelLoading => "Model loading...".to_string(),
        AgentStatusKind::Thinking => "Thinking...".to_string(),
        AgentStatusKind::CallingTool => "Calling tool...".to_string(),
        AgentStatusKind::Working => "Working...".to_string(),
        AgentStatusKind::Idle => "Idle".to_string(),
    }
}

pub(super) fn map(event: ServerEvent, ui: Ui) -> Option<UiEvent> {
    let seq = ui.seq;
    match event {
        ServerEvent::AgentStatus {
            agent_id,
            status,
            detail,
            status_id,
            lifecycle,
            parent_agent_id,
            session_id,
            run_id,
            parent_run_id,
        } => {
            let data = |status: &str| ActivityData {
                status: status.to_string(),
                parent_id: parent_agent_id.clone(),
                run_id: run_id.clone(),
                parent_run_id: parent_run_id.clone(),
            };
            let idle = status.eq_ignore_ascii_case("idle");
            if idle && lifecycle.is_none() {
                // Still emit the idle event so the UI can transition agent status.
                return Some(
                    ui.event(format!("act-{seq}"), UI_KIND_ACTIVITY)
                        .phase(UI_PHASE_DONE)
                        .agent(agent_id)
                        .session(session_id)
                        .data(data("idle")),
                );
            }
            let phase = lifecycle
                .unwrap_or_else(|| if idle { UI_PHASE_DONE } else { UI_PHASE_DOING }.to_string());
            let text = detail
                .map(|v| v.trim().to_string())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| default_status_text(AgentStatusKind::from_str_loose(&status)));
            let id = status_id.unwrap_or_else(|| format!("activity-{agent_id}-{status}-{seq}"));
            Some(
                ui.event(id, UI_KIND_ACTIVITY)
                    .phase(&phase)
                    .text(text)
                    .agent(agent_id)
                    .session(session_id)
                    .data(data(&status)),
            )
        }
        ServerEvent::QueueUpdated {
            project_root,
            session_id,
            agent_id,
            items,
        } => Some(
            ui.event(
                format!("queue-{project_root}|{session_id}|{agent_id}"),
                UI_KIND_QUEUE,
            )
            .text(format!(
                "Queued {} message{}",
                items.len(),
                if items.len() == 1 { "" } else { "s" }
            ))
            .agent(agent_id)
            .session(Some(session_id))
            .project_root(Some(project_root))
            .data(QueueData { items }),
        ),
        ServerEvent::StateUpdated => Some(
            ui.event(format!("run-sync-{seq}"), UI_KIND_RUN)
                .phase(UI_PHASE_SYNC)
                .text("State updated")
                .global(),
        ),
        ServerEvent::Outcome {
            agent_id,
            outcome,
            session_id,
        } => Some(
            ui.event(format!("run-outcome-{agent_id}-{seq}"), UI_KIND_RUN)
                .phase(UI_PHASE_OUTCOME)
                .text("Run outcome")
                .agent(agent_id)
                .session(session_id)
                .data(OutcomeData { outcome }),
        ),
        ServerEvent::ContextUsage {
            agent_id,
            stage,
            message_count,
            char_count,
            estimated_tokens,
            token_limit,
            actual_prompt_tokens,
            actual_completion_tokens,
            compressed,
            summary_count,
            session_id,
        } => Some(
            ui.event(format!("run-context-{agent_id}"), UI_KIND_RUN)
                .phase(UI_PHASE_CONTEXT_USAGE)
                .agent(agent_id.clone())
                .session(session_id)
                .data(ContextUsageData {
                    agent_id,
                    stage,
                    message_count,
                    char_count,
                    estimated_tokens,
                    token_limit,
                    actual_prompt_tokens,
                    actual_completion_tokens,
                    compressed,
                    summary_count,
                }),
        ),
        ServerEvent::SubagentSpawned {
            parent_id,
            subagent_id,
            task,
            session_id,
            subagent_run_id,
            parent_run_id,
        } => Some(
            ui.event(
                format!(
                    "run-subagent-spawned-{}-{seq}",
                    subagent_run_id.as_deref().unwrap_or(&subagent_id)
                ),
                UI_KIND_RUN,
            )
            .phase(UI_PHASE_SUBAGENT_SPAWNED)
            .text(format!("Spawned subagent {}", subagent_id))
            .agent(parent_id)
            .session(session_id)
            .data(SubagentSpawnedData {
                subagent_id,
                task,
                subagent_run_id,
                parent_run_id,
            }),
        ),
        ServerEvent::SubagentResult {
            parent_id,
            subagent_id,
            outcome,
            session_id,
            subagent_run_id,
            parent_run_id,
        } => Some(
            ui.event(
                format!(
                    "run-subagent-result-{}-{seq}",
                    subagent_run_id.as_deref().unwrap_or(&subagent_id)
                ),
                UI_KIND_RUN,
            )
            .phase(UI_PHASE_SUBAGENT_RESULT)
            .text(format!("Subagent {} returned", subagent_id))
            .agent(parent_id)
            .session(session_id)
            .data(SubagentResultData {
                subagent_id,
                outcome,
                subagent_run_id,
                parent_run_id,
            }),
        ),
        ServerEvent::PlanUpdate {
            agent_id,
            plan,
            session_id,
        } => Some(
            ui.event(format!("run-plan-{agent_id}-{seq}"), UI_KIND_RUN)
                .phase(UI_PHASE_PLAN_UPDATE)
                .text("Plan updated")
                .agent(agent_id)
                .session(session_id)
                .data(PlanUpdateData { plan }),
        ),
        ServerEvent::Resync {
            reason,
            lagged_count,
        } => Some(
            ui.event(format!("run-resync-{seq}"), UI_KIND_RUN)
                .phase(UI_PHASE_RESYNC)
                .text("Resync required")
                .global()
                .data(ResyncData {
                    reason,
                    lagged_count,
                }),
        ),
        _ => None,
    }
}
