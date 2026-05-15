//! Every-N-turns memory consolidation trigger.
//!
//! Fired from the post-turn seam in [`super::handler`] once a session has
//! completed a multiple of `consolidate_every_n_turns` turns. The cadence
//! is *derived* from chat history (see [`crate::engine::memory::should_consolidate`])
//! — there is no persisted counter, so it is per-session and restart-safe by
//! construction.
//!
//! This module owns the *trigger*: the owner-only gate, the per-session
//! overlap guard, and the async spawn off the user's turn. The actual
//! encode → consolidate → evict work lives in [`run_consolidation_tick`]
//! (filled in task #5); the widget surface is task #6. See
//! `memory-spec.md` §2 and the locked contract.

use super::ChatRunCtx;
use crate::agent_manager::AgentManager;
use crate::engine::{memory, AgentEngine};
use crate::server::ServerState;
use std::path::PathBuf;
use std::sync::Arc;

/// Inspect the just-completed turn and, if this session has hit a
/// consolidation interval, spawn the consolidation tick off the user's
/// turn. Reads everything it needs from the still-locked `engine` up front
/// so the spawned task never touches the engine lock.
///
/// No-ops (cheap, no spawn) when:
/// - the session is not an owner session (`include_memory == false` — the
///   consumer/mission profile never consolidates the user's biography), or
/// - the turn count is not a positive multiple of the configured interval, or
/// - there is no real session id to key the overlap guard by.
pub(super) fn maybe_fire_consolidation(ctx: &ChatRunCtx, engine: &AgentEngine) {
    // Owner-only: consumer/mission sessions get `include_memory = false`
    // and must not write to the user's memory. Reuses the established gate
    // rather than re-deriving session type.
    if !engine.prompt_profile.include_memory {
        return;
    }
    if !memory::should_consolidate(&engine.chat_history, engine.cfg.consolidate_every_n_turns) {
        return;
    }
    let Some(session_id) = ctx.session_id.clone() else {
        return;
    };

    let state = ctx.state.clone();
    let manager = ctx.manager.clone();
    let agent_id = ctx.agent_id.clone();
    let ws_root = ctx.root.clone();
    let episodic_ttl_days = engine.cfg.episodic_ttl_days;

    tokio::spawn(async move {
        // Per-session overlap guard. Acquired *inside* the task so two
        // rapidly-qualifying turns can't both get past the trigger and
        // double-run: the second observes the id already present and bows
        // out. Mirrors the mission scheduler's per-mission `running` flag.
        {
            let mut active = state.consolidation_active.lock().await;
            if !active.insert(session_id.clone()) {
                tracing::debug!(
                    session_id = %session_id,
                    "memory consolidation already in flight; skipping this tick"
                );
                return;
            }
        }

        let outcome =
            run_consolidation_tick(&state, &manager, &session_id, &agent_id, &ws_root, episodic_ttl_days)
                .await;
        if let Err(e) = outcome {
            // Surfaced as an explicit failure widget in task #6; for now
            // the trigger wiring just records it.
            tracing::warn!(session_id = %session_id, error = %e, "memory consolidation tick failed");
        }

        // Best-effort release. A panic inside the tick would leak the slot
        // until the next daemon restart (which clears the set) — acceptable
        // and matches the mission scheduler's guard robustness; the fix for
        // a panicking tick is the tick, not masking it here.
        state.consolidation_active.lock().await.remove(&session_id);
    });
}

/// One encode → consolidate → evict pass for `session_id`.
///
/// **Stub (task #4).** The trigger, gate, and overlap guard are wired and
/// tested; the body — spawn the built-in `ling-mem` subagent to encode the
/// recent exchange into episodic, consolidate past-TTL episodic rows
/// terminally into semantic, then run the `ling-mem evict` backstop at
/// `now − episodic_ttl_days` — lands in task #5, with the widget surface
/// in task #6. Kept as a real function with the final signature so #5 only
/// fills the body and #4 stays independently committable/testable.
async fn run_consolidation_tick(
    _state: &Arc<ServerState>,
    _manager: &Arc<AgentManager>,
    session_id: &str,
    _agent_id: &str,
    _ws_root: &PathBuf,
    episodic_ttl_days: u64,
) -> anyhow::Result<()> {
    tracing::info!(
        session_id = %session_id,
        episodic_ttl_days,
        "memory consolidation tick fired (trigger wired; encode→consolidate→evict pending task #5)"
    );
    Ok(())
}
