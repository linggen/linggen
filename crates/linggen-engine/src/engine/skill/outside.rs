//! What an agent outside Linggen — Claude Code, Codex — is offered of an
//! app, through the MCP door's `app` tool, for the owner at this Mac. The
//! same list a session's `AppTool` lists (`reach: anywhere`, through
//! `tools::anywhere_tools`) and the same judge (`judge::judge`, with
//! `Asker::OutsideHost`); this module adds only what is closed to it.
//!
//! An app that keeps its talk private (`private-talk: true`) is closed to
//! an outside host until the owner opens it, in Settings → Tools & MCP: the
//! apps they opened are the engine config's `outside_apps`, handed in here
//! as `opened`. A closed app is not listed and not run; the person's own
//! apps and what a model may start are `tools::model_may_start`'s.

use super::judge::{judge, Asker};
use super::params::SessionParams;
use super::reach::Offer;
use super::tools::{
    action_of, anywhere_tools, granted_on, model_may_start, resolve_offered, run, working_folder,
    Refusal,
};
use super::Skill;
use crate::engine::skill_tool::SkillToolDef;
use crate::engine::tools::ToolResult;

/// Whether `skill` reaches an outside host only once the owner has opened
/// it: the one place that says which apps those are.
fn kept_from_outside(skill: &Skill) -> bool {
    skill.private_talk
}

/// Whether `skill` is an app the owner may open to outside hosts: one that
/// is kept from them and offers an action anywhere. The Settings list is
/// these.
pub fn can_be_opened(skill: &Skill) -> bool {
    kept_from_outside(skill) && anywhere_tools(skill).next().is_some()
}

/// Why `skill` is no app to an outside host, or `Ok`: an app only the
/// person starts, or a private one that is not among the `opened` apps.
pub fn open_to_host(skill: &Skill, opened: &[String]) -> Result<(), Refusal> {
    model_may_start(skill)?;
    if kept_from_outside(skill) && !opened.contains(&skill.name) {
        return Err(Refusal::NotYours(format!(
            "'{}' keeps its talk private; the owner opens it to outside agents in Settings → Tools & MCP",
            skill.name
        )));
    }
    Ok(())
}

/// The judge's verdict on `def` for an outside host: the one asker, built
/// here for the list and for the run.
fn judged(skill: &Skill, def: &SkillToolDef, offer: Offer) -> Result<(), Refusal> {
    let cwd = working_folder(skill);
    let action = action_of(skill, def, &cwd);
    let asker = Asker::OutsideHost {
        offer,
        granted: granted_on(skill, &cwd),
    };
    judge(asker, &action).allowed_or_refusal(&action)
}

/// What `skill` shows an outside host: the actions it offers anywhere that
/// the judge lets it run — none while the app is closed to it, and not a
/// destructive, admin or unclassed action, nor one its own grant does not
/// cover. (A session's `AppTool` lists more; a call refuses these the same.)
pub fn host_tools<'a>(skill: &'a Skill, opened: &[String]) -> Vec<&'a SkillToolDef> {
    if open_to_host(skill, opened).is_err() {
        return Vec::new();
    }
    anywhere_tools(skill)
        .filter(|def| judged(skill, def, Offer::to_caller(def, false)).is_ok())
        .collect()
}

/// Run `tool_name` of `skill` for an outside host, as the judge says it
/// may: found among what the app offers anywhere, then judged as the action
/// it is under the app's own grant. No session is bound.
pub async fn run_for_host(
    skill: &Skill,
    tool_name: &str,
    args: &serde_json::Value,
    opened: &[String],
) -> Result<ToolResult, Refusal> {
    open_to_host(skill, opened)?;
    let (def, offer) = resolve_offered(skill, tool_name, false, |_| Ok(()))?;
    judged(skill, def, offer)?;
    run(skill, tool_name, args, &SessionParams::new()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Offer` stays the one word for "what an app offers", here as the
    /// outside host is offered it: nothing hers alone.
    #[test]
    fn what_is_hers_alone_is_not_offered_to_an_outside_host() {
        let text = "---\nname: zz-out\ndescription: t\ntools:\n  - name: Peek\n    description: d\n    cmd: \"echo\"\n    tier: read\n    pet: true\n---\nb\n";
        let skill =
            crate::extensions::skills::parse_skill_text(text, super::super::SkillSource::Project)
                .unwrap();
        assert!(host_tools(&skill, &[]).is_empty());
        assert_eq!(Offer::to_caller(&skill.tool_defs[0], false), Offer::Nobody);
    }
}
