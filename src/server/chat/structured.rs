use crate::server::chat::helpers::{emit_outcome_event, persist_and_emit_message, persist_message_only};
use crate::server::ServerEvent;

use super::plan_flow::{run_plan_dispatch, run_plan_execution};
use super::runtime::{
    persist_and_emit_last_assistant_text, push_user_turn_with_recall, run_loop_with_tracking,
    send_thinking_status, spawn_thinking_forwarder, unwire_interrupt_channel, wire_ask_user_bridge,
    wire_interrupt_channel,
};
use super::ChatRunCtx;

/// Dispatch the structured (auto) mode agent loop.
pub(super) async fn run_structured_loop(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
) {
    // Vision gate: reject images if the model doesn't support vision.
    if !ctx.images.is_empty() {
        let has_vision = engine
            .model_manager
            .has_vision(&engine.model_id)
            .await
            .unwrap_or(false);
        if !has_vision {
            let err_msg = format!(
                "Model `{}` does not support vision/image input. Please use a vision-capable model (e.g. qwen3-vl, llava, llama3.2-vision).",
                engine.model_id
            );
            persist_and_emit_message(
                &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
            )
            .await;
            let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
            return;
        }
        engine.pending_images = ctx.images.clone();
    }

    send_thinking_status(ctx, "Thinking").await;
    engine.observations.clear();
    // Drop a stale "planned" plan from a previous plan-mode run so it doesn't
    // block execution of the new structured loop.
    if let Some(p) = &engine.plan {
        if p.status == crate::engine::PlanStatus::Planned {
            engine.plan = None;
        }
    }
    let task_for_loop = ctx.clean_msg.trim().to_string();
    engine.task = Some(task_for_loop);
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
        &ctx.manager, &ctx.root, engine, &ctx.agent_id,
        ctx.session_id.as_deref(), "chat:structured-loop", &ctx.events_tx,
    )
    .await;

    // Drop the thinking sender so the forwarder task exits.
    engine.thinking_tx = None;
    unwire_interrupt_channel(ctx, engine, &interrupt_key).await;

    // Agent requested plan mode — re-dispatch using existing plan machinery.
    if let Ok(crate::engine::AgentOutcome::PlanModeRequested { ref reason }) = outcome {
        let plan_task = reason.clone().unwrap_or_else(|| ctx.clean_msg.clone());
        engine.task = Some(plan_task);
        run_plan_dispatch(ctx, engine).await;
        return;
    }

    // Agent created a plan that needs approval — store as pending.
    if let Ok(ref ok_outcome @ crate::engine::AgentOutcome::Plan(ref plan)) = outcome {
        // Persist the plan as a JSON message so it survives session reload.
        // The UI renders it as a PlanBlock via tryRenderSpecialBlock.
        let plan_json = serde_json::json!({ "type": "plan", "plan": plan }).to_string();
        persist_message_only(
            &ctx.manager, &ctx.root, &ctx.agent_id,
            &ctx.agent_id, "user", &plan_json,
            ctx.session_id.as_deref(), false,
        ).await;
        emit_outcome_event(ok_outcome, &ctx.events_tx, &ctx.agent_id, ctx.session_id.as_deref());
        ctx.manager
            .set_pending_plan(
                &ctx.root.to_string_lossy(),
                &ctx.agent_id,
                ctx.session_id.as_deref(),
                plan.clone(),
            )
            .await;
        let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
        return;
    }

    // Agent plan was approved inline — start execution immediately.
    if let Ok(crate::engine::AgentOutcome::PlanApproved(ref plan)) = outcome {
        persist_and_emit_last_assistant_text(ctx, engine).await;
        emit_outcome_event(outcome.as_ref().unwrap(), &ctx.events_tx, &ctx.agent_id, ctx.session_id.as_deref());
        engine.plan = Some(plan.clone());
        engine.plan_mode = false;
        engine.observations.clear();
        if engine.task.is_none() {
            engine.task = Some(format!("Execute the approved plan: {}", plan.summary));
        }
        run_plan_execution(ctx, engine).await;
        return;
    }

    if let Ok(outcome) = &outcome {
        emit_outcome_event(outcome, &ctx.events_tx, &ctx.agent_id, ctx.session_id.as_deref());
        // persist_assistant_message() (engine/context.rs) already emits
        // AgentEvent::Message which the bridge converts to ServerEvent::Message.
        // Emitting one here would duplicate the assistant response for WebRTC consumers.
    } else if let Err(err) = outcome {
        let error_msg = super::helpers::format_turn_error(&err.to_string());
        persist_and_emit_message(
            &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
            &ctx.agent_id, "user", &error_msg, ctx.session_id.as_deref(), false,
        )
        .await;
    }
    let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
}
