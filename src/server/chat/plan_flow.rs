use crate::engine::agent::AgentManager;
use crate::engine::{AgentEngine, Plan, PlanStatus};
use crate::server::chat::helpers::{
    emit_outcome_event, persist_and_emit_message, persist_message_only,
};
use crate::server::{ServerEvent, ServerState};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

use super::handler::resolve_request_root;
use super::runtime::{
    push_user_turn_with_recall, run_loop_with_tracking, send_thinking_status,
    spawn_thinking_forwarder, unwire_inbox, wire_engine_bridges,
    wire_inbox,
};
use super::types::{EditPlanRequest, PlanActionRequest};
use super::ChatRunCtx;

/// Dispatch plan mode: agent researches codebase and produces a structured plan (read-only).
pub(super) async fn run_plan_dispatch(ctx: &ChatRunCtx, engine: &mut crate::engine::AgentEngine) {
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

    wire_inbox(ctx, engine);
    wire_engine_bridges(&ctx.state, engine, ctx.session_id.clone());

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
    unwire_inbox(ctx, engine);
    engine.plan_mode = false;

    match outcome {
        Ok(ref out) => {
            // A plan reaches the UI via PlanUpdate; any other reply was
            // already shown and saved by the engine (as in `structured`).
            if let crate::engine::AgentOutcome::Plan(ref plan) = out {
                let plan_json = serde_json::json!({ "type": "plan", "plan": plan }).to_string();
                persist_message_only(
                    &ctx.manager,
                    &ctx.root,
                    &ctx.agent_id,
                    &ctx.agent_id,
                    "user",
                    &plan_json,
                    ctx.session_id.as_deref(),
                    false,
                    engine.last_run_id.as_deref(),
                )
                .await;
            }
            emit_outcome_event(ctx, out, engine.last_run_id.as_deref()).await;
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
                &ctx.manager,
                &ctx.events_tx,
                &ctx.root,
                &ctx.agent_id,
                &ctx.agent_id,
                "user",
                &error_msg,
                ctx.session_id.as_deref(),
                false,
                engine.last_run_id.as_deref(),
            )
            .await;
        }
    }
    let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
}

/// Run the execution loop for an approved plan. Wires thinking/interrupt channels,
/// runs the loop, and emits outcome events. Engine must already have plan + task set.
pub(super) async fn run_plan_execution(ctx: &ChatRunCtx, engine: &mut crate::engine::AgentEngine) {
    let (thinking_tx, thinking_rx) = tokio::sync::mpsc::unbounded_channel();
    engine.thinking_tx = Some(thinking_tx);
    wire_inbox(ctx, engine);
    wire_engine_bridges(&ctx.state, engine, ctx.session_id.clone());

    spawn_thinking_forwarder(
        thinking_rx,
        ctx.events_tx.clone(),
        ctx.agent_id.clone(),
        ctx.session_id.clone(),
    );

    send_thinking_status(ctx, "Executing plan").await;

    let exec_outcome = run_loop_with_tracking(
        &ctx.manager,
        &ctx.root,
        engine,
        &ctx.agent_id,
        ctx.session_id.as_deref(),
        "chat:plan-execution",
        &ctx.events_tx,
    )
    .await;

    engine.thinking_tx = None;
    unwire_inbox(ctx, engine);

    match exec_outcome {
        Ok(ref out) => {
            emit_outcome_event(ctx, out, engine.last_run_id.as_deref()).await;
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
                            client_id: None,
                            via: None,
                            via_note: None,
                            images: Vec::new(),
                        });
                    }
                }
            }
        }
        Err(err) => {
            let error_msg = super::helpers::format_turn_error(&err.to_string());
            persist_and_emit_message(
                &ctx.manager,
                &ctx.events_tx,
                &ctx.root,
                &ctx.agent_id,
                &ctx.agent_id,
                "user",
                &error_msg,
                ctx.session_id.as_deref(),
                false,
                engine.last_run_id.as_deref(),
            )
            .await;
        }
    }
    let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
}

/// A plan's stamp: the place it was made in, and the app it is kept to.
type Stamp = (Option<String>, Option<String>);

