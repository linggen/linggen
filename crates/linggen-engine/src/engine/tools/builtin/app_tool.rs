//! AppTool — a model reaches an app's actions from a session that is not
//! the app's own. What it may reach is what the app offers it: any model
//! what the app declares `reach: anywhere`, the companion also what the
//! app marks `pet: true` to read (`skill::reach::Offer`). Whether it may
//! run one is `engine::skill::judge`'s, asked at the permission gate with
//! the app, the action and its class (`AppTool::call`) — not the prompt's.

use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::{ActionClass, PermissionMode};
use crate::engine::skill::reach::Offer;
use crate::engine::skill::tools::{
    action_row, anywhere_tools, model_may_start, pet_tools, resolve_offered, run, run_for_pet, Refusal,
};
use crate::engine::skill::Skill;
use crate::engine::skill_tool::SkillToolDef;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

#[derive(serde::Deserialize)]
struct AppToolArgs {
    #[serde(default)]
    app: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    args: Option<Value>,
}

pub struct AppTool;
#[async_trait]
impl Tool for AppTool {
    fn place_reach(&self) -> crate::engine::tools::PlaceReach {
        crate::engine::tools::PlaceReach::IntoApp
    }
    fn name(&self) -> &'static str {
        "AppTool"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["app_tool"]
    }
    fn description(&self) -> &'static str {
        "Do what an app offers from here. Call with `app` and no `tool` to \
         list that app's actions (tool, class, description, args); then call \
         with app + tool + args to run one."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "app": { "type": "string", "description": "The app (skill) name." },
                "tool": {
                    "type": "string",
                    "description": "The action to run. Omit to list the app's actions."
                },
                "args": { "type": "object", "description": "The action's args." }
            }
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "AppTool",
            "args": {"app": "string?", "tool": "string?", "args": "object?"},
            "returns": "the action's output, or the list of actions an app offers",
            "notes": "No `tool`: list what the app offers. Only actions an app offers you run."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: AppToolArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for AppTool: {e}"))?;
        let app = named(args.app.as_deref());
        let Some(tool) = named(args.tool.as_deref()) else {
            return Ok(ToolResult::Success(list_for(tools, app).await?));
        };
        let Some(app) = app else {
            anyhow::bail!("AppTool: name the `app` whose action '{tool}' to run");
        };
        let tool_args = args.args.unwrap_or_else(|| json!({}));
        if !tool_args.is_object() {
            anyhow::bail!("AppTool: `args` is the action's args, as an object");
        }
        let (skill, _, _) = tools.app_action(app, tool).await.map_err(|r| anyhow::anyhow!(r.to_string()))?;
        // Called from a session of this app: its bound values come along.
        let params = tools.session_params_for(Some(&skill.name));
        let ran = match tools.is_companion() {
            true => run_for_pet(&skill, tool, &tool_args, &params, tools.person_spoke).await,
            false => run(&skill, tool, &tool_args, &params).await,
        };
        ran.map_err(|r| anyhow::anyhow!(r.to_string()))
    }
}

/// The action an AppTool call names, as the permission gate asks about it.
pub(crate) struct AppAction {
    pub app: String,
    pub tool: String,
    pub class: ActionClass,
    /// What the app offers it as to this caller.
    pub offer: Offer,
    /// The app, whose folder the action runs in and whose own grants hold
    /// there.
    pub skill: Skill,
}

/// What an AppTool call is, to the permission gate.
pub(crate) enum AppCall {
    /// A listing, or a call that names no action: nothing inside to judge.
    Listing,
    /// The caller may not reach it at all, and why.
    Refused(String),
    /// An action to judge.
    Action(AppAction),
}

impl Tools {
    /// The AppTool call `args` as the gate sees it.
    pub(crate) async fn app_call(&self, args: &Value) -> AppCall {
        let named = |key: &str| named(args.get(key).and_then(Value::as_str)).map(str::to_string);
        let (Some(app), Some(tool)) = (named("app"), named("tool")) else {
            return AppCall::Listing;
        };
        match self.app_action(&app, &tool).await {
            Ok((skill, def, offer)) => AppCall::Action(AppAction { app, tool, class: def.class, offer, skill }),
            Err(refusal) => AppCall::Refused(refusal.to_string()),
        }
    }

