use crate::config::AgentPolicyCapability;
use crate::server::{AgentStatusKind, ServerEvent, ServerState};
use crate::state_fs::StateFile;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

async fn agent_allows_policy(
    state: &Arc<ServerState>,
    root: &PathBuf,
    agent_id: &str,
    capability: AgentPolicyCapability,
) -> bool {
    match state.manager.list_agent_specs(root).await {
        Ok(entries) => entries
            .into_iter()
            .find(|entry| entry.agent_id.eq_ignore_ascii_case(agent_id))
            .map(|entry| entry.spec.allows_policy(capability))
            .unwrap_or(false),
        Err(_) => false,
    }
}

async fn first_patch_agent(state: &Arc<ServerState>, root: &PathBuf) -> Option<String> {
    let entries = state.manager.list_agent_specs(root).await.ok()?;
    entries
        .into_iter()
        .find(|entry| entry.spec.allows_policy(AgentPolicyCapability::Patch))
        .map(|entry| entry.agent_id)
}

#[derive(Deserialize)]
pub(crate) struct TaskRequest {
    project_root: String,
    agent_id: String,
    task: String,
}

pub(crate) async fn set_task(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<TaskRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);

    match state
        .manager
        .get_or_create_agent(&root, &req.agent_id)
        .await
    {
        Ok(agent) => {
            let mut engine = agent.lock().await;
            engine.set_task(req.task.clone());

            // Persist planning task when this main agent is allowed to finalize.
            if agent_allows_policy(
                &state,
                &root,
                &req.agent_id,
                AgentPolicyCapability::Finalize,
            )
            .await
            {
                if let Ok(ctx) = state.manager.get_or_create_project(root).await {
                    let planning_task = StateFile::PmTask {
                        id: format!("plan-{}", crate::util::now_ts_secs()),
                        status: "active".to_string(),
                        assigned_tasks: Vec::new(),
                    };
                    let _ = ctx
                        .state_fs
                        .write_file("active.md", &planning_task, &req.task);
                    let _ = state.events_tx.send(ServerEvent::StateUpdated);
                }
            }

            StatusCode::OK.into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct RunRequest {
    project_root: String,
    agent_id: String,
    session_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CancelRunRequest {
    run_id: String,
}

#[derive(Serialize)]
struct CancelRunResponse {
    cancelled_run_ids: Vec<String>,
}

pub(crate) async fn run_agent(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<RunRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);
    let agent_id = req.agent_id.clone();
    let session_id = req.session_id.clone();
    let events_tx = state.events_tx.clone();
    let manager = state.manager.clone();
    let state_clone = state.clone();

    match state
        .manager
        .get_or_create_agent(&root, &req.agent_id)
        .await
    {
        Ok(agent) => {
            tokio::spawn(async move {
                let run_id = match manager
                    .begin_agent_run(
                        &root,
                        session_id.as_deref(),
                        &agent_id,
                        None,
                        Some("api/run".to_string()),
                    )
                    .await
                {
                    Ok(id) => id,
                    Err(_) => format!("run-{}-fallback", agent_id),
                };
                state_clone
                    .send_agent_status(
                        agent_id.clone(),
                        AgentStatusKind::Working,
                        Some("Running".to_string()),
                        None,
                    )
                    .await;
                let mut engine = agent.lock().await;
                engine.set_parent_agent(None);
                engine.set_run_id(Some(run_id.clone()));
                let run_result = engine.run_agent_loop(session_id.as_deref()).await;
                engine.set_run_id(None);
                let outcome = match run_result {
                    Ok(outcome) => {
                        let _ = manager
                            .finish_agent_run(
                                &run_id,
                                crate::project_store::AgentRunStatus::Completed,
                                None,
                            )
                            .await;
                        outcome
                    }
                    Err(err) => {
                        let msg = err.to_string();
                        let status = if msg.to_lowercase().contains("cancel") {
                            crate::project_store::AgentRunStatus::Cancelled
                        } else {
                            crate::project_store::AgentRunStatus::Failed
                        };
                        let _ = manager.finish_agent_run(&run_id, status, Some(msg)).await;
                        crate::engine::AgentOutcome::None
                    }
                };

                // If a Finalize-capable agent finalized a task, persist and queue for Patch-capable main agent.
                let is_finalize_agent = agent_allows_policy(
                    &state_clone,
                    &root,
                    &agent_id,
                    AgentPolicyCapability::Finalize,
                )
                .await;
                if is_finalize_agent {
                    if let crate::engine::AgentOutcome::Task(packet) = &outcome {
                        if let Ok(ctx) = manager.get_or_create_project(root.clone()).await {
                            let assignee = first_patch_agent(&state_clone, &root)
                                .await
                                .unwrap_or_else(|| "unassigned".to_string());
                            let task_id = format!("task-{}", crate::util::now_ts_secs());
                            let coder_task = StateFile::CoderTask {
                                id: task_id.clone(),
                                status: "queued".to_string(),
                                story_id: None,
                                assigned_to: assignee,
                            };
                            let body = format!(
                                "## {}\n\n### User Stories\n{}\n\n### Acceptance Criteria\n{}",
                                packet.title,
                                packet.user_stories.join("\n"),
                                packet.acceptance_criteria.join("\n")
                            );
                            let _ = ctx.state_fs.write_file(
                                &format!("tasks/{}.md", task_id),
                                &coder_task,
                                &body,
                            );
                            let _ = events_tx.send(ServerEvent::StateUpdated);
                        }
                    }
                }

                let _ = events_tx.send(ServerEvent::Outcome {
                    agent_id: agent_id.clone(),
                    outcome,
                });
                state_clone
                    .send_agent_status(
                        agent_id.clone(),
                        AgentStatusKind::Idle,
                        Some("Idle".to_string()),
                        None,
                    )
                    .await;
            });

            Json(serde_json::json!({ "status": "started" })).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct CancelToolRequest {
    block_id: String,
}

pub(crate) async fn cancel_tool_execution(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CancelToolRequest>,
) -> impl IntoResponse {
    let triggered = state.manager.trigger_tool_cancel(&req.block_id);
    Json(serde_json::json!({
        "status": if triggered { "cancelled" } else { "not_found" }
    }))
}

pub(crate) async fn cancel_agent_run(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CancelRunRequest>,
) -> impl IntoResponse {
    match state.manager.cancel_run_tree(&req.run_id).await {
        Ok(runs) => {
            // Cancel any pending AskUser questions for the cancelled agents so
            // the tool unblocks immediately. Dropping the sender causes the
            // oneshot receiver to return Err, which is handled gracefully.
            {
                let cancelled_agents: std::collections::HashSet<String> =
                    runs.iter().map(|r| r.agent_id.clone()).collect();
                let mut pending = state.pending_ask_user.lock().await;
                pending.retain(|_, entry| !cancelled_agents.contains(&entry.agent_id));
            }

            for run in &runs {
                state
                    .send_agent_status(
                        run.agent_id.clone(),
                        AgentStatusKind::Idle,
                        Some("Cancelled".to_string()),
                        None,
                    )
                    .await;
            }
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            Json(CancelRunResponse {
                cancelled_run_ids: runs.into_iter().map(|r| r.run_id).collect(),
            })
            .into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