/// Where an approved plan runs: the place of the turn that made it, in a
/// day's session; anywhere else the session's own (`turn_place::told_in`).
fn plan_place(
    manager: &crate::engine::agent::AgentManager,
    session_id: &str,
    stamp: &Stamp,
) -> crate::engine::turn_place::TurnPlace {
    super::turn_place::told_in(manager, session_id, stamp.0.as_deref())
}

/// The stamp of the plan `agent_id` waits on in `session_id`: that of its
/// newest plan row still waiting for the person — the plan being decided,
/// whichever place made it. What is written or sent of the plan afterwards
/// — an edit, an approval, its run, a rejection — carries it. Only a day's
/// session stamps a plan; anywhere else its rows are as before.
fn plan_stamp(manager: &crate::engine::agent::AgentManager, session_id: &str, agent_id: &str) -> Stamp {
    if !crate::state_fs::day_session::is_day_session(session_id) {
        return (None, None);
    }
    let rows = manager.global_sessions.get_chat_history(session_id).unwrap_or_default();
    rows.iter()
        .rev()
        .filter(|r| r.from_id == agent_id)
        .find(|r| waiting_plan(&r.content))
        .map(|r| r.stamp())
        .unwrap_or_default()
}

/// Whether `content` is a plan row still waiting for the person.
fn waiting_plan(content: &str) -> bool {
    let Ok(row) = serde_json::from_str::<serde_json::Value>(content) else {
        return false;
    };
    row["type"] == "plan" && row["plan"]["status"] == "planned"
}

/// Keep `line` in the session and send it, stamped as the plan is.
async fn say_of_plan(
    state: &Arc<ServerState>,
    (agent_id, from, to): (&str, &str, &str),
    line: &str,
    (session_id, stamp): (Option<&str>, &Stamp),
) {
    let sid = session_id.unwrap_or("default");
    let (place, kept_to) = stamp.clone();
    let row = crate::state_fs::sessions::ChatMsg {
        run_id: None,
        via: None,
        via_note: None,
        agent_id: agent_id.to_string(),
        from_id: from.to_string(),
        to_id: to.to_string(),
        content: line.to_string(),
        timestamp: crate::util::now_ts_secs(),
        is_observation: false,
        client_id: None,
        failed: false,
        images: Vec::new(),
        place,
        kept_to: kept_to.clone(),
    };
    let event = ServerEvent::Message {
        from: from.to_string(),
        to: to.to_string(),
        content: line.to_string(),
        session_id: session_id.map(str::to_string),
        run_id: None,
        parent_agent_id: None,
        client_id: None,
        via: None,
        via_note: None,
        images: Vec::new(),
    };
    let _ = state.events_tx.send_kept(event, kept_to);
    if let Err(e) = state.manager.global_sessions.add_chat_message(sid, &row) {
        tracing::warn!("[plan] could not keep a line of {sid}: {e}");
    }
}

/// Send the plan's new state, stamped as the plan is.
fn send_plan_update(state: &Arc<ServerState>, agent_id: String, plan: Plan, (session_id, stamp): (Option<String>, &Stamp)) {
    let kept_to = stamp.1.clone();
    let event = ServerEvent::PlanUpdate {
        agent_id,
        plan,
        session_id,
    };
    let _ = state.events_tx.send_kept(event, kept_to);
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
        // Plain-text messages sit between the plan and the tail of the
        // history — skip them. Bailing out here (`.ok()?`) made recovery dead
        // for every plan that wasn't the very last message in the session.
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&msg.content) else {
            continue;
        };
        if parsed.get("type").and_then(|v| v.as_str()) != Some("plan") {
            continue;
        }
        let Some(plan_val) = parsed.get("plan") else {
            continue;
        };
        let Ok(plan) = serde_json::from_value::<crate::engine::Plan>(plan_val.clone()) else {
            continue;
        };
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
    let root = resolve_request_root(&req.project_root);
    let session_id = req.session_id.clone();
    let sid = session_id.as_deref().unwrap_or("default").to_string();
    let Some((mut plan, held)) = take_plan(&state, &root, &req.agent_id, session_id.as_deref()).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "No pending plan" })),
        )
            .into_response();
    };
    plan.status = PlanStatus::Approved;

    let agent = match state
        .manager
        .get_or_create_session_agent(&sid, &root, &req.agent_id)
        .await
    {
        Ok(a) => a,
        Err(_) => {
            crate::server::tasks::tell_all(&state, &sid, &req.agent_id, held);
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Agent not found" })),
            )
                .into_response();
        }
    };

    // The plan's run takes its place in the turn line now, ahead of the
    // background-task ends held while the plan waited and any told later.
    let (_, place) = super::helpers::take_turn_place(state.manager.session_turn(&sid));
    let stamp = plan_stamp(&state.manager, &sid, &req.agent_id);
    say_of_plan(
        &state,
        (&req.agent_id, "user", &req.agent_id),
        "Plan approved. Starting execution.",
        (session_id.as_deref(), &stamp),
    )
    .await;

    let agent_id = req.agent_id.clone();
    tokio::spawn(run_approved_plan_task(
        state.clone(),
        agent,
        plan,
        root,
        req.agent_id,
        session_id,
        (place, stamp),
    ));
    crate::server::tasks::tell_all(&state, &sid, &agent_id, held);

    Json(serde_json::json!({ "status": "approved" })).into_response()
}

