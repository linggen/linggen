use crate::engine::agent::AgentManager;
use crate::engine::{AgentEngine, Plan, PlanStatus};
use crate::server::chat::helpers::{
    emit_outcome_event, persist_and_emit_message, persist_message_only,
};
use crate::server::{ServerEvent, ServerState};
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

use super::runtime::{
    persist_and_emit_last_assistant_text, push_user_turn_with_recall, run_loop_with_tracking,
    send_thinking_status, spawn_thinking_forwarder, unwire_interrupt_channel, wire_ask_user_bridge,
    wire_interrupt_channel,
};
use super::types::{EditPlanRequest, PlanActionRequest};
use super::ChatRunCtx;
use crate::server::AgentStatusKind;

/// Dispatch plan mode: agent researches codebase and produces a structured plan (read-only).
pub(super) async fn run_plan_dispatch(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
) {
    send_thinking_status(ctx, "Planning").await;

    // Extract task from "/plan <task>" prefix or use full message.
    let task_text = ctx
        .clean_msg
        .strip_prefix("/plan ")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ctx.clean_msg.trim());

    engine.plan_mode = true;
    engine.plan = None;
    engine.observations.clear();
    engine.task = Some(task_text.to_string());
    // Forward images so the plan-mode sub-loop can see them (pending_images
    // was consumed by std::mem::take in the previous loop's prepare_loop_messages).
    if engine.pending_images.is_empty() && !ctx.images.is_empty() {
        engine.pending_images = ctx.images.clone();
    }
    push_user_turn_with_recall(ctx, engine).await;

    let (thinking_tx, thinking_rx) = tokio::sync::mpsc::unbounded_channel();
    engine.thinking_tx = Some(thinking_tx);

    let interrupt_key = wire_interrupt_channel(ctx, engine).await;
    wire_ask_user_bridge(&ctx.state, engine, ctx.session_id.clone());

    spawn_thinking_forwarder(
        thinking_rx,
        ctx.events_tx.clone(),
        ctx.agent_id.clone(),
        ctx.session_id.clone(),
    );

    let outcome = run_loop_with_tracking(
        &ctx.manager,
        &ctx.root,
        engine,
        &ctx.agent_id,
        ctx.session_id.as_deref(),
        "chat:plan",
        &ctx.events_tx,
    )
    .await;

    engine.thinking_tx = None;
    unwire_interrupt_channel(ctx, engine, &interrupt_key).await;
    engine.plan_mode = false;

    match outcome {
        Ok(ref out) => {
            // Skip emitting the raw plan text as a regular message — the plan
            // content reaches the UI via the PlanUpdate SSE event instead.
            // Emitting here would hide the PlanBlock widget behind a duplicate
            // text bubble.
            if !matches!(out, crate::engine::AgentOutcome::Plan(_)) {
                persist_and_emit_last_assistant_text(ctx, engine).await;
            }
            if let crate::engine::AgentOutcome::Plan(ref plan) = out {
                let plan_json = serde_json::json!({ "type": "plan", "plan": plan }).to_string();
                persist_message_only(
                    &ctx.manager, &ctx.root, &ctx.agent_id,
                    &ctx.agent_id, "user", &plan_json,
                    ctx.session_id.as_deref(), false,
                ).await;
            }
            emit_outcome_event(out, &ctx.events_tx, &ctx.agent_id, ctx.session_id.as_deref());
            if let crate::engine::AgentOutcome::Plan(ref plan) = out {
                ctx.manager
                    .set_pending_plan(
                        &ctx.root.to_string_lossy(),
                        &ctx.agent_id,
                        ctx.session_id.as_deref(),
                        plan.clone(),
                    )
                    .await;
            }
        }
        Err(err) => {
            let error_msg = super::helpers::format_turn_error(&err.to_string());
            persist_and_emit_message(
                &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                &ctx.agent_id, "user", &error_msg, ctx.session_id.as_deref(), false,
            )
            .await;
        }
    }
    let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
}

/// Run the execution loop for an approved plan. Wires thinking/interrupt channels,
/// runs the loop, and emits outcome events. Engine must already have plan + task set.
pub(super) async fn run_plan_execution(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
) {
    let (thinking_tx, thinking_rx) = tokio::sync::mpsc::unbounded_channel();
    engine.thinking_tx = Some(thinking_tx);
    let interrupt_key = wire_interrupt_channel(ctx, engine).await;
    wire_ask_user_bridge(&ctx.state, engine, ctx.session_id.clone());

    spawn_thinking_forwarder(
        thinking_rx,
        ctx.events_tx.clone(),
        ctx.agent_id.clone(),
        ctx.session_id.clone(),
    );

    send_thinking_status(ctx, "Executing plan").await;

    let exec_outcome = run_loop_with_tracking(
        &ctx.manager, &ctx.root, engine, &ctx.agent_id,
        ctx.session_id.as_deref(), "chat:plan-execution", &ctx.events_tx,
    )
    .await;

    engine.thinking_tx = None;
    unwire_interrupt_channel(ctx, engine, &interrupt_key).await;

    match exec_outcome {
        Ok(ref out) => {
            emit_outcome_event(out, &ctx.events_tx, &ctx.agent_id, ctx.session_id.as_deref());
            // Done (AgentOutcome::None): emit last_assistant_text so the UI
            // shows the completion summary.
            if matches!(out, crate::engine::AgentOutcome::None) {
                if let Some(text) = &engine.last_assistant_text {
                    if !text.is_empty() {
                        let _ = ctx.events_tx.send(ServerEvent::Message {
                            from: ctx.agent_id.clone(),
                            to: "user".to_string(),
                            content: text.clone(),
                            session_id: ctx.session_id.clone(),
                run_id: None,
                parent_agent_id: None,
            });
                    }
                }
            }
        }
        Err(err) => {
            let error_msg = super::helpers::format_turn_error(&err.to_string());
            persist_and_emit_message(
                &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                &ctx.agent_id, "user", &error_msg, ctx.session_id.as_deref(), false,
            )
            .await;
        }
    }
    let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
}

