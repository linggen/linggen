//! A light turn in a session (`engine::light_turn`): the session's agent
//! answers a hidden ask in one line, or is silent — its soul, the chat's
//! recent tail, no tools. It waits behind a running turn and runs on the
//! member's model, as a full turn would; the ask itself is never kept.

use super::ChatRunCtx;
use crate::server::ServerState;
use std::sync::Arc;

/// How many of the thread's newest messages a light turn reads.
const TAIL_MESSAGES: usize = 6;

/// `agent`'s one line to `ask` in `session_id`, kept as its reply; `None`
/// when it was silent, the session is gone, or the call failed.
pub(crate) async fn reply_in_session(
    state: Arc<ServerState>,
    session_id: &str,
    agent: &str,
    ask: &str,
) -> Option<String> {
    let meta = state
        .manager
        .global_sessions
        .get_session_meta(session_id)
        .ok()??;
    let root = crate::util::resolve_path(std::path::Path::new(&meta.cwd));
    let engine = state
        .manager
        .get_or_create_session_agent(session_id, &root, agent)
        .await
        .ok()?;
    // One agent speaks at a time: the session's turn lock, then the engine.
    let _turn = state.manager.session_turn(session_id).lock_owned().await;
    let mut engine = engine.lock().await;
    let ctx = ChatRunCtx {
        state: state.clone(),
        manager: state.manager.clone(),
        events_tx: state.events_tx.clone(),
        root,
        agent_id: agent.to_string(),
        session_id: Some(session_id.to_string()),
        clean_msg: String::new(),
        images: Vec::new(),
        policy: crate::engine::session_policy::SessionPolicy::owner(),
        sender: None,
        via: None,
        silence_ok: true,
    };
    ready(&ctx, &mut engine).await;
    let tail = super::thread::spoken_tail(&ctx, TAIL_MESSAGES).await;
    engine.light_reply(tail, ask, session_id).await
}

/// The engine as a turn of this member's would have it: live models, the
/// member's model, its seat, and the session's skill.
async fn ready(ctx: &ChatRunCtx, engine: &mut crate::engine::AgentEngine) {
    engine.model_manager = ctx.manager.models.read().await.clone();
    let sid = ctx.session_id.as_deref().unwrap_or("default");
    let members = super::members::of_session(&ctx.manager, sid).await;
    let pet = ctx.manager.get_config_snapshot().await.pet;
    let asked = super::handler::turn_model(
        None,
        &ctx.agent_id,
        &members,
        crate::server::resident::resolve_pet_model(&pet.model),
    );
    super::handler::resolve_effective_model(
        engine,
        (false, &[]),
        &asked,
        (Some(sid), &ctx.agent_id),
        &ctx.manager,
    )
    .await;
    super::handler::seat_member(engine, ctx).await;
    super::handler::apply_session_bound_skill(engine, ctx).await;
}