/// The plan for the person's decision and the ends held behind it. After a
/// restart, once the person moved on, or when the plan was kept under
/// another root, the plan comes from the session's history; whatever the
/// agent's plans in the session still held is let go with it.
async fn take_plan(
    state: &Arc<ServerState>,
    root: &Path,
    agent_id: &str,
    session_id: Option<&str>,
) -> Option<(Plan, Vec<String>)> {
    let root_str = root.to_string_lossy();
    if let Some(pending) = state.manager.take_pending_plan(&root_str, agent_id, session_id).await {
        return Some((pending.plan, pending.held));
    }
    let plan = recover_plan_from_session(state, root, agent_id, session_id).await?;
    let held = state.manager.drop_pending_plans(session_id.unwrap_or("default"), agent_id).await;
    Some((plan, held))
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
    (place, stamp): (super::helpers::TurnSlot, Stamp),
) {
    let _turn = place.await;
    let mut engine = agent.lock().await;
    let manager = state.manager.clone();
    let events_tx = state.events_tx.clone();
    let sid_default = session_id.as_deref().unwrap_or("default").to_string();

    engine.plan = Some(plan);
    engine.plan_mode = false;
    engine.observations.clear();
    engine.task = Some(format!(
        "Execute the approved plan: {}",
        engine
            .plan
            .as_ref()
            .map(|p| p.summary.as_str())
            .unwrap_or("Plan")
    ));

    // Where it runs, kept before anything of the run is written: in a day's
    // session its rows are the place it was made in (`turn_place::keep_in_day`).
    let place = plan_place(&manager, &sid_default, &stamp);
    let _place = super::turn_place::keep_in_day(&manager, &sid_default, &place).await;
    // Emit PlanUpdate SSE and rewrite the persisted plan message in-place.
    let approved_snapshot = engine.plan.clone().unwrap();
    engine
        .persist_and_emit_plan(approved_snapshot.clone())
        .await;
    persist_plan_message(&manager, &root, (&sid_default, &stamp), &agent_id, &approved_snapshot).await;

    // Plan execution only calls run_agent_loop (no skill dispatch); ctx.policy
    // is consulted only for the place's skill. Engine-level restrictions
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
        sender: None,
        via: None,
        silence_ok: false,
        place,
    };
    run_plan_in_place(&ctx, &mut engine).await;
    complete_plan(&mut engine, &manager, &root, (&sid_default, &agent_id, &stamp)).await;

    let run_id = engine.last_run_id.clone();
    super::runtime::close_turn(&state, agent_id, session_id, run_id).await;
}

/// The plan runs in the place its session's turns stand in: the app's
/// tools and folder, as the turn that made the plan had them.
/// Its thread is brought up to what that place may be given first: the
/// engine's cached thread may have been read for another place.
/// Only a day's session is read again: elsewhere every turn of a session
/// stands in one place, and the plan runs on the thread it was made on.
async fn run_plan_in_place(ctx: &ChatRunCtx, engine: &mut AgentEngine) {
    let day = ctx.session_id.as_deref().is_some_and(crate::state_fs::day_session::is_day_session);
    if day {
        super::thread::sync(engine, ctx).await;
    }
    super::turn_place::bind_turn_place(engine, ctx).await;
    run_plan_execution(ctx, engine).await;
    if day {
        super::thread::mark(engine, ctx);
    }
    super::turn_place::leave_turn_place(engine);
}

