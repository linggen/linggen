//! A session's members — the agents at its table (`doc/shared-session-spec.md`).
//!
//! The session keeps them in its meta (`agents`, the lead first). A skill
//! declares the members of its sessions in frontmatter (`members:`); an
//! ordinary chat seats the lead agent; Yinyue's daily thread seats her.
//! `@name` addresses a member and seats one who isn't there yet, for good.

use crate::engine::agent::{AgentManager, COMPANION_AGENT_ID, LEAD_AGENT_ID};
use crate::engine::model_choice::Choice;
use crate::engine::skill::Skill;
use crate::state_fs::sessions::{SessionMember, SessionMeta};

// The members of a session, and the one writer of a member's kept model,
// live in the engine (`engine::session_members`): the engine's mid-call
// fallback writes through the same one. Re-exported so every caller reads
// as it did.
pub(crate) use crate::engine::session_members::{default_members, effective, set_model};

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
            if meta.is_old_claude() {
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

/// The model `member`'s turn runs on in its session, for a call made outside
/// the turn (the summary of a compaction, the prompt preview, her push
/// line): what `model_choice::resolve` answers for it — the model the
/// session keeps for the member, else (the companion) her Pet setting, asked
/// as `asker` (the turn's own: a person's turn never names a model they were
/// not given), standing on the agent's own chain. `None`: the agent has no
/// model, or `asker` was given none.
pub(crate) async fn model_of(
    manager: &AgentManager,
    root: &std::path::Path,
    member: &SessionMember,
    asker: crate::engine::model_choice::Asker<'_>,
) -> Option<Choice> {
    let models = manager.models.read().await.clone();
    let pet = match is_companion(&member.id) {
        true => crate::server::resident::resolve_pet_model(&manager.get_config_snapshot().await.pet.model),
        false => None,
    };
    let standing = manager.agent_model_id(root, &member.id).await;
    turn_model_of(&models, (member.model.clone(), pet), standing.as_deref(), asker)
}

/// The model an app's page declares for `member` (SKILL.md `model:`), when
/// it can run now: the standing model of a turn at that app, in front of the
/// agent's own. `None`: no app, none declared, or the declared one cannot
/// run (and the session may switch). The one reader of it — a turn
/// (`handler::resolve_effective_model`) and the "Default" label
/// ([`default_model_of`]) both ask here.
pub(crate) async fn app_standing_model(
    manager: &AgentManager,
    app: Option<&str>,
    member: &str,
    may_switch: bool,
) -> Option<String> {
    let skill = manager.skills.reload_one(app?).await?;
    manager.app_model_id(member, &skill, may_switch).await
}

/// The model session `session_id` runs on when nobody picks one — what its
/// model picker's "Default" names: the answering member's turn with the
/// session's kept model left out (the companion keeps her Pet setting),
/// standing on the app's declared model, else the agent's own chain, asked
/// as `asker`. `None`: no model that `asker` may run.
///
/// An owner's turn at the session's app only: another person's apps are
/// decided by their own policy at the turn, which this does not read.
pub(crate) async fn default_model_of(
    manager: &AgentManager,
    session_id: &str,
    asker: crate::engine::model_choice::Asker<'_>,
) -> Option<Choice> {
    let meta = manager.global_sessions.get_session_meta(session_id).ok().flatten()?;
    let root = std::path::PathBuf::from(&meta.cwd);
    let members = of_session(manager, session_id).await;
    let id = default_responder(&members);
    let may_switch = manager.session_may_switch(session_id);
    let app = super::turn_place::of_session_id(manager, session_id).skill;
    let app = app.filter(|_| matches!(asker, crate::engine::model_choice::Asker::Owner));
    let app_model = app_standing_model(manager, app.as_deref(), &id, may_switch).await;
    let pet = match is_companion(&id) {
        true => crate::server::resident::resolve_pet_model(&manager.get_config_snapshot().await.pet.model),
        false => None,
    };
    let standing = match app_model {
        Some(model) => Some(model),
        None => manager.agent_model_id(&root, &id).await,
    };
    let models = manager.models.read().await.clone();
    turn_model_of(&models, (None, pet), standing.as_deref(), asker)
}

/// [`model_of`] on the facts: the kept and the Pet models, the agent's own.
pub(crate) fn turn_model_of(
    models: &crate::provider::models::ModelManager,
    (kept, pet): (Option<String>, Option<String>),
    standing: Option<&str>,
    asker: crate::engine::model_choice::Asker<'_>,
) -> Option<Choice> {
    use crate::engine::model_choice::{resolve, Rule, OUTSIDE_A_TURN_MAY_SWITCH};
    let asked = kept.or(pet);
    let rule = Rule {
        asker,
        may_switch: OUTSIDE_A_TURN_MAY_SWITCH,
    };
    let choice = resolve(models, asked.as_deref(), standing.unwrap_or(""), rule);
    if choice.nothing_given() {
        return None;
    }
    // With no standing model, only a model that was asked for and runs counts.
    (standing.is_some() || choice.is_asked()).then_some(choice)
}

/// Whether `id` is the companion.
pub(crate) fn is_companion(id: &str) -> bool {
    id == COMPANION_AGENT_ID
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The model the summary of a compaction names for `member`.
    async fn said_for(
        manager: &AgentManager,
        root: &std::path::Path,
        member: &SessionMember,
        asker: crate::engine::model_choice::Asker<'_>,
    ) -> Option<String> {
        let members = [member.clone()];
        let models = crate::server::chat::compact_rows::session_models(manager, root, &members, asker).await?;
        Some(models.summarizer)
    }

    /// The preview and compaction name the model the member's turn runs
    /// on: a kept model this machine lacks means the agent's own model, as
    /// in the turn — not the pet setting behind it.
    #[test]
    fn the_member_model_named_is_the_one_its_turn_runs_on() {
        use crate::engine::model_choice::Asker;
        let models = crate::provider::models::ModelManager::new_with_credentials(
            vec![],
            &crate::credentials::Credentials::default(),
        );
        let luna = Some("gpt-6-luna".to_string());
        let named = |kept: Option<&str>, pet: Option<String>, standing: Option<&str>| {
            turn_model_of(&models, (kept.map(Into::into), pet), standing, Asker::Owner)
                .map(|c| c.model().to_string())
        };
        assert_eq!(
            named(Some("gpt-6-sol"), luna.clone(), Some("own")).as_deref(),
            Some("gpt-6.2-sol")
        );
        assert_eq!(named(None, luna.clone(), Some("own")), luna);
        assert_eq!(named(Some("ghost"), luna, Some("own")).as_deref(), Some("own"));
        assert_eq!(named(Some("ghost"), None, None), None, "no model of its own");
    }

    /// The turn, its summary and its preview agree on one model: what
    /// `model_choice::resolve` answers for the member's turn, for the asker
    /// the turn has — whoever asks, and whatever the member keeps. A person
    /// given models never has one named that they were not given, and given
    /// one, they get that one for all three.
    #[tokio::test]
    async fn the_turn_its_summary_and_its_preview_name_one_model() {
        use crate::engine::model_choice::{resolve, Asker, Rule};
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("agents")).unwrap();
        std::fs::write(
            root.path().join("agents/ling.md"),
            "---\nname: ling\ndescription: d\ntools: []\nmodel: gpt-6.2-sol\n---\nYou are ling.\n",
        )
        .unwrap();
        let config = crate::config::Config::default();
        let (manager, _events) = AgentManager::new(
            config,
            None,
            std::sync::Arc::new(crate::engine::test_registries::Empty),
            std::sync::Arc::new(crate::extensions::agents::AgentLoader::new()),
            std::sync::Arc::new(crate::engine::test_registries::Empty),
            crate::engine::InterfaceMode::Web,
        );
        let models = manager.models.read().await.clone();
        let standing = manager.agent_model_id(root.path(), "ling").await.unwrap();
        assert_eq!(standing, "gpt-6.2-sol", "the agent's own chain");
        let two = vec!["gpt-6.1-terra".to_string(), "gpt-6.2-sol".to_string()];
        let one = vec!["gpt-6.1-terra".to_string()];
        let none: Vec<String> = vec![];
        for given in [Some(&two), Some(&one), Some(&none), None] {
            let asker = given.map_or(Asker::Owner, |g| Asker::Guest(g));
            for kept in [None, Some("ghost-model"), Some("gpt-6.2-sol"), Some("gpt-6.1-terra")] {
                let member = SessionMember { id: "ling".into(), model: kept.map(Into::into) };
                let turn = resolve(&models, kept, &standing, Rule { asker, may_switch: true });
                let wanted = (!turn.nothing_given()).then(|| turn.model().to_string());
                let said = said_for(&manager, root.path(), &member, asker).await;
                assert_eq!(said, wanted, "given {given:?}, kept {kept:?}");
                if let Some(given) = given.filter(|g| !g.is_empty()) {
                    let said = said.expect("they were given models");
                    let own = |id: &str| resolve(&models, Some(id), "", Rule { asker: Asker::Owner, may_switch: true }).model().to_string();
                    assert!(given.iter().any(|g| own(g) == said), "{said} not given: {given:?}, kept {kept:?}");
                }
            }
        }
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
