use super::block_on_async;
use super::{AskUserBridge, ToolResult, Tools};
use crate::agent_manager::AgentManager;
use crate::config::AgentPolicyCapability;
use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub(crate) struct TaskArgs {
    pub(crate) target_agent_id: String,
    pub(crate) task: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct WebSearchArgs {
    #[serde(alias = "q")]
    pub(super) query: String,
    pub(super) max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WebFetchArgs {
    pub(super) url: String,
    pub(super) max_bytes: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SkillArgs {
    #[serde(alias = "name")]
    pub(super) skill: String,
    pub(super) args: Option<String>,
}

impl Tools {
    /// Validate delegation policy/depth/target without executing.
    /// Returns the manager and caller agent id on success.
    pub(crate) fn validate_delegation(
        &self,
        args: &TaskArgs,
    ) -> Result<(Arc<AgentManager>, String)> {
        let manager = self
            .manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Delegation requires AgentManager context"))?;
        let caller_id = self
            .agent_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Delegation requires caller agent id"))?;

        if self.delegation_depth >= self.max_delegation_depth {
            anyhow::bail!(
                "Delegation denied: max delegation depth ({}) reached",
                self.max_delegation_depth
            );
        }
        let policy = self
            .agent_policy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Delegation denied: missing agent policy"))?;
        if !policy.allows(AgentPolicyCapability::Delegate) {
            anyhow::bail!(
                "Delegation denied: agent '{}' policy does not allow Delegate",
                caller_id
            );
        }
        if !policy.allows_delegate_target(&args.target_agent_id) {
            let allowed = if policy.delegate_targets.is_empty() {
                "(none)".to_string()
            } else {
                policy.delegate_targets.join(", ")
            };
            anyhow::bail!(
                "Delegation denied: target '{}' is not allowed by policy for '{}'. Allowed: {}",
                args.target_agent_id,
                caller_id,
                allowed
            );
        }

        Ok((manager.clone(), caller_id))
    }

    pub(super) fn task(&self, args: TaskArgs) -> Result<ToolResult> {
        let (manager, caller_id) = self.validate_delegation(&args)?;
        let delegation_depth = self.delegation_depth;
        let max_delegation_depth = self.max_delegation_depth;
        let ws_root = self.root.clone();
        let parent_run_id = self.run_id.clone();
        let session_id = self.session_id.clone();

        let ask_bridge = self.ask_user_bridge.clone();
        block_on_async(run_delegation(
            manager,
            ws_root,
            caller_id,
            args.target_agent_id,
            args.task,
            parent_run_id,
            delegation_depth,
            max_delegation_depth,
            ask_bridge,
            session_id,
        ))
    }

    pub(super) fn invoke_skill(&self, args: SkillArgs) -> Result<ToolResult> {
        let manager = self
            .manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Skill tool requires AgentManager context"))?;

        let skill = block_on_async(manager.skill_manager.get_skill(&args.skill));

        match skill {
            Some(skill) => {
                if skill.disable_model_invocation {
                    anyhow::bail!(
                        "Skill '{}' is user-only (disable_model_invocation). It can only be invoked by the user via /{}.",
                        args.skill, args.skill
                    );
                }
                let mut content = format!(
                    "<skill-name>{}</skill-name>\n\n{}\n\n{}",
                    skill.name, skill.description, skill.content
                );
                if let Some(ref extra_args) = args.args {
                    content.push_str(&format!("\n\nSkill arguments: {}", extra_args));
                }
                Ok(ToolResult::Success(content))
            }
            None => {
                let available = block_on_async(manager.skill_manager.list_skills());
                let names: Vec<String> = available
                    .iter()
                    .filter(|s| !s.disable_model_invocation)
                    .map(|s| s.name.clone())
                    .collect();
                anyhow::bail!(
                    "Skill '{}' not found. Available skills: [{}]",
                    args.skill,
                    names.join(", ")
                );
            }
        }
    }
}

/// Execute a single delegation on a fresh, ephemeral engine.
///
/// This is a standalone async function (not a method) so it can be spawned onto
/// a `JoinSet` for parallel execution.  Each call creates its own `AgentEngine`
/// via `AgentManager::spawn_delegation_engine`, runs the agent loop, and drops
/// the engine when done.
pub(crate) async fn run_delegation(
    manager: Arc<AgentManager>,
    ws_root: PathBuf,
    caller_id: String,
    target_agent_id: String,
    task: String,
    parent_run_id: Option<String>,
    delegation_depth: usize,
    max_delegation_depth: usize,
    ask_user_bridge: Option<Arc<AskUserBridge>>,
    session_id: Option<String>,
) -> Result<ToolResult> {
    let run_id = manager
        .begin_agent_run(
            &ws_root,
            None,
            &target_agent_id,
            parent_run_id,
            Some(format!("delegated by {}", caller_id)),
        )
        .await?;

    manager
        .send_event(crate::agent_manager::AgentEvent::Message {
            from: caller_id.clone(),
            to: target_agent_id.clone(),
            content: format!("Delegated task: {}", task),
        })
        .await;

    manager
        .send_event(crate::agent_manager::AgentEvent::SubagentSpawned {
            parent_id: caller_id.clone(),
            subagent_id: target_agent_id.clone(),
            task: task.clone(),
        })
        .await;

    let engine_result = manager
        .spawn_delegation_engine(&ws_root, &target_agent_id)
        .await;
    let mut engine = match engine_result {
        Ok(e) => e,
        Err(err) => {
            let _ = manager
                .finish_agent_run(
                    &run_id,
                    crate::project_store::AgentRunStatus::Failed,
                    Some(err.to_string()),
                )
                .await;
            return Err(err);
        }
    };

    engine.set_parent_agent(Some(caller_id.clone()));
    engine.set_delegation_depth(delegation_depth + 1, max_delegation_depth);
    engine.set_run_id(Some(run_id.clone()));
    engine.set_task(task);
    engine.tools.builtins.set_session_id(session_id.clone());

    // Wire AskUser bridge so the subagent can prompt for permissions and user questions.
    if let Some(bridge) = ask_user_bridge {
        engine.tools.set_ask_user_bridge(bridge);
    }

    let run_result = engine.run_agent_loop(session_id.as_deref()).await;
    // Capture sub-agent's last response before engine is dropped.
    let last_text = engine.last_assistant_text.take();

    let (outcome, status, detail) = match run_result {
        Ok(outcome) => (outcome, crate::project_store::AgentRunStatus::Completed, None),
        Err(err) => {
            let msg = err.to_string();
            let status = if msg.to_lowercase().contains("cancel") {
                crate::project_store::AgentRunStatus::Cancelled
            } else {
                crate::project_store::AgentRunStatus::Failed
            };
            let _ = manager
                .finish_agent_run(&run_id, status, Some(msg.clone()))
                .await;
            return Err(err);
        }
    };
    let _ = manager.finish_agent_run(&run_id, status, detail).await;

    manager
        .send_event(crate::agent_manager::AgentEvent::SubagentResult {
            parent_id: caller_id,
            subagent_id: target_agent_id,
            outcome: outcome.clone(),
        })
        .await;

    // When the sub-agent finished normally (AgentOutcome::None), surface its
    // last response text so the parent agent sees the actual result instead
    // of "agent_outcome: None".
    match outcome {
        crate::engine::AgentOutcome::None => {
            let text = last_text.unwrap_or_else(|| "Sub-agent completed.".to_string());
            Ok(ToolResult::Success(text))
        }
        other => Ok(ToolResult::AgentOutcome(other)),
    }
}
