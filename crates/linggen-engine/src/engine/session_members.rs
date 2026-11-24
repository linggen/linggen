//! A session's members — the agents at its table (`doc/shared-session-spec.md`):
//! who a new session seats, the members as they stand, and the one writer of
//! the model the session keeps for a member. The server's `chat::members`
//! builds on it; the engine's own mid-call fallback (`streaming.rs`) writes
//! through it too.

use crate::agent_ids::LEAD_AGENT_ID;
use crate::engine::agent::AgentManager;
use crate::engine::skill::Skill;
use crate::state_fs::sessions::{SessionMember, SessionMeta};

/// The members a new session starts with: the skill's declared members, or
/// the lead agent alone.
pub fn default_members(skill: Option<&Skill>) -> Vec<SessionMember> {
    let declared: Vec<SessionMember> = skill
        .map(|s| s.members.iter().map(|id| SessionMember::new(id)).collect())
        .unwrap_or_default();
    if declared.is_empty() {
        return vec![SessionMember::new(LEAD_AGENT_ID)];
    }
    declared
}

/// The session's members as they stand: its skill's declared members first
/// (in the skill's order, each with the model the session keeps for it),
/// then anyone else the session seated; a session with none seats the
/// defaults.
pub fn effective(meta: &SessionMeta, skill: Option<&Skill>) -> Vec<SessionMember> {
    let mut out: Vec<SessionMember> = skill
        .map(|s| s.members.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|id| {
            meta.member(id)
                .cloned()
                .unwrap_or_else(|| SessionMember::new(id))
        })
        .collect();
    for m in &meta.agents {
        if !out.iter().any(|o| o.id == m.id) {
            out.push(m.clone());
        }
    }
    if out.is_empty() {
        return default_members(skill);
    }
    out
}

/// Keep `model` as `id`'s model in the session (`None` clears it). True
/// when the meta changed.
pub async fn set_model(
    manager: &AgentManager,
    session_id: &str,
    id: &str,
    model: Option<String>,
) -> bool {
    let store = &manager.global_sessions;
    let skill = match store
        .get_session_meta(session_id)
        .ok()
        .flatten()
        .and_then(|m| m.skill)
    {
        Some(name) => manager.skills.get_skill(&name).await,
        None => None,
    };
    store
        .edit(session_id, |meta| {
            // The members as the skill makes them, kept only if the model changed.
            let members = effective(meta, skill.as_ref());
            let stored = std::mem::replace(&mut meta.agents, members);
            if meta.set_member_model(id, model) {
                meta.effort = None;
                return true;
            }
            meta.agents = stored;
            false
        })
        .unwrap_or(false)
}
