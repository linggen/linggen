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
    let place = crate::engine::turn_place::TurnPlace::of_session(Some(&meta));
    let _mark = state.manager.global_sessions.running_places.mark(session_id, &place);
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
        place,
        turn: Default::default(),
    };
    ready(&ctx, &mut engine).await;
    let tail = super::thread::spoken_tail(&ctx, TAIL_MESSAGES).await;
    let reply = engine.light_reply(tail, ask, session_id).await;
    super::turn_place::leave_turn_place(&mut engine);
    match reply {
        Ok(line) => line,
        Err(e) => {
            super::helpers::announce_allowance_spent(
                &state.events_tx,
                agent,
                Some(session_id),
                &e.to_string(),
            );
            None
        }
    }
}

/// `agent`'s one line to `ask` in `session_id` now, without waiting for the
/// turn running there: it stands at the session's own place and reads only
/// what that place may be given. Nothing is kept — the caller keeps the line
/// when the session is free (`resident::triggers`).
pub(crate) async fn line_now(state: Arc<ServerState>, session_id: &str, agent: &str, ask: &str) -> Option<String> {
    let meta = state.manager.global_sessions.get_session_meta(session_id).ok()??;
    let root = crate::util::resolve_path(std::path::Path::new(&meta.cwd));
    let engine = state.manager.get_or_create_session_agent(session_id, &root, agent).await.ok()?;
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
        place: crate::engine::turn_place::TurnPlace::of_session(Some(&meta)),
        turn: Default::default(),
    };
    ready(&ctx, &mut engine).await;
    let tail = super::thread::spoken_tail(&ctx, TAIL_MESSAGES).await;
    let line = engine.light_line(tail, ask, session_id).await;
    super::turn_place::leave_turn_place(&mut engine);
    line.ok().flatten()
}

/// The engine as a turn of this member's would have it: live models, the
/// member's model, its seat, and its place.
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
    // A guest's session keeps to the models the room shares here too.
    let shared = engine.guest_models();
    super::handler::resolve_effective_model(
        engine,
        (shared.is_some(), shared.as_deref().unwrap_or_default()),
        &asked,
        (Some(sid), &ctx.agent_id),
        &ctx.manager,
    )
    .await;
    super::handler::seat_member(engine, ctx).await;
    super::turn_place::bind_turn_place(engine, ctx).await;
}
