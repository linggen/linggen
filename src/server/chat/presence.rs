//! Whether an agent is there at all in a skill's sessions.
//!
//! A skill may keep an agent away until its own state says otherwise
//! (`place.<agent>.absent_until` — `doc/skill-spec.md` § Place). While the
//! agent is absent, nothing runs for it in that skill's sessions: a guest
//! turn is refused before any model call and no app moment reaches it. The
//! engine names no app and no agent — it reads what the skill declares.

use crate::engine::agent::AgentManager;
use crate::engine::skill::record::Places;
use crate::engine::skill::Skill;

/// The skill a session is bound to.
async fn session_skill(manager: &AgentManager, session_id: &str) -> Option<Skill> {
    let meta = manager
        .global_sessions
        .get_session_meta(session_id)
        .ok()??;
    manager.skills.get_skill(&meta.skill?).await
}

/// The places the skill of `session_id` declares.
pub(crate) async fn session_places(manager: &AgentManager, session_id: &str) -> Option<Places> {
    session_skill(manager, session_id).await?.place
}

/// Whether `agent` is absent from `skill` right now (`Skill::keeps_away`).
fn absent_from(skill: &Skill, agent: &str) -> bool {
    skill.keeps_away(agent)
}

/// Whether `agent` is absent from the skill bound to `session_id`.
pub(crate) async fn absent_in_session(
    manager: &AgentManager,
    session_id: &str,
    agent: &str,
) -> bool {
    session_skill(manager, session_id)
        .await
        .is_some_and(|skill| absent_from(&skill, agent))
}

/// Whether `agent` is absent from the skill named `skill_name`.
pub(crate) async fn absent_in_skill(manager: &AgentManager, skill_name: &str, agent: &str) -> bool {
    manager
        .skills
        .get_skill(skill_name)
        .await
        .is_some_and(|skill| absent_from(&skill, agent))
}

#[cfg(test)]
mod tests {
    use super::absent_from;
    use crate::engine::skill::SkillSource;

    fn skill(extra: &str, dir: Option<&std::path::Path>) -> crate::engine::skill::Skill {
        let text = format!("---\nname: zz\ndescription: d\n{extra}---\nBody.");
        let mut skill =
            crate::extensions::skills::parse_skill_text(&text, SkillSource::Global).unwrap();
        skill.skill_dir = dir.map(|d| d.to_path_buf());
        skill
    }

    /// Kept away until the skill's own state says she's there; everyone
    /// else, and every skill that declares nothing, is always there.
    #[test]
    fn an_agent_is_absent_until_the_skills_state_says_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let gated =
            "place:\n  yinyue:\n    absent_until: {file: state.json, path: companion.joined}\n";
        let s = skill(gated, Some(dir.path()));
        assert!(absent_from(&s, "yinyue"));
        assert!(!absent_from(&s, "ling"));

        std::fs::write(
            dir.path().join("state.json"),
            r#"{"companion":{"joined":"2026-09-18"}}"#,
        )
        .unwrap();
        assert!(!absent_from(&s, "yinyue"));

        assert!(
            absent_from(&skill(gated, None), "yinyue"),
            "unreadable: away"
        );
        assert!(!absent_from(&skill("", Some(dir.path())), "yinyue"));
    }
}