/// Recover a pending plan from persisted session messages after server restart.
/// Scans the session history backwards for the last plan message — the most
/// recent plan's status decides whether there is a pending plan.
async fn recover_plan_from_session(
    state: &Arc<ServerState>,
    _root: &std::path::Path,
    agent_id: &str,
    session_id: Option<&str>,
) -> Option<crate::engine::Plan> {
    let sid = session_id.unwrap_or("default");
    let messages = state.manager.global_sessions.get_chat_history(sid).ok()?;
    for msg in messages.iter().rev() {
        let parsed = serde_json::from_str::<serde_json::Value>(&msg.content).ok()?;
        if parsed.get("type").and_then(|v| v.as_str()) != Some("plan") {
            continue;
        }
        let plan_val = parsed.get("plan")?;
        let plan = serde_json::from_value::<crate::engine::Plan>(plan_val.clone()).ok()?;
        if plan.status == crate::engine::PlanStatus::Planned {
            tracing::info!("[plan] Recovered pending plan from session history for {agent_id}");
            return Some(plan);
        }
        // Most recent plan is not "planned" — no pending plan
        return None;
    }
    None
}

pub(crate) async fn approve_plan_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<PlanActionRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);
    let root_str = root.to_string_lossy().to_string();
    let session_id = req.session_id.clone();

    // Fallback: after server restart the in-memory pending_plans map is empty.
    // Reconstruct from the last persisted plan message in the session.
    let plan = match state
        .manager
        .take_pending_plan(&root_str, &req.agent_id, session_id.as_deref())
        .await
    {
        Some(p) => Some(p),
        None => recover_plan_from_session(&state, &root, &req.agent_id, session_id.as_deref()).await,
    };
    let Some(mut plan) = plan else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "No pending plan" })),
        )
            .into_response();
    };
    plan.status = PlanStatus::Approved;

    let sid = session_id.as_deref().unwrap_or("default");
    let agent = match state
        .manager
        .get_or_create_session_agent(sid, &root, &req.agent_id)
        .await
    {
        Ok(a) => a,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Agent not found" })),
            )
                .into_response();
        }
    };

    persist_and_emit_message(
        &state.manager,
        &state.events_tx,
        &root,
        &req.agent_id,
        "user",
        &req.agent_id,
        "Plan approved. Starting execution.",
        session_id.as_deref(),
        false,
    )
    .await;

    tokio::spawn(run_approved_plan_task(
        state.clone(),
        agent,
        plan,
        root,
        req.agent_id,
        session_id,
    ));

    Json(serde_json::json!({ "status": "approved" })).into_response()
}

/// Drive an approved plan from "Approved" through execution to "Completed".
/// Spawned by `approve_plan_handler` so the HTTP response returns immediately.
async fn run_approved_plan_task(
    state: Arc<ServerState>,
    agent: Arc<Mutex<AgentEngine>>,
    plan: Plan,
    root: PathBuf,
    agent_id: String,
    session_id: Option<String>,
) {
    let mut engine = agent.lock().await;
    let manager = state.manager.clone();
    let events_tx = state.events_tx.clone();
    let sid_default = session_id.as_deref().unwrap_or("default").to_string();

    engine.plan = Some(plan);
    engine.plan_mode = false;
    engine.observations.clear();
    engine.task = Some(format!(
        "Execute the approved plan: {}",
        engine.plan.as_ref().map(|p| p.summary.as_str()).unwrap_or("Plan")
    ));

    // Emit PlanUpdate SSE and rewrite the persisted plan message in-place.
    let approved_snapshot = engine.plan.clone().unwrap();
    engine.persist_and_emit_plan(approved_snapshot.clone()).await;
    persist_plan_message(&manager, &root, &sid_default, &agent_id, &approved_snapshot).await;

    // Plan execution only calls run_agent_loop (no skill dispatch), so
    // ctx.policy is not consulted. Engine-level restrictions
    // (consumer_allowed_tools, locked) are already set from the original
    // chat request.
    let ctx = ChatRunCtx {
        state: state.clone(),
        manager: manager.clone(),
        events_tx: events_tx.clone(),
        root: root.clone(),
        agent_id: agent_id.clone(),
        session_id: session_id.clone(),
        clean_msg: String::new(),
        images: Vec::new(),
        policy: crate::engine::session_policy::SessionPolicy::default(),
    };
    run_plan_execution(&ctx, &mut engine).await;

    // Mark plan as completed and rewrite the persisted plan message.
    if let Some(ref mut plan) = engine.plan {
        if matches!(plan.status, PlanStatus::Executing | PlanStatus::Approved) {
            plan.status = PlanStatus::Completed;
            let completed_snapshot = plan.clone();
            engine.persist_and_emit_plan(completed_snapshot.clone()).await;
            persist_plan_message(&manager, &root, &sid_default, &agent_id, &completed_snapshot)
                .await;
        }
    }

    let _ = events_tx.send(ServerEvent::TurnComplete {
        agent_id: agent_id.clone(),
        duration_ms: None,
        context_tokens: None,
        parent_id: None,
        session_id: session_id.clone(),
        run_id: None,
        parent_run_id: None,
    });
    state
        .send_agent_status(
            agent_id,
            AgentStatusKind::Idle,
            Some("Idle".to_string()),
            None,
            session_id,
        )
        .await;
}

