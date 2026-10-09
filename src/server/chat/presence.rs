//! Whether an agent is there at all in a skill's sessions.
//!
//! A skill may keep an agent away until its own state says otherwise
//! (`place.<agent>.absent_until` — `doc/skill-spec.md` § Place). While the
//! agent is absent, nothing runs for it in that skill's sessions: a message
//! to it is refused before any model call and no app moment reaches it. The
//! engine names no app and no agent — it reads what the skill declares.
//!
//! Outside a skill that gates the agent, the agent is there when it is there
//! at all (`engine::agent::met`): one that appears only once met is absent
//! from every other session until it has been. A skill's own gate stays
//! authoritative at its own table.

use crate::engine::agent::AgentManager;
use crate::engine::skill::Skill;

/// The skill a session is bound to.
async fn session_skill(manager: &AgentManager, session_id: &str) -> Option<Skill> {
    let meta = manager
        .global_sessions
        .get_session_meta(session_id)
        .ok()??;
    manager.skills.get_skill(&meta.skill?).await
}

/// Whether `agent` is absent from `skill` right now: the skill's own gate
/// when it has one (`Skill::keeps_away`), else whether the agent is there
/// at all (`here`).
fn absent_from(skill: &Skill, agent: &str, here: bool) -> bool {
    match &skill.place {
        Some(places) if places.is_gated(agent) => skill.keeps_away(agent),
        _ => !here,
    }
}

/// Whether `agent` is absent from the skill bound to `session_id` — or, for
/// a session with no skill, from the machine.
pub(crate) async fn absent_in_session(
    manager: &AgentManager,
    session_id: &str,
    agent: &str,
) -> bool {
    let here = manager.agent_present_now(agent).await;
    match session_skill(manager, session_id).await {
        Some(skill) => absent_from(&skill, agent, here),
        None => !here,
    }
}

/// Whether `agent` is absent from the skill named `skill_name` — or, for a
/// name that is no skill, from the machine.
pub(crate) async fn absent_in_skill(manager: &AgentManager, skill_name: &str, agent: &str) -> bool {
    let here = manager.agent_present_now(agent).await;
    match manager.skills.get_skill(skill_name).await {
        Some(skill) => absent_from(&skill, agent, here),
        None => !here,
    }
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
        assert!(absent_from(&s, "yinyue", true));
        assert!(!absent_from(&s, "ling", true));

        std::fs::write(
            dir.path().join("state.json"),
            r#"{"companion":{"joined":"2026-09-18"}}"#,
        )
        .unwrap();
        assert!(!absent_from(&s, "yinyue", true));

        assert!(
            absent_from(&skill(gated, None), "yinyue", true),
            "unreadable: away"
        );
        assert!(!absent_from(&skill("", Some(dir.path())), "yinyue", true));
    }

    /// The skill's own gate is authoritative at its table, whether or not the
    /// agent is there elsewhere; a skill with no gate for her follows the
    /// machine — she isn't at its table before she is met.
    #[test]
    fn a_skill_with_its_own_gate_decides_and_one_without_follows_the_machine() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("state.json"),
            r#"{"companion":{"joined":"2026-09-18"}}"#,
        )
        .unwrap();
        let gated =
            "place:\n  yinyue:\n    absent_until: {file: state.json, path: companion.joined}\n";
        let own = skill(gated, Some(dir.path()));
        assert!(
            !absent_from(&own, "yinyue", false),
            "its gate says she is there"
        );

        let open = skill("members: [ling, yinyue]\n", Some(dir.path()));
        assert!(
            absent_from(&open, "yinyue", false),
            "not met: not at its table"
        );
        assert!(!absent_from(&open, "yinyue", true));
        assert!(!absent_from(&open, "ling", true));
    }
}