    /// `app`'s action `tool` as this session's model is offered it, or why
    /// it is not: the one place a call is resolved, for the gate and for the
    /// run.
    async fn app_action(&self, app: &str, tool: &str) -> Result<(Skill, SkillToolDef, Offer), Refusal> {
        let manager = self
            .get_manager()
            .ok_or_else(|| Refusal::NoSkill("no apps to reach here".into()))?;
        let skill = manager.skills.get_skill(app).await.ok_or_else(|| Refusal::no_skill(app))?;
        let (def, offer) = resolve_offered(&skill, tool, self.is_companion(), |offer| {
            self.app_reachable(&skill, offer)
        })?;
        let def = def.clone();
        Ok((skill, def, offer))
    }

    /// An app that keeps this agent away is no app to it
    /// (`place.<agent>.absent_until`).
    pub(crate) fn app_present(&self, skill: &Skill) -> Result<(), Refusal> {
        let away = self
            .agent_id()
            .is_some_and(|agent| skill.keeps_away_in(agent, &self.session_params_for(Some(&skill.name))));
        match away {
            true => Err(Refusal::no_skill(&skill.name)),
            false => Ok(()),
        }
    }

    /// Whether this session's model may reach `skill`'s actions offered as
    /// `offer`. What is offered anywhere is also the
    /// person's to use (`People::may_use_app`, as the turn's policy holds
    /// it), and a model's to start; and not a private app's, which keeps a
    /// session of its own out of the day — as the Skill tool refuses it.
    pub(crate) fn app_reachable(&self, skill: &Skill, offer: Offer) -> Result<(), Refusal> {
        self.app_present(skill)?;
        if offer != Offer::Anywhere {
            return Ok(());
        }
        let app = &skill.name;
        if self.session_policy.as_ref().is_some_and(|p| !p.is_skill_allowed(app)) {
            return Err(Refusal::NotYours(format!("'{app}' is not one of the apps you may use")));
        }
        model_may_start(skill)?;
        if self.kept_out_of_day(skill) {
            return Err(Refusal::NotYours(format!(
                "refused: {app} keeps a session of its own; it is not reached from the day's session."
            )));
        }
        Ok(())
    }
}

/// A name the call gives: blank is none — the same as leaving it out
/// (seen 2026-09-24: `{app: "lingjing", tool: ""}` was refused as a tool
/// named '').
fn named(name: Option<&str>) -> Option<&str> {
    name.map(str::trim).filter(|n| !n.is_empty())
}

/// The listing a call with no action asks for: `app`'s, or every app's.
async fn list_for(tools: &Tools, app: Option<&str>) -> Result<String> {
    let manager = tools
        .get_manager()
        .ok_or_else(|| anyhow::anyhow!("AppTool: no apps to reach here"))?;
    let skills = match app {
        Some(app) => {
            let skill = manager
                .skills
                .get_skill(app)
                .await
                .ok_or_else(|| anyhow::anyhow!(Refusal::no_skill(app).to_string()))?;
            tools.app_present(&skill).map_err(|r| anyhow::anyhow!(r.to_string()))?;
            vec![skill]
        }
        None => manager.skills.list_skills().await,
    };
    let rows = listing(tools, &skills);
    // A named app that offers nothing says why, when it can.
    if let (true, Some(_), [skill]) = (rows.is_empty(), app, skills.as_slice()) {
        tools
            .app_reachable(skill, Offer::Anywhere)
            .map_err(|r| anyhow::anyhow!(r.to_string()))?;
    }
    Ok(rows_or_none(rows))
}

