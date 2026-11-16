//! What a turn takes from the app it stands in: the session's skill bound
//! to the turn before it runs, and who the turn belongs to.

use super::ChatRunCtx;
use crate::server::ServerEvent;

/// Activate a session-bound skill (set on the session meta, e.g. for
/// skill-embed sessions) so its SKILL.md context, allow-skills scope, and
/// declared permission grants take effect for this turn.
///
/// Skips silently when no skill is bound, when policy blocks it, when the
/// skill name doesn't resolve, or when one is already active. The
/// permission grants are applied only when the session is interactive —
/// non-interactive (mission/consumer) sessions don't auto-elevate.
pub(super) async fn apply_session_bound_skill(
    engine: &mut crate::engine::AgentEngine,
    ctx: &ChatRunCtx,
) {
    if engine.active_skill.is_some() {
        return;
    }
    let Some(sid) = ctx.session_id.as_deref() else {
        return;
    };
    let bound_skill_name = match ctx.manager.global_sessions.get_session_meta(sid) {
        Ok(meta) => meta.and_then(|m| m.skill),
        Err(e) => {
            tracing::warn!("Failed to read session meta for {}: {}", sid, e);
            return;
        }
    };
    let Some(skill_name) = bound_skill_name else {
        return;
    };

    if !ctx.policy.is_skill_allowed(&skill_name) {
        tracing::info!("Session-bound skill '{}' blocked by policy", skill_name);
        return;
    }
    let Some(skill) = ctx.manager.skills.reload_one(&skill_name).await else {
        return;
    };
    tracing::info!("Session-bound skill activated: {}", skill.name);

    // Scoped per-app memory: if the bound skill declares a `memory-context`,
    // wire its dir (+ its recall thresholds) onto the prompt profile so this
    // turn auto-recalls ONLY the rows under `~/.linggen/skills/<name>`.
    // Independent of `include_memory`, which stays off for skill sessions:
    // core rides via `include_core`, nothing else of the biography.
    engine.prompt_profile.scope_to_app(&skill);

    // Ensure session_dir is populated so skill-permission saves persist.
    // Otherwise run_agent_loop later loads permission.json from disk and
    // clobbers the in-memory grants we're about to apply.
    let sdir = crate::paths::global_sessions_dir().join(sid);
    if engine.session_dir.is_none() {
        engine.session_dir = Some(sdir.clone());
    }

    // Hydrate session_permissions from disk BEFORE activate_skill —
    // otherwise write_skill_grants merges SKILL.md grants on top of the
    // empty in-memory default and saves, clobbering any runtime grants
    // (e.g. pulse's workspace_path) that the skill iframe PATCHed onto
    // permission.json before the first user turn. Mirrors the same
    // preload the slash-command path does in skill_dispatch.rs.
    engine.session_permissions = crate::engine::permission::SessionPermissions::load(&sdir);

    if let crate::engine::ActivationOutcome::Activated {
        grants_changed: true,
    } = engine
        .activate_skill(skill, crate::engine::ActivationMode::SessionBound)
        .await
    {
        let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
    }
}

/// Who a turn belongs to: "mission", "skill" or "user". The request names it
/// for a new session; a session already bound to a skill stays the skill's
/// whatever surface the turn came from. The main chat and the phone send no
/// `skill_name`, and reading the request alone ran their turns in a skill
/// session as the user's — the core block and a full biography recall landed
/// in the skill's context.
pub(super) fn turn_creator(
    mission_id: Option<&str>,
    skill_name: Option<&str>,
    bound_skill: Option<&str>,
) -> &'static str {
    if mission_id.is_some() {
        return "mission";
    }
    if skill_name.or(bound_skill).is_some() {
        return "skill";
    }
    "user"
}

#[cfg(test)]
mod tests {
    use super::turn_creator;

    #[test]
    fn turn_creator_follows_the_request_for_a_new_session() {
        assert_eq!(turn_creator(Some("dream"), None, None), "mission");
        assert_eq!(turn_creator(None, Some("health"), None), "skill");
        assert_eq!(turn_creator(None, None, None), "user");
    }

    #[test]
    fn turn_creator_keeps_a_skill_bound_session_the_skills() {
        // A turn typed into a game's session from the main chat or the
        // phone carries no skill_name — it is still the skill's turn.
        assert_eq!(turn_creator(None, None, Some("game")), "skill");
        assert_eq!(turn_creator(Some("dream"), None, Some("game")), "mission");
    }
}
