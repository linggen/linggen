//! What a turn takes from the app it stands in: its place entered before
//! the turn runs and left when it ends, and who the turn belongs to.

use super::ChatRunCtx;
use crate::engine::turn_place::TurnPlace;
use crate::server::ServerEvent;

/// Stand this turn in its place (`ctx.place`): the place's skill — its
/// SKILL.md, tools, folder, memory scope and declared grants — is entered
/// for this turn, and whatever place the engine stood in before is left
/// ([`AgentEngine::enter_place`]). [`leave_turn_place`] puts it all back
/// when the turn ends.
///
/// A skill the policy blocks, or one that no longer resolves, enters the
/// place without it. The declared grants are applied only when the session
/// is interactive — non-interactive (mission/consumer) sessions don't
/// auto-elevate.
///
/// [`AgentEngine::enter_place`]: crate::engine::AgentEngine::enter_place
pub(super) async fn bind_turn_place(engine: &mut crate::engine::AgentEngine, ctx: &ChatRunCtx) {
    // Left before this place's grants are read: leaving puts the grants
    // back to the session's own.
    engine.leave_place();
    let skill = place_skill(ctx).await;
    if skill.is_some() {
        if let Some(sid) = ctx.session_id.as_deref() {
            load_grants(engine, sid, ctx.place.grants_key());
        }
    }
    let outcome = engine
        .enter_place(
            ctx.place.clone(),
            skill,
            crate::engine::ActivationMode::SessionBound,
        )
        .await;
    if matches!(outcome, crate::engine::ActivationOutcome::Activated { grants_changed: true }) {
        let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
    }
    let day_on = crate::server::day_session::enabled(&ctx.manager).await;
    engine.keep_talk_to(ctx.place.private_app(&*ctx.manager.skills, day_on).await);
}

/// In a day's session, keep `place` as the running turn's before a turn the
/// engine starts on its own writes anything (a task's end notice, an
/// approved plan's run): its rows are then that place's. Elsewhere nothing
/// is kept, and rows are as before.
pub(crate) fn keep_in_day(
    manager: &crate::engine::agent::AgentManager,
    session_id: &str,
    place: &TurnPlace,
) -> Option<crate::engine::turn_place::running::PlaceMark> {
    if !crate::state_fs::day_session::is_day_session(session_id) {
        return None;
    }
    mark_turn(manager, session_id, place)
}

/// Keep `place` as the running turn's in `session_id`: the rows it saves
/// name it. `None` when a turn's place is kept there already.
pub(crate) fn mark_turn(
    manager: &crate::engine::agent::AgentManager,
    session_id: &str,
    place: &TurnPlace,
) -> Option<crate::engine::turn_place::running::PlaceMark> {
    manager.global_sessions.running_places.mark(session_id, place)
}

/// The turn is over: nothing of its place stays on the engine.
pub(super) fn leave_turn_place(engine: &mut crate::engine::AgentEngine) {
    engine.leave_place();
}

/// The place's skill, when it has one the turn's policy allows and it still
/// resolves.
async fn place_skill(ctx: &ChatRunCtx) -> Option<crate::engine::skill::Skill> {
    let name = ctx.place.skill.as_deref()?;
    if !ctx.policy.is_skill_allowed(name) {
        tracing::info!("Place skill '{}' blocked by policy", name);
        return None;
    }
    let skill = ctx.manager.skills.reload_one(name).await?;
    tracing::info!("Place skill entered: {}", skill.name);
    Some(skill)
}

/// Read the session's grants before the skill's are written over them.
///
/// `session_dir` is set so skill-permission saves persist: otherwise
/// run_agent_loop later loads permission.json from disk and clobbers the
/// in-memory grants about to be applied. The grants are hydrated from disk
/// BEFORE the skill is entered — otherwise write_skill_grants merges
/// SKILL.md grants on top of the empty in-memory default and saves,
/// clobbering any runtime grants (e.g. pulse's workspace_path) that the
/// skill iframe PATCHed onto permission.json before the first user turn.
/// Mirrors the same preload the slash-command path does in skill_dispatch.rs.
fn load_grants(engine: &mut crate::engine::AgentEngine, sid: &str, place: Option<&str>) {
    let sdir = crate::paths::global_sessions_dir().join(sid);
    if engine.session_dir.is_none() {
        engine.session_dir = Some(sdir.clone());
    }
    engine.session_permissions =
        crate::engine::permission::SessionPermissions::load_at(&sdir, place);
}