/// What the apps offer this caller, as JSON lines: the companion what she
/// may call in this turn, any other model what is offered anywhere and is
/// not closed to it ([`Tools::app_reachable`]).
fn listing(tools: &Tools, skills: &[Skill]) -> Vec<String> {
    let companion = tools.is_companion();
    skills
        .iter()
        .flat_map(|s| {
            let offered: Vec<&SkillToolDef> = match companion {
                true => pet_tools(s, tools.person_spoke).collect(),
                false => anywhere_tools(s).collect(),
            };
            offered
                .into_iter()
                .filter(|t| tools.app_reachable(s, Offer::to_caller(t, companion)).is_ok())
                .map(|t| action_row(s, t).to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn rows_or_none(rows: Vec<String>) -> String {
    match rows.is_empty() {
        true => "No app offers you an action right now.".to_string(),
        false => rows.join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::agent::AgentManager;
    use crate::engine::skill::SkillSource;
    use crate::engine::tools::ToolCall;
    use std::sync::Arc;

    #[test]
    fn a_blank_name_is_no_name() {
        assert_eq!(named(None), None);
        assert_eq!(named(Some("")), None);
        assert_eq!(named(Some("  ")), None);
        assert_eq!(named(Some(" Progress ")), Some("Progress"));
    }

    /// An app with a read, an edit and a destructive action offered
    /// anywhere, a read offered to her alone, and one kept here.
    fn jukebox(dir: &std::path::Path, extra: &str) -> Skill {
        let text = format!(
            "---\nname: jukebox\ndescription: Plays music.\n{extra}permission:\n  paths:\n    - {{ path: \"{dir}\", mode: edit }}\ntools:\n\
             \x20 - name: List\n    description: What is in the library.\n    cmd: \"echo listing\"\n    tier: read\n    reach: anywhere\n\
             \x20 - name: Play\n    description: Play a song.\n    cmd: \"echo playing\"\n    tier: edit\n    reach: anywhere\n\
             \x20 - name: Delete\n    description: Delete a song.\n    cmd: \"echo deleted\"\n    tier: destructive\n    reach: anywhere\n\
             \x20 - name: Peek\n    description: Hers.\n    cmd: \"echo peeked\"\n    tier: read\n    pet: true\n\
             \x20 - name: Tune\n    description: Here only.\n    cmd: \"echo tuned\"\n    tier: read\n---\nbody\n",
            dir = dir.display()
        );
        let mut skill = crate::extensions::skills::parse_skill_text(&text, SkillSource::Project).unwrap();
        skill.skill_dir = Some(dir.to_path_buf());
        skill
    }

    fn tools_for(dir: &std::path::Path, agent: &str, skills: Vec<Skill>) -> Tools {
        let (manager, _events) = AgentManager::new(
            crate::config::Config::default(),
            None,
            Arc::new(crate::engine::test_registries::Installed(skills)),
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            crate::engine::InterfaceMode::Web,
        );
        let mut tools = Tools::new(dir.to_path_buf()).unwrap();
        tools.set_context(manager, agent.into());
        tools
    }

    async fn call(tools: &Tools, args: Value) -> Result<String, String> {
        let call = ToolCall { tool: "AppTool".into(), args, block_id: None };
        match tools.execute(call).await {
            Ok(ToolResult::Success(text)) => Ok(text),
            Ok(ToolResult::CommandOutput { stdout, .. }) => Ok(stdout),
            Ok(other) => Ok(format!("{other:?}")),
            Err(e) => Err(e.to_string()),
        }
    }

    fn tools_named(listed: &str) -> Vec<String> {
        listed
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|row| row["tool"].as_str().map(String::from))
            .collect()
    }

    /// A model that is not the companion is offered what the app declares
    /// `reach: anywhere`, with each action's class — nothing hers alone,
    /// nothing kept in the app — and runs what it is offered.
    #[tokio::test]
    async fn a_model_is_offered_what_an_app_offers_anywhere() {
        let dir = tempfile::tempdir().unwrap();
        let tools = tools_for(dir.path(), "ling", vec![jukebox(dir.path(), "")]);
        let listed = call(&tools, json!({"app": "jukebox"})).await.unwrap();
        assert_eq!(tools_named(&listed), ["List", "Play", "Delete"], "{listed}");
        assert!(listed.contains("\"class\":\"destructive\""), "{listed}");
        let all = call(&tools, json!({})).await.unwrap();
        assert_eq!(tools_named(&all), ["List", "Play", "Delete"], "no app named lists every app's");

        assert!(call(&tools, json!({"app": "jukebox", "tool": "List"})).await.unwrap().contains("listing"));
        assert!(call(&tools, json!({"app": "jukebox", "tool": "Play"})).await.unwrap().contains("playing"));
        for hidden in ["Peek", "Tune"] {
            let why = call(&tools, json!({"app": "jukebox", "tool": hidden})).await.unwrap_err();
            assert_eq!(why, format!("'jukebox' does not offer '{hidden}' to you"));
        }
        let why = call(&tools, json!({"app": "jukebox", "tool": "Nope"})).await.unwrap_err();
        assert_eq!(why, "'jukebox' declares no tool 'Nope'");
        let why = call(&tools, json!({"tool": "Play"})).await.unwrap_err();
        assert!(why.contains("name the `app`"), "{why}");
    }

    /// A user-only app is never started by a model.
    #[tokio::test]
    async fn a_user_only_app_is_closed_to_a_model() {
        let dir = tempfile::tempdir().unwrap();
        let tools = tools_for(dir.path(), "ling", vec![jukebox(dir.path(), "disable-model-invocation: true\n")]);
        assert!(tools_named(&call(&tools, json!({})).await.unwrap()).is_empty());
        let why = call(&tools, json!({"app": "jukebox", "tool": "List"})).await.unwrap_err();
        assert!(why.contains("user-only"), "{why}");
    }

    /// A private app keeps a session of its own out of the day: from the
    /// day's session it is not reached, as the Skill tool refuses it; from
    /// any other session it is.
    #[tokio::test]
    async fn a_private_app_is_not_reached_from_the_day() {
        let dir = tempfile::tempdir().unwrap();
        let mut tools = tools_for(dir.path(), "ling", vec![jukebox(dir.path(), "private-talk: true\n")]);
        tools.session_id = Some("sess-day-2026-11-21".into());
        let why = call(&tools, json!({"app": "jukebox", "tool": "List"})).await.unwrap_err();
        assert!(why.contains("keeps a session of its own"), "{why}");
        assert!(tools_named(&call(&tools, json!({})).await.unwrap()).is_empty());
        tools.session_id = Some("sess-1790000009-chat".into());
        assert!(call(&tools, json!({"app": "jukebox", "tool": "List"})).await.is_ok());
    }

    /// The companion runs a read, and an edit action only in a turn the
    /// person started by asking her; a destructive one never.
    #[tokio::test]
    async fn the_companion_runs_an_edit_action_only_when_the_person_asked() {
        let dir = tempfile::tempdir().unwrap();
        let mut tools = tools_for(dir.path(), "yinyue", vec![jukebox(dir.path(), "")]);
        let play = json!({"app": "jukebox", "tool": "Play"});
        assert!(call(&tools, json!({"app": "jukebox", "tool": "Peek"})).await.unwrap().contains("peeked"));
        assert!(call(&tools, json!({"app": "jukebox", "tool": "List"})).await.unwrap().contains("listing"));
        let why = call(&tools, play.clone()).await.unwrap_err();
        assert!(why.contains("this turn is yours"), "{why}");
        let listed = call(&tools, json!({"app": "jukebox"})).await.unwrap();
        assert_eq!(tools_named(&listed), ["List", "Peek"], "her own turn offers her reads");

        tools.person_spoke = true;
        assert!(call(&tools, play).await.unwrap().contains("playing"));
        let listed = call(&tools, json!({"app": "jukebox"})).await.unwrap();
        assert_eq!(tools_named(&listed), ["List", "Play", "Peek"], "an asked turn adds the change");
        let why = call(&tools, json!({"app": "jukebox", "tool": "Delete"})).await.unwrap_err();
        assert!(why.contains("the person does this themselves, in the app"), "{why}");
    }

    /// An app whose world she hasn't entered yet offers her nothing —
    /// listed or called by name; everyone else is reached as before.
    #[tokio::test]
    async fn an_app_keeping_her_away_offers_her_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let away = "place:\n  yinyue:\n    absent_until: {file: state.json, path: companion}\n";
        let skills = vec![jukebox(dir.path(), away)];
        let hers = tools_for(dir.path(), "yinyue", skills.clone());
        assert!(tools_named(&call(&hers, json!({})).await.unwrap()).is_empty());
        let why = call(&hers, json!({"app": "jukebox", "tool": "Peek"})).await.unwrap_err();
        assert_eq!(why, "no skill 'jukebox'");
        let why = call(&hers, json!({"app": "jukebox"})).await.unwrap_err();
        assert_eq!(why, "no skill 'jukebox'");
        let his = tools_for(dir.path(), "ling", skills);
        assert!(call(&his, json!({"app": "jukebox", "tool": "List"})).await.is_ok());
    }
}
