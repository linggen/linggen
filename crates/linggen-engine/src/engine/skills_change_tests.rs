//! A change of skills on disk reaches every live engine at its next turn and
//! takes nothing else from it (`AgentManager::mark_skills_changed`,
//! `AgentEngine::read_changes_if_any`). The server's one follow-up,
//! `server::api::skills::skills_changed`, is the only caller.

use super::{AgentEngine, AgentRole, EngineConfig, InterfaceMode};
use crate::engine::agent::AgentManager;
use crate::engine::skill::{Skill, SkillRegistry};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

/// Skills whose listing a test rewrites, as a save on disk would.
#[derive(Default)]
struct Listed(Mutex<Vec<(String, String, bool)>>);

#[async_trait]
impl SkillRegistry for Listed {
    async fn get_skill(&self, _: &str) -> Option<Skill> {
        None
    }
    async fn reload_one(&self, _: &str) -> Option<Skill> {
        None
    }
    async fn list_skills(&self) -> Vec<Skill> {
        Vec::new()
    }
    async fn match_trigger(&self, _: &str) -> Option<(String, String)> {
        None
    }
    async fn list_metadata(&self) -> Vec<(String, String, bool)> {
        self.0.lock().unwrap().clone()
    }
}

fn names(engine: &AgentEngine) -> Vec<&str> {
    engine
        .available_skills_metadata
        .iter()
        .map(|(n, _, _)| n.as_str())
        .collect()
}

fn engine() -> AgentEngine {
    let root = std::env::temp_dir();
    let cfg =
        EngineConfig::from_app_config(&crate::config::Config::default(), root, InterfaceMode::Web);
    let models = Arc::new(crate::provider::models::ModelManager::new_with_credentials(
        Vec::new(),
        &crate::credentials::Credentials::default(),
    ));
    AgentEngine::new(cfg, models, "m".to_string(), AgentRole::Lead).unwrap()
}

/// A skill saved while a session's turn runs: the save neither waits on the
/// turn nor drops its engine — the session is still found on the same
/// engine, with what the person declined — and its next turn lists the new
/// skill.
#[tokio::test]
async fn a_change_of_skills_keeps_every_live_engine() {
    let listed = Arc::new(Listed::default());
    listed
        .0
        .lock()
        .unwrap()
        .push(("old".into(), "Old.".into(), false));
    let empty = || Arc::new(crate::engine::test_registries::Empty);
    let (manager, _events) = AgentManager::new(
        crate::config::Config::default(),
        None,
        listed.clone(),
        empty(),
        empty(),
        InterfaceMode::Web,
    );
    let mut built = engine();
    built.read_changes_if_any(&manager).await;
    let declined = std::env::temp_dir().join("declined-here");
    built.declined_scopes.decline(&declined.to_string_lossy());
    let live = Arc::new(tokio::sync::Mutex::new(built));
    manager.insert_session_engine("s", "ling", live.clone()).await;

    // The turn holds its engine; a skill is saved meanwhile.
    let turn = live.lock().await;
    listed
        .0
        .lock()
        .unwrap()
        .push(("new".into(), "New.".into(), false));
    // Not async: it cannot wait on the turn's lock.
    manager.mark_skills_changed();
    let found = manager.session_engines_of("s").await;
    assert_eq!(found.len(), 1, "the session lost its engine");
    assert!(
        Arc::ptr_eq(&found[0], &live),
        "the session is on another engine"
    );
    drop(turn);

    // Its next turn: the new skill is listed, the decline is still held.
    let mut engine = live.lock().await;
    assert_eq!(names(&engine), ["old"]);
    engine.read_changes_if_any(&manager).await;
    assert_eq!(names(&engine), ["old", "new"]);
    assert!(engine
        .declined_scopes
        .covering(&declined.to_string_lossy())
        .is_some());

    // No change since: the list is not read again.
    listed.0.lock().unwrap().clear();
    engine.read_changes_if_any(&manager).await;
    assert_eq!(names(&engine), ["old", "new"]);
}