/// The place a turn of session `session_id` stands in when no page sent it
/// (`TurnPlace::of_session`).
pub(crate) fn of_session_id(
    manager: &crate::engine::agent::AgentManager,
    session_id: &str,
) -> TurnPlace {
    let meta = manager.global_sessions.get_session_meta(session_id).ok().flatten();
    TurnPlace::of_session(meta.as_ref())
}

/// Where a turn the engine starts on its own stands — a background task's
/// end, an approved plan's run: in a day's session, the place `key` its work
/// began in (the app it is about); anywhere else, and with no key, the
/// session's own.
pub(crate) fn told_in(
    manager: &crate::engine::agent::AgentManager,
    session_id: &str,
    key: Option<&str>,
) -> TurnPlace {
    let meta = manager.global_sessions.get_session_meta(session_id).ok().flatten();
    told_place(meta.as_ref(), key)
}

fn told_place(meta: Option<&crate::state_fs::sessions::SessionMeta>, key: Option<&str>) -> TurnPlace {
    let day = meta.is_some_and(|m| crate::state_fs::day_session::is_day_session(&m.id));
    let (true, Some(key)) = (day, key) else {
        return TurnPlace::of_session(meta);
    };
    let skill = (key != crate::engine::turn_place::HOME_KEY).then_some(key);
    TurnPlace::of_request(skill, meta)
}

/// Who a turn belongs to: "mission", "skill" or "user" — a mission's, else
/// the app's its place stands in (`TurnPlace::of_request`: the page's skill
/// for a new session; a session already bound to a skill stays the skill's
/// whatever surface the turn came from). The main chat and the phone send no
/// `skill_name`, and reading the request alone ran their turns in a skill
/// session as the user's — the core block and a full biography recall landed
/// in the skill's context.
pub(super) fn turn_creator(mission_id: Option<&str>, place: &TurnPlace) -> &'static str {
    if mission_id.is_some() {
        return "mission";
    }
    if place.skill.is_some() {
        return "skill";
    }
    "user"
}

#[cfg(test)]
mod tests {
    use super::{told_place, turn_creator};

    /// A task's end and an approved plan stand where their work began — in
    /// a day's session; anywhere else in the session's own place, as before.
    #[test]
    fn work_the_engine_tells_of_stands_where_it_began_in_a_day() {
        let day = SessionMeta { id: "sess-day-2026-11-16".into(), ..Default::default() };
        assert_eq!(told_place(Some(&day), Some("ledger")).skill.as_deref(), Some("ledger"));
        assert_eq!(told_place(Some(&day), Some(":home")), TurnPlace::home());
        assert_eq!(told_place(Some(&day), None), TurnPlace::home());
        let game = SessionMeta { id: "sess-1-g".into(), skill: Some("game".into()), ..Default::default() };
        assert_eq!(told_place(Some(&game), Some("ledger")), TurnPlace::of_session(Some(&game)));
    }
    use crate::engine::turn_place::TurnPlace;
    use crate::state_fs::sessions::SessionMeta;

    #[test]
    fn turn_creator_follows_the_request_for_a_new_session() {
        let health = TurnPlace::of_request(Some("health"), None);
        assert_eq!(turn_creator(Some("dream"), &TurnPlace::home()), "mission");
        assert_eq!(turn_creator(None, &health), "skill");
        assert_eq!(turn_creator(None, &TurnPlace::of_request(None, None)), "user");
    }

    #[test]
    fn turn_creator_keeps_a_skill_bound_session_the_skills() {
        // A turn typed into a game's session from the main chat or the
        // phone carries no skill_name — it is still the skill's turn.
        let game = SessionMeta {
            skill: Some("game".into()),
            ..Default::default()
        };
        let place = TurnPlace::of_request(None, Some(&game));
        assert_eq!(turn_creator(None, &place), "skill");
        assert_eq!(turn_creator(Some("dream"), &place), "mission");
    }
}