/// Persist a plan as the latest `{type:"plan", plan:...}` message in the
/// session, falling back to append when no prior plan message exists.
/// Used by approve/edit flows to keep `recover_plan_from_session` aligned
/// with the in-memory plan state across daemon restarts.
async fn persist_plan_message(
    manager: &Arc<AgentManager>,
    root: &Path,
    session_id: &str,
    agent_id: &str,
    plan: &Plan,
) {
    let plan_json = serde_json::json!({ "type": "plan", "plan": plan });
    let msg = crate::state_fs::sessions::ChatMsg {
        agent_id: agent_id.to_string(),
        from_id: agent_id.to_string(),
        to_id: "user".to_string(),
        content: plan_json.to_string(),
        timestamp: crate::util::now_ts_secs(),
        is_observation: false,
    };
    if !manager.update_last_plan_message(session_id, &msg).await {
        manager.add_chat_message(root, session_id, &msg).await;
    }
}

pub(crate) async fn reject_plan_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<PlanActionRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);
    let root_str = root.to_string_lossy().to_string();

    let removed = state
        .manager
        .take_pending_plan(&root_str, &req.agent_id, req.session_id.as_deref())
        .await;
    let removed = match removed {
        Some(p) => Some(p),
        None => recover_plan_from_session(&state, &root, &req.agent_id, req.session_id.as_deref()).await,
    };

    if removed.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "No pending plan" })),
        )
            .into_response();
    }

    let mut rejected_plan = removed.unwrap();
    rejected_plan.status = crate::engine::PlanStatus::Rejected;
    let _ = state.events_tx.send(ServerEvent::PlanUpdate {
        agent_id: req.agent_id.clone(),
        plan: rejected_plan.clone(),
        session_id: req.session_id.clone(),
    });

    // Persist rejection so recover_plan_from_session sees the updated status
    // on reload and doesn't resurface the old "planned" buttons.
    let plan_json = serde_json::json!({ "type": "plan", "plan": rejected_plan });
    persist_and_emit_message(
        &state.manager,
        &state.events_tx,
        &root,
        &req.agent_id,
        &req.agent_id,
        "user",
        &plan_json.to_string(),
        req.session_id.as_deref(),
        false,
    )
    .await;

    Json(serde_json::json!({ "status": "rejected" })).into_response()
}

pub(crate) async fn edit_plan_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<EditPlanRequest>,
) -> impl IntoResponse {
    let root = PathBuf::from(&req.project_root);
    let root_str = root.to_string_lossy().to_string();
    let session_id = req.session_id.clone();
    let agent_id = req.agent_id.clone();

    // Resolve the plan: in-memory first, then session-history fallback.
    // After daemon restart the in-memory map is empty — recover from disk,
    // apply the edit, and re-insert so subsequent approve/reject calls
    // observe the edited plan.
    let plan = match state
        .manager
        .edit_pending_plan(&root_str, &agent_id, session_id.as_deref(), &req.text)
        .await
    {
        Some(p) => p,
        None => {
            let Some(mut plan) = recover_plan_from_session(
                &state, &root, &agent_id, session_id.as_deref(),
            )
            .await
            else {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "No pending plan"})),
                )
                    .into_response();
            };
            plan.plan_text = req.text.clone();
            plan.summary = crate::engine::AgentEngine::extract_plan_summary(&req.text);
            state
                .manager
                .set_pending_plan(&root_str, &agent_id, session_id.as_deref(), plan.clone())
                .await;
            plan
        }
    };

    // Persist the edited plan as the latest plan message. Without this,
    // an edit made before approval is lost across daemon restart —
    // recovery would resurface the pre-edit plan from session history.
    let sid = session_id.as_deref().unwrap_or("default");
    persist_plan_message(&state.manager, &root, sid, &agent_id, &plan).await;

    let _ = state.events_tx.send(ServerEvent::PlanUpdate {
        agent_id,
        plan,
        session_id,
    });

    (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response()
}
