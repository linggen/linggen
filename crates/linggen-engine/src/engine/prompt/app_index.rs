//! The index of apps the prompt carries where `AppTool` is offered: one
//! line per installed app with an action offered anywhere (`reach:
//! anywhere`) — its name, what it is for in one short line, its actions'
//! names. Nothing per action beyond its name: a model that wants an app's
//! arguments asks `AppTool` for them. The one place it is made, beside the
//! skills listing (`skills_listing`); an app with no such action adds no
//! line, and with no such app the section is absent.
//!
//! It lists what a call would not refuse (`Tools::app_reachable`: the
//! person's own apps, a day's private ones kept out) and only where the tool
//! is offered — not in an app's own session, a mission, a delegate, nor a
//! quiet turn of Yinyue's.

use super::AgentEngine;
use crate::engine::skill::reach::Offer;
use crate::engine::skill::tools::anywhere_tools;
use std::collections::HashSet;

/// How much of an app's description the index keeps.
const ABOUT_CHARS: usize = 90;

impl AgentEngine {
    /// The section, or nothing. `allowed`: the tool set this turn is sent.
    pub(crate) fn app_index(&self, allowed: Option<&HashSet<String>>) -> String {
        use crate::prompts::keys;
        let offered = allowed.is_none_or(|set| set.contains("AppTool")) && !self.is_withheld("AppTool");
        if !offered {
            return String::new();
        }
        let own = self.active_skill.as_ref().map(|s| s.name.as_str());
        let entries: Vec<String> = self
            .anywhere_apps
            .iter()
            .filter(|skill| Some(skill.name.as_str()) != own)
            .filter(|skill| self.tools.builtins.app_reachable(skill, Offer::Anywhere).is_ok())
            .map(|skill| {
                let actions: Vec<&str> = anywhere_tools(skill).map(|t| t.name.as_str()).collect();
                self.prompt_store.render_or_fallback(
                    keys::SYSTEM_APP_ENTRY,
                    &[
                        ("name", skill.name.as_str()),
                        ("about", &one_line(&skill.description)),
                        ("actions", &actions.join(", ")),
                    ],
                )
            })
            .collect();
        if entries.is_empty() {
            return String::new();
        }
        let mut out = self.prompt_store.render_or_fallback(keys::SYSTEM_APPS_HEADER, &[]);
        out.extend(entries);
        out
    }
}

