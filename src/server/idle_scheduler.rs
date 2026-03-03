use crate::server::{ServerEvent, ServerState};
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tracing::{debug, info, warn};

/// How often the scheduler checks for idle agents (seconds).
const CHECK_INTERVAL_SECS: u64 = 10;

/// Maximum idle triggers per mission to prevent runaway cost.
const MAX_IDLE_TRIGGERS_PER_MISSION: u64 = 100;

/// Background loop that checks for idle agents and dispatches idle_prompts.
///
/// When a mission is active for a project and an agent has an idle_prompt configured,
/// this loop will dispatch the idle_prompt as a chat message to that agent after
/// the configured idle_interval has elapsed since the agent's last activity.
pub async fn idle_scheduler_loop(state: Arc<ServerState>) {
    let mut interval = time::interval(Duration::from_secs(CHECK_INTERVAL_SECS));
    let mut trigger_counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

    loop {
        interval.tick().await;

        let projects = match state.manager.store.list_projects() {
            Ok(p) => p,
            Err(e) => {
                debug!("Idle scheduler: failed to list projects: {}", e);
                continue;
            }
        };

        for project in &projects {
            let project_path = &project.path;

            // Check if this project has an active mission
            let mission = match state.manager.store.get_mission(project_path) {
                Ok(Some(m)) if m.active => m,
                _ => continue,
            };

            let mission_key = format!("{}|{}", project_path, mission.created_at);

            // Check trigger cap
            let count = trigger_counts.get(&mission_key).copied().unwrap_or(0);
            if count >= MAX_IDLE_TRIGGERS_PER_MISSION {
                debug!(
                    "Idle scheduler: mission '{}' hit trigger cap ({}), skipping",
                    mission_key, MAX_IDLE_TRIGGERS_PER_MISSION
                );
                continue;
            }

            // Get all agent specs for this project
            let root = std::path::PathBuf::from(project_path);
            let agent_specs = match state.manager.list_agent_specs(&root).await {
                Ok(specs) => specs,
                Err(_) => continue,
            };

            for spec_entry in &agent_specs {
                let agent_id = &spec_entry.agent_id;

                // Get effective idle config (merges mission -> persisted override -> markdown defaults)
                let (idle_prompt, idle_interval) = state
                    .manager
                    .get_effective_idle_config(&root, agent_id)
                    .await;

                let Some(prompt) = idle_prompt else {
                    continue; // No idle_prompt = reactive agent, skip
                };

                // Check if agent has been idle long enough
                let idle_duration = state
                    .manager
                    .get_agent_idle_duration(project_path, agent_id)
                    .await;

                if idle_duration < Duration::from_secs(idle_interval) {
                    continue; // Not idle long enough yet
                }

                // Check if agent is currently busy (try_lock fails = busy)
                let agent = match state.manager.get_or_create_agent(&root, agent_id).await {
                    Ok(a) => a,
                    Err(_) => continue,
                };

                if agent.try_lock().is_err() {
                    debug!(
                        "Idle scheduler: agent '{}' is busy, skipping idle trigger",
                        agent_id
                    );
                    continue; // Agent is busy, skip
                }

                // Dispatch idle prompt
                info!(
                    "Idle scheduler: triggering idle_prompt for agent '{}' in project '{}'",
                    agent_id, project_path
                );

                // Record activity now to prevent re-triggering before the run completes
                state
                    .manager
                    .update_agent_activity(project_path, agent_id)
                    .await;

                // Increment trigger count
                *trigger_counts.entry(mission_key.clone()).or_insert(0) += 1;

                // Emit SSE event for UI observability
                let _ = state.events_tx.send(ServerEvent::IdlePromptTriggered {
                    agent_id: agent_id.clone(),
                    project_root: project_path.clone(),
                });

                // Construct the idle message
                let idle_message = state.prompt_store.render_or_fallback(
                    crate::prompts::keys::IDLE_MESSAGE_FORMAT,
                    &[("mission", &mission.text), ("prompt", &prompt)],
                );

                // Dispatch using the same code path as chat_handler
                let state_clone = state.clone();
                let agent_id_owned = agent_id.clone();
                let project_path_owned = project_path.clone();
                let root_owned = root.clone();

                tokio::spawn(async move {
                    dispatch_idle_prompt(
                        state_clone,
                        root_owned,
                        &project_path_owned,
                        &agent_id_owned,
                        &idle_message,
                    )
                    .await;
                });
            }
        }

        // Clean up trigger counts for inactive missions
        let active_missions: std::collections::HashSet<String> = projects
            .iter()
            .filter_map(|p| {
                state
                    .manager
                    .store
                    .get_mission(&p.path)
                    .ok()
                    .flatten()
                    .filter(|m| m.active)
                    .map(|m| format!("{}|{}", p.path, m.created_at))
            })
            .collect();
        trigger_counts.retain(|k, _| active_missions.contains(k));
    }
}

