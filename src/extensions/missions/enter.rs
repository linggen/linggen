//! Entering a mission context. The scheduler's dispatch, a chat turn typed
//! into a mission session, and the system-prompt export all set the engine
//! up the same way — here.

use crate::engine::mission::record::Mission;
use crate::engine::mission::MissionRegistry;
use crate::engine::skill::Skill;
use crate::engine::skill::SkillRegistry;
use crate::engine::tool_scope::compute_tool_scope;
use crate::engine::AgentEngine;
use std::collections::HashSet;

/// Put `mission` in charge of `engine`: its body as the runbook, its tool
/// scope, and — for a skill mission — its own skill's tools. The skill's
/// SKILL.md body is never added. Returns that skill so a dispatch can apply
/// its grants.
pub async fn enter_mission(
    engine: &mut AgentEngine,
    mission: &Mission,
    missions: &dyn MissionRegistry,
    skills: &dyn SkillRegistry,
) -> Option<Skill> {
    engine.active_mission = Some(crate::engine::ActiveMission {
        name: mission.name.clone().unwrap_or_else(|| mission.id.clone()),
        description: mission.description.clone(),
        body: mission.prompt.clone(),
        mission_dir: Some(missions.mission_dir(&mission.id)),
    });
    let skill = owning_skill(mission, skills).await;
    if let Some(ref skill) = skill {
        engine.lend_skill_tools(skill);
    }
    engine.cfg.mission_allowed_tools = tool_scope(mission, skill.as_ref());
    // Mission sessions don't write to the user's biographical memory and
    // don't carry the core block + memory protocol. Any cached prompt
    // predates the mission body.
    engine.prompt_profile.include_memory = false;
    engine.cached_system_prompt = None;
    skill
}

async fn owning_skill(mission: &Mission, skills: &dyn SkillRegistry) -> Option<Skill> {
    let name = mission.skill.as_deref()?;
    let skill = match skills.reload_one(name).await {
        Some(s) => Some(s),
        None => skills.get_skill(name).await,
    };
    if skill.is_none() {
        tracing::warn!(
            "mission '{}': its skill '{}' is not installed — running without its tools",
            mission.id,
            name
        );
    }
    skill
}

/// `allowed-tools` as the engine's restriction set, widened by the owning
/// skill's own tools — intrinsic to a skill mission, as a skill's tools are
/// to its own sessions. Empty `allowed-tools` stays unrestricted.
pub fn tool_scope(mission: &Mission, skill: Option<&Skill>) -> Option<HashSet<String>> {
    let mut scope = compute_tool_scope(&mission.allowed_tools)?;
    if let Some(skill) = skill {
        scope.extend(skill.tool_defs.iter().map(|td| td.name.clone()));
    }
    Some(scope)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mission(tools: &[&str]) -> Mission {
        let md = format!(
            "---\nschedule: \"0 9 * * *\"\nenabled: true\nallowed-tools: [{}]\n---\nBody\n",
            tools.join(", ")
        );
        super::super::parser::parse_mission_md("m", &md).unwrap()
    }

    fn skill_with_tool(name: &str) -> Skill {
        let md = format!(
            "---\nname: cfo\ndescription: d\ntools:\n  - name: {name}\n    description: t\n    cmd: \"echo hi\"\n---\nBody\n"
        );
        crate::extensions::skills::parse_skill_text(&md, crate::engine::skill::SkillSource::Global)
            .unwrap()
    }

    #[test]
    fn empty_allowed_tools_means_unrestricted() {
        assert!(tool_scope(&mission(&[]), None).is_none());
    }

    #[test]
    fn listed_tools_become_the_allowlist_without_skill() {
        let set = tool_scope(&mission(&["Read", "Bash"]), None).unwrap();
        assert_eq!(set.len(), 2);
        assert!(!set.contains("Skill"), "missions never delegate to skills");
    }

    #[test]
    fn a_skill_missions_scope_includes_its_skills_tools() {
        let skill = skill_with_tool("LatestAnalysis");
        let set = tool_scope(&mission(&["WebFetch"]), Some(&skill)).unwrap();
        assert!(set.contains("WebFetch"));
        assert!(set.contains("LatestAnalysis"));
    }
}