/// `text`'s first line, whitespace squeezed, cut to [`ABOUT_CHARS`].
fn one_line(text: &str) -> String {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let squeezed = first.split_whitespace().collect::<Vec<_>>().join(" ");
    match squeezed.chars().count() > ABOUT_CHARS {
        true => format!("{}…", squeezed.chars().take(ABOUT_CHARS).collect::<String>().trim_end()),
        false => squeezed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::skill::{Skill, SkillSource};
    use crate::engine::{AgentEngine, AgentRole, EngineConfig, InterfaceMode};
    use std::sync::Arc;

    #[test]
    fn a_description_is_one_short_line() {
        assert_eq!(one_line("  Plays   music\non this Mac. "), "Plays music");
        assert_eq!(one_line(""), "");
        let long = "x".repeat(200);
        let cut = one_line(&long);
        assert_eq!(cut.chars().count(), ABOUT_CHARS + 1, "{cut}");
        assert!(cut.ends_with('…'));
        assert_eq!(one_line("日本語".repeat(40).as_str()).chars().count(), ABOUT_CHARS + 1);
    }

    fn app(name: &str, extra: &str) -> Skill {
        let text = format!(
            "---\nname: {name}\ndescription: Plays music\n  on this Mac.\n{extra}tools:\n\
             \x20 - name: List\n    description: d\n    cmd: \"echo\"\n    tier: read\n    reach: anywhere\n\
             \x20 - name: Play\n    description: d\n    cmd: \"echo\"\n    tier: edit\n    reach: anywhere\n\
             \x20 - name: Tune\n    description: d\n    cmd: \"echo\"\n    tier: read\n---\nbody\n"
        );
        crate::extensions::skills::parse_skill_text(&text, SkillSource::Project).unwrap()
    }

    fn engine(apps: Vec<Skill>) -> AgentEngine {
        let cfg = EngineConfig::from_app_config(&crate::config::Config::default(), std::env::temp_dir(), InterfaceMode::Web);
        let models = Arc::new(crate::provider::models::ModelManager::new_with_credentials(
            Vec::new(),
            &crate::credentials::Credentials::default(),
        ));
        let mut engine = AgentEngine::new(cfg, models, "m".to_string(), AgentRole::Lead).unwrap();
        engine.anywhere_apps = apps;
        engine
    }

    /// One line per app: its name, its description in one line, the names
    /// of what it offers anywhere — and nothing of what it keeps to itself.
    #[test]
    fn an_app_is_one_line_with_the_names_of_its_actions_offered_anywhere() {
        let engine = engine(vec![app("jukebox", ""), app("radio", "")]);
        let index = engine.app_index(None);
        assert!(index.contains("## Apps you can reach"), "{index}");
        assert!(index.contains("- **jukebox**: Plays music on this Mac. Actions: List, Play."), "{index}");
        assert!(index.contains("- **radio**:"), "{index}");
        assert!(!index.contains("Tune"), "an action kept here is not named: {index}");
        assert_eq!(index.matches("\n- ").count(), 2, "one line each: {index}");
        assert_eq!(engine_without_apps().app_index(None), "", "no app, no section");
    }

    fn engine_without_apps() -> AgentEngine {
        engine(Vec::new())
    }

    /// The section is where `AppTool` is offered, and nowhere else.
    #[test]
    fn the_index_is_only_where_apptool_is_offered() {
        let mut e = engine(vec![app("jukebox", "")]);
        let set = |tools: &[&str]| tools.iter().map(|t| t.to_string()).collect::<HashSet<String>>();
        assert!(!e.app_index(Some(&set(&["Read", "AppTool"]))).is_empty());
        assert_eq!(e.app_index(Some(&set(&["Read"]))), "", "a set without it");
        e.begin_turn(crate::engine::turn_place::TurnSettings {
            allowlist: Some(set(&["sense", "recent_activity"])),
            ..Default::default()
        });
        assert_eq!(e.app_index(None), "", "her quiet turn's four tools");
        e.end_turn();
        e.active_mission = Some(crate::engine::types::ActiveMission { body: "Check.".into(), mission_dir: None });
        assert_eq!(e.app_index(None), "", "a mission has no AppTool");
        assert!(e.is_withheld("AppTool") && e.is_withheld("app_tool"));
    }

    /// In an app's own session its lead has no `AppTool`, so no index: that
    /// is its app's place, and what it needs of it is its own tools.
    #[tokio::test]
    async fn an_apps_own_session_has_no_apptool_and_no_index() {
        let mut e = engine(vec![app("jukebox", "")]);
        let own = crate::extensions::skills::parse_skill_text(
            "---\nname: dice\ndescription: d\napp:\n  launcher: web\n  entry: index.html\n---\nbody",
            SkillSource::Project,
        )
        .unwrap();
        assert!(!e.is_withheld("AppTool"));
        e.activate_skill(own, crate::engine::ActivationMode::SessionBound).await;
        assert!(e.is_withheld("AppTool"), "its lead has none");
        assert_eq!(e.app_index(None), "");
    }

    /// What a call would refuse is not indexed: an app the person was not
    /// given, a private app in the day, a user-only app.
    #[test]
    fn the_index_leaves_out_what_a_call_would_refuse() {
        let mut e = engine(vec![
            app("jukebox", ""),
            app("diary", "private-talk: true\n"),
            app("tutor", "disable-model-invocation: true\n"),
        ]);
        e.tools.builtins.session_id = Some("sess-day-2026-11-21".into());
        let index = e.app_index(None);
        assert!(index.contains("jukebox") && !index.contains("diary") && !index.contains("tutor"), "{index}");
        e.tools.builtins.session_id = Some("sess-1790000009-chat".into());
        assert!(e.app_index(None).contains("diary"), "outside the day a private app is reached");
        crate::engine::session_policy::SessionPolicy::person(vec!["diary".into()], vec![]).apply(&mut e);
        let index = e.app_index(None);
        assert!(index.contains("diary") && !index.contains("jukebox"), "{index}");
    }
}