/// Dispatch an idle prompt to an agent, using the same execution path as chat_handler.
async fn dispatch_idle_prompt(
    state: Arc<ServerState>,
    root: std::path::PathBuf,
    project_path: &str,
    agent_id: &str,
    message: &str,
) {
    use crate::server::AgentStatusKind;

    let agent = match state.manager.get_or_create_agent(&root, agent_id).await {
        Ok(a) => a,
        Err(e) => {
            warn!(
                "Idle scheduler: failed to get agent '{}': {}",
                agent_id, e
            );
            return;
        }
    };

    // Try to lock the agent — if busy, skip silently
    let Ok(mut engine) = agent.try_lock() else {
        debug!(
            "Idle scheduler: agent '{}' became busy before dispatch",
            agent_id
        );
        return;
    };

    let manager = state.manager.clone();
    let events_tx = state.events_tx.clone();

    // Begin a run record
    let run_id = match manager
        .begin_agent_run(&root, None, agent_id, None, Some("idle_prompt".to_string()))
        .await
    {
        Ok(id) => id,
        Err(e) => {
            warn!("Idle scheduler: failed to begin run for '{}': {}", agent_id, e);
            return;
        }
    };

    // Persist and emit the idle prompt as a system message
    crate::server::chat_helpers::persist_and_emit_message(
        &manager,
        &events_tx,
        &root,
        agent_id,
        "system",
        agent_id,
        message,
        None,
        false,
    )
    .await;

    state
        .send_agent_status(
            agent_id.to_string(),
            AgentStatusKind::Working,
            Some("Processing idle prompt".to_string()),
            None,
        )
        .await;

    // Set up the engine for the idle prompt run
    engine.observations.clear();
    engine.task = Some(message.to_string());
    engine.set_parent_agent(None);
    engine.set_run_id(Some(run_id.clone()));
    let result = engine.run_agent_loop(None).await;
    engine.set_run_id(None);

    match result {
        Ok(outcome) => {
            let _ = manager
                .finish_agent_run(
                    &run_id,
                    crate::project_store::AgentRunStatus::Completed,
                    None,
                )
                .await;
            let _ = events_tx.send(ServerEvent::Outcome {
                agent_id: agent_id.to_string(),
                outcome,
            });
        }
        Err(err) => {
            let msg = err.to_string();
            let status = if msg.to_lowercase().contains("cancel") {
                crate::project_store::AgentRunStatus::Cancelled
            } else {
                crate::project_store::AgentRunStatus::Failed
            };
            let _ = manager.finish_agent_run(&run_id, status, Some(msg)).await;
        }
    }

    state
        .send_agent_status(
            agent_id.to_string(),
            AgentStatusKind::Idle,
            Some("Idle".to_string()),
            None,
        )
        .await;

    // Update activity timestamp
    manager
        .update_agent_activity(project_path, agent_id)
        .await;
}