/// Mark plan as completed and rewrite the persisted plan message. The run's
/// end may already have settled a done checklist to Completed
/// (`settle_finished_plan`); the saved message still needs the update.
async fn complete_plan(
    engine: &mut AgentEngine,
    manager: &Arc<AgentManager>,
    root: &Path,
    (sid, agent_id, stamp): (&str, &str, &Stamp),
) {
    let Some(ref mut plan) = engine.plan else {
        return;
    };
    if !matches!(
        plan.status,
        PlanStatus::Executing | PlanStatus::Approved | PlanStatus::Completed
    ) {
        return;
    }
    plan.status = PlanStatus::Completed;
    let completed_snapshot = plan.clone();
    engine
        .persist_and_emit_plan(completed_snapshot.clone())
        .await;
    persist_plan_message(manager, root, (sid, stamp), agent_id, &completed_snapshot).await;
}

/// Keep a plan's new state as the latest `{type:"plan", plan:...}` row of
/// the session, appended: `recover_plan_from_session` reads the newest one,
/// so the plan stays aligned with the in-memory state across restarts.
/// Written with the plan's own stamp (`plan_stamp`).
async fn persist_plan_message(
    manager: &Arc<AgentManager>,
    root: &Path,
    (session_id, stamp): (&str, &Stamp),
    agent_id: &str,
    plan: &Plan,
) {
    let plan_json = serde_json::json!({ "type": "plan", "plan": plan });
    let (place, kept_to) = stamp.clone();
    let msg = crate::state_fs::sessions::ChatMsg {
        run_id: None,
        via: None,
        via_note: None,
        agent_id: agent_id.to_string(),
        from_id: agent_id.to_string(),
        to_id: "user".to_string(),
        content: plan_json.to_string(),
        timestamp: crate::util::now_ts_secs(),
        is_observation: false,
        client_id: None,
        failed: false,
        images: Vec::new(),
        place,
        kept_to,
    };
    manager.add_chat_message(root, session_id, &msg).await;
}

pub(crate) async fn reject_plan_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<PlanActionRequest>,
) -> impl IntoResponse {
    let root = resolve_request_root(&req.project_root);
    let Some((mut rejected_plan, held)) =
        take_plan(&state, &root, &req.agent_id, req.session_id.as_deref()).await
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "No pending plan" })),
        )
            .into_response();
    };
    let stamp = plan_stamp(&state.manager, req.session_id.as_deref().unwrap_or("default"), &req.agent_id);
    rejected_plan.status = crate::engine::PlanStatus::Rejected;
    send_plan_update(&state, req.agent_id.clone(), rejected_plan.clone(), (req.session_id.clone(), &stamp));

    // Persist rejection so recover_plan_from_session sees the updated status
    // on reload and doesn't resurface the old "planned" buttons.
    let plan_json = serde_json::json!({ "type": "plan", "plan": rejected_plan });
    say_of_plan(
        &state,
        (&req.agent_id, &req.agent_id, "user"),
        &plan_json.to_string(),
        (req.session_id.as_deref(), &stamp),
    )
    .await;
    crate::server::tasks::tell_all(
        &state,
        req.session_id.as_deref().unwrap_or("default"),
        &req.agent_id,
        held,
    );

    Json(serde_json::json!({ "status": "rejected" })).into_response()
}

pub(crate) async fn edit_plan_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<EditPlanRequest>,
) -> impl IntoResponse {
    let root = resolve_request_root(&req.project_root);
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
            let Some(mut plan) =
                recover_plan_from_session(&state, &root, &agent_id, session_id.as_deref()).await
            else {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "No pending plan"})),
                )
                    .into_response();
            };
            plan.set_plan_text(&req.text);
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
    let stamp = plan_stamp(&state.manager, sid, &agent_id);
    persist_plan_message(&state.manager, &root, (sid, &stamp), &agent_id, &plan).await;
    send_plan_update(&state, agent_id, plan, (session_id, &stamp));

    (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response()
}
