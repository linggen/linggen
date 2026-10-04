//! A session's members — the agents at its table (`doc/shared-session-spec.md`).
//!
//! The session keeps them in its meta (`agents`, the lead first). A skill
//! declares the members of its sessions in frontmatter (`members:`); an
//! ordinary chat seats the lead agent; Yinyue's daily thread seats her.
//! `@name` addresses a member and seats one who isn't there yet, for good.

use crate::engine::agent::{AgentManager, COMPANION_AGENT_ID, LEAD_AGENT_ID};
use crate::engine::skill::Skill;
use crate::state_fs::sessions::{SessionMember, SessionMeta};

/// The members a new session starts with: the skill's declared members, or
/// the lead agent alone.
pub(crate) fn default_members(skill: Option<&Skill>) -> Vec<SessionMember> {
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
pub(crate) fn effective(meta: &SessionMeta, skill: Option<&Skill>) -> Vec<SessionMember> {
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

/// Who answers a message that names nobody, on the Mac: the lead agent when
/// it is a member, else the session's first member.
pub(crate) fn default_responder(members: &[SessionMember]) -> String {
    if members.iter().any(|m| m.id == LEAD_AGENT_ID) {
        return LEAD_AGENT_ID.to_string();
    }
    members
        .first()
        .map(|m| m.id.clone())
        .unwrap_or_else(|| LEAD_AGENT_ID.to_string())
}

/// The session's lead: the member whose tools an ordinary session uses and
/// whose place a skill's SKILL.md is.
pub(crate) fn lead(members: &[SessionMember]) -> String {
    members
        .first()
        .map(|m| m.id.clone())
        .unwrap_or_else(|| LEAD_AGENT_ID.to_string())
}

/// `members` with `id` seated (at the end) — `None` when already there.
pub(crate) fn seated(members: &[SessionMember], id: &str) -> Option<Vec<SessionMember>> {
    if members.iter().any(|m| m.id == id) {
        return None;
    }
    let mut out = members.to_vec();
    out.push(SessionMember::new(id));
    Some(out)
}

/// The session's members, read from its meta and its skill.
pub(crate) async fn of_session(manager: &AgentManager, session_id: &str) -> Vec<SessionMember> {
    let Ok(Some(meta)) = manager.global_sessions.get_session_meta(session_id) else {
        return default_members(None);
    };
    let skill = match meta.skill.as_deref() {
        Some(name) => manager.skills.get_skill(name).await,
        None => None,
    };
    effective(&meta, skill.as_ref())
}

/// Seat `id` in the session for good (a no-op when it is there already).
/// True when it was added.
pub(crate) async fn seat(manager: &AgentManager, session_id: &str, id: &str) -> bool {
    let store = &manager.global_sessions;
    let Ok(Some(mut meta)) = store.get_session_meta(session_id) else {
        return false;
    };
    let current = of_session(manager, session_id).await;
    let Some(members) = seated(&current, id) else {
        return false;
    };
    meta.agents = members;
    let ok = store.update_session_meta(&meta).is_ok();
    if ok {
        tracing::info!("[members] {id} joins {session_id}");
    }
    ok
}

/// Keep `model` as `id`'s model in the session (`None` clears it). True
/// when the meta changed.
pub(crate) async fn set_model(
    manager: &AgentManager,
    session_id: &str,
    id: &str,
    model: Option<String>,
) -> bool {
    let store = &manager.global_sessions;
    let Ok(Some(mut meta)) = store.get_session_meta(session_id) else {
        return false;
    };
    meta.agents = of_session(manager, session_id).await;
    meta.set_member_model(id, model) && store.update_session_meta(&meta).is_ok()
}

/// Whether `id` is the companion.
pub(crate) fn is_companion(id: &str) -> bool {
    id == COMPANION_AGENT_ID
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(members: &[&str]) -> Skill {
        let text = format!(
            "---\nname: game\ndescription: d\nmembers: [{}]\n---\nBody.",
            members.join(", ")
        );
        crate::extensions::skills::parse_skill_text(
            &text,
            crate::engine::skill::SkillSource::Global,
        )
        .unwrap()
    }

    fn ids(members: &[SessionMember]) -> Vec<&str> {
        members.iter().map(|m| m.id.as_str()).collect()
    }

    /// An ordinary chat seats Ling; a skill seats what it declares.
    #[test]
    fn a_new_session_seats_the_lead_or_the_skills_members() {
        assert_eq!(ids(&default_members(None)), ["ling"]);
        assert_eq!(
            ids(&default_members(Some(&skill(&["ling", "yinyue"])))),
            ["ling", "yinyue"]
        );
    }

    /// The skill's members lead, in its order, keeping the models the
    /// session holds for them; anyone the session seated besides follows.
    #[test]
    fn a_skills_members_lead_and_keep_their_session_models() {
        let meta = SessionMeta {
            agents: vec![
                SessionMember {
                    id: "yinyue".into(),
                    model: Some("luna".into()),
                },
                SessionMember::new("memory"),
            ],
            ..Default::default()
        };
        let got = effective(&meta, Some(&skill(&["ling", "yinyue"])));
        assert_eq!(ids(&got), ["ling", "yinyue", "memory"]);
        assert_eq!(got[1].model.as_deref(), Some("luna"));
        assert_eq!(ids(&effective(&SessionMeta::default(), None)), ["ling"]);
    }

    /// On the Mac Ling answers a message that names nobody when he is
    /// there; otherwise the first member does (her daily thread: her).
    #[test]
    fn the_mac_default_is_ling_if_seated_else_the_first_member() {
        let her = vec![SessionMember::new("yinyue")];
        assert_eq!(default_responder(&her), "yinyue");
        let both = vec![SessionMember::new("yinyue"), SessionMember::new("ling")];
        assert_eq!(default_responder(&both), "ling");
        assert_eq!(lead(&both), "yinyue", "the lead is the first member");
        assert_eq!(default_responder(&[]), "ling");
    }

    /// `@name` seats a member once; seating one already there changes
    /// nothing.
    #[test]
    fn addressing_seats_a_member_once() {
        let ling = vec![SessionMember::new("ling")];
        let both = seated(&ling, "yinyue").unwrap();
        assert_eq!(ids(&both), ["ling", "yinyue"]);
        assert!(seated(&both, "yinyue").is_none());
    }

    /// An old session's pinned agent and model become a one-member list;
    /// one that pinned neither seats who answered there.
    #[test]
    fn an_old_session_migrates_to_members() {
        let yaml =
            "id: s\ntitle: t\ncreated_at: 1\ncreator: mission\nagent_id: memory\nmodel_id: gpt-x\n";
        let mut meta: SessionMeta = serde_norway::from_str(yaml).unwrap();
        assert!(meta.migrate_members(|| vec!["ling".into()]));
        assert_eq!(
            meta.agents,
            vec![SessionMember {
                id: "memory".into(),
                model: Some("gpt-x".into())
            }]
        );
        let written = serde_norway::to_string(&meta).unwrap();
        assert!(!written.contains("agent_id") && !written.contains("model_id"));
        assert!(!meta.migrate_members(|| vec!["x".into()]), "once");

        let mut chat: SessionMeta =
            serde_norway::from_str("id: c\ntitle: t\ncreated_at: 1\nmodel_id: gpt-y\n").unwrap();
        chat.migrate_members(|| vec!["ling".into(), "yinyue".into()]);
        assert_eq!(ids(&chat.agents), ["ling"]);
        assert_eq!(chat.agents[0].model.as_deref(), Some("gpt-y"));

        let mut table: SessionMeta =
            serde_norway::from_str("id: g\ntitle: t\ncreated_at: 1\nskill: game\n").unwrap();
        table.migrate_members(|| vec!["ling".into(), "yinyue".into()]);
        assert_eq!(ids(&table.agents), ["ling", "yinyue"]);
    }
}
