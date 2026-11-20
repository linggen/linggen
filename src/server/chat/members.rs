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

/// Where `agent` sits for a turn standing at `place` of session
/// `session_id`, as [`AgentEngine::seat_at`] takes it: `None` when it leads
/// the session; else at the lead's table, with the lead's declared tools as
/// the session's set — except at home in a day's session (with the setting
/// on), where a member keeps its own list: a day is no table set up for two
/// agents to share one set. At an app's place the place says what each
/// member is offered.
///
/// [`AgentEngine::seat_at`]: crate::engine::AgentEngine::seat_at
pub(crate) async fn seat_of(
    manager: &AgentManager,
    root: &std::path::Path,
    members: &[SessionMember],
    agent: &str,
    session_id: &str,
    place: &crate::engine::turn_place::TurnPlace,
) -> Option<(String, Option<Vec<String>>)> {
    let lead = lead(members);
    if lead == agent {
        return None;
    }
    if place.skill.is_none() && crate::server::day_session::is_day(session_id) && manager.daily_session() {
        return Some((lead, None));
    }
    let tools = manager
        .agents
        .find(root, &lead)
        .await
        .ok()
        .flatten()
        .map(|spec| spec.spec.tools);
    Some((lead, tools))
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
    let skill = match store
        .get_session_meta(session_id)
        .ok()
        .flatten()
        .and_then(|m| m.skill)
    {
        Some(name) => manager.skills.get_skill(&name).await,
        None => None,
    };
    let ok = store
        .edit(session_id, |meta| {
            if crate::server::old_claude::is_read_only(&meta) {
                return false;
            }
            let current = effective(meta, skill.as_ref());
            let Some(members) = seated(&current, id) else {
                return false;
            };
            meta.agents = members;
            true
        })
        .unwrap_or(false);
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
                return true;
            }
            meta.agents = stored;
            false
        })
        .unwrap_or(false)
}

/// The model `member` runs on in its session: the model the session keeps
/// for it, else (the companion) her pet setting — when this machine has
/// it — else its own chain, as its turn would.
pub(crate) async fn model_of(
    manager: &AgentManager,
    root: &std::path::Path,
    member: &SessionMember,
) -> Option<String> {
    let models = manager.models.read().await.clone();
    let pet = is_companion(&member.id).then(|| async {
        crate::server::resident::resolve_pet_model(&manager.get_config_snapshot().await.pet.model)
    });
    let pet = match pet {
        Some(f) => f.await,
        None => None,
    };
    match asked_of(&models, member.model.clone(), pet) {
        Some(m) => Some(m),
        None => manager.agent_model_id(root, &member.id).await,
    }
}

/// The model a member's turn asks for — the kept one, else the pet
/// setting, as the turn asks (`turn_model`) — when it runs as asked
/// (`model_choice::resolve`). `None`: the turn runs on the agent's own.
fn asked_of(
    models: &crate::provider::models::ModelManager,
    kept: Option<String>,
    pet: Option<String>,
) -> Option<String> {
    use crate::engine::model_choice::{resolve, Asker, Rule};
    let asked = kept.or(pet)?;
    let rule = Rule {
        asker: Asker::Owner,
        may_switch: true,
    };
    let choice = resolve(models, Some(&asked), "", rule);
    choice.is_asked().then(|| choice.model().to_string())
}

/// Whether `id` is the companion.
pub(crate) fn is_companion(id: &str) -> bool {
    id == COMPANION_AGENT_ID
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The preview and compaction name the model the member's turn runs
    /// on: a kept model this machine lacks means the agent's own model, as
    /// in the turn — not the pet setting behind it.
    #[test]
    fn the_member_model_named_is_the_one_its_turn_runs_on() {
        let models = crate::provider::models::ModelManager::new_with_credentials(
            vec![],
            &crate::credentials::Credentials::default(),
        );
        let luna = Some("gpt-6-luna".to_string());
        assert_eq!(
            asked_of(&models, Some("gpt-6-sol".into()), luna.clone()).as_deref(),
            Some("gpt-6.2-sol")
        );
        assert_eq!(asked_of(&models, None, luna.clone()), luna);
        assert_eq!(asked_of(&models, Some("ghost".into()), luna), None);
    }

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
