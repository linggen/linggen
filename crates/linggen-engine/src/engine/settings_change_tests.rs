//! A change of settings or models reaches every live engine at its next turn
//! and takes nothing else from it (`AgentManager::mark_settings_changed`,
//! `AgentEngine::read_changes_if_any`), as a change of skills does
//! (`skills_change_tests`). The one counter is `changes_generation`.

use super::{AgentEngine, AgentRole, EngineConfig, InterfaceMode};
use crate::config::{Config, ModelConfig};
use crate::engine::agent::AgentManager;
use std::sync::Arc;

fn model(id: &str) -> ModelConfig {
    ModelConfig {
        id: id.into(),
        provider: "ollama".into(),
        url: "http://127.0.0.1:9".into(),
        model: id.into(),
        api_key: None,
        keep_alive: None,
        context_window: Some(32_000),
        tags: vec![],
        supports_tools: Some(true),
        auth_mode: None,
        reasoning_effort: None,
        max_output_tokens: None,
        provided_by: None,
        is_builtin: false,
        family_binding: None,
        price: None,
        price_lookup: None,
    }
}

fn config(models: &[&str], chain: &[&str], max_iters: usize) -> Config {
    let mut c = Config::default();
    c.models = models.iter().map(|m| model(m)).collect();
    c.routing.default_models = chain.iter().map(|m| m.to_string()).collect();
    c.agent.max_iters = max_iters;
    c
}

/// A scratch folder the config is saved into — never the real one.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("settings-change-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn manager_of(config: Config, dir: &std::path::Path) -> Arc<AgentManager> {
    let empty = || Arc::new(crate::engine::test_registries::Empty);
    let (manager, _events) = AgentManager::new(
        config,
        Some(dir.to_path_buf()),
        empty(),
        empty(),
        empty(),
        InterfaceMode::Web,
    );
    manager
}

/// An engine built as the manager builds one: from its config and models.
async fn built(manager: &AgentManager, model_id: &str) -> AgentEngine {
    let config = manager.get_config_snapshot().await;
    let models = manager.models.read().await.clone();
    let mut engine = AgentEngine::new(
        EngineConfig::from_app_config(&config, std::env::temp_dir(), InterfaceMode::Web),
        models,
        model_id.to_string(),
        AgentRole::Lead,
    )
    .unwrap();
    engine.default_models = config.routing.default_models.clone();
    engine
}

fn listed(engine: &AgentEngine, id: &str) -> bool {
    engine
        .model_manager
        .list_models()
        .iter()
        .any(|m| m.id == id)
}

/// Settings saved while a session's turn runs: the save neither waits on the
/// turn nor drops its engine — the session is on the same engine, with what
/// the person declined.
#[tokio::test]
async fn a_change_of_config_keeps_every_live_engine() {
    let dir = scratch("keeps");
    let manager = manager_of(config(&["old", "keep"], &["old"], 10), &dir);
    let mut engine = built(&manager, "old").await;
    let declined = std::env::temp_dir().join("declined-here");
    engine.declined_scopes.decline(&declined.to_string_lossy());
    let live = Arc::new(tokio::sync::Mutex::new(engine));
    manager
        .insert_session_engine("s", "ling", live.clone())
        .await;

    let turn = live.lock().await;
    manager
        .apply_config(config(&["keep"], &["keep"], 20))
        .await
        .unwrap();
    let found = manager.session_engines_of("s").await;
    assert_eq!(found.len(), 1, "the session lost its engine");
    assert!(
        Arc::ptr_eq(&found[0], &live),
        "the session is on another engine"
    );
    drop(turn);
    assert!(live
        .lock()
        .await
        .declined_scopes
        .covering(&declined.to_string_lossy())
        .is_some());
}

/// Its next turn: the model list is the new one, the removed model is not
/// the turn's, the chain, the limits and the default model are the new
/// ones — and what the session built up is still there. What the default
/// permission mode is for new sessions does not move a live one.
#[tokio::test]
async fn a_turn_after_a_change_of_config_runs_on_the_new_settings() {
    let dir = scratch("reads");
    let manager = manager_of(config(&["old", "keep"], &["old"], 10), &dir);
    let mut engine = built(&manager, "old").await;
    engine.agent_id = Some("ling".into());
    let declined = std::env::temp_dir().join("declined-here");
    engine.declined_scopes.decline(&declined.to_string_lossy());
    let permission = engine.cfg.permission_mode;
    let live = Arc::new(tokio::sync::Mutex::new(engine));
    manager
        .insert_session_engine("s", "ling", live.clone())
        .await;

    let mut next = config(&["keep"], &["keep"], 20);
    next.agent.default_permission_mode = Some(crate::engine::permission::PermissionMode::Admin);
    manager.apply_config(next).await.unwrap();

    let mut engine = live.lock().await;
    // Before the turn reads, it is as it was built: the running turn's.
    assert!(listed(&engine, "old"));
    assert_eq!(engine.cfg.max_iters, 10);
    engine.read_live_state(&manager).await;
    assert!(!listed(&engine, "old"), "a removed model is still here");
    assert!(listed(&engine, "keep"));
    assert_eq!(engine.default_models, ["keep"]);
    assert_eq!(engine.cfg.max_iters, 20);
    assert_eq!(engine.default_model_id, "keep");
    assert_eq!(
        engine.model_id, "keep",
        "the turn would run on a removed model"
    );
    assert_eq!(
        engine.cfg.permission_mode, permission,
        "a live session's mode moved"
    );
    assert!(engine
        .declined_scopes
        .covering(&declined.to_string_lossy())
        .is_some());
}

/// A member's own recall (the companion's) is not the config's: a change of
/// config does not overwrite it.
#[tokio::test]
async fn a_change_of_config_leaves_a_members_tuned_recall() {
    let dir = scratch("recall");
    let manager = manager_of(config(&["m"], &["m"], 10), &dir);
    let mut engine = built(&manager, "m").await;
    engine.tune_recall(1, Some(0.8));
    let mut next = config(&["m"], &["m"], 10);
    next.agent.memory_recall_count = 7;
    manager.apply_config(next).await.unwrap();
    engine.read_live_state(&manager).await;
    assert_eq!(engine.cfg.memory_recall_count, 1);
}

/// The lock. The session engines are dropped with their session
/// (`remove_session_engine`) and at no other time: a settings, models or
/// skills change is `mark_settings_changed` / `mark_skills_changed`, and each
/// engine reads it at its next turn (`read_changes_if_any`). The field is
/// private to `engine::agent`; this holds the module itself to it.
#[test]
fn session_engines_are_never_cleared_by_a_change() {
    let text = include_str!("agent/mod.rs");
    let mut drops = Vec::new();
    for (at, _) in text.match_indices("session_engines") {
        let line_start = text[..at].rfind('\n').map_or(0, |n| n + 1);
        let line = text[line_start..].lines().next().unwrap_or("");
        let after = &text[at..(at + 120).min(text.len())];
        let cut =
            after.contains(".clear()") || after.contains(".drain(") || after.contains(".take(");
        let retain = after.contains(".retain(");
        if line.trim_start().starts_with("//") {
            continue;
        }
        if cut || retain {
            drops.push(after.split_whitespace().collect::<Vec<_>>().join(" "));
        }
    }
    assert_eq!(
        drops.len(),
        1,
        "session engines are dropped only with their session (`remove_session_engine`). A \
         change of settings, models or skills goes through `mark_settings_changed` / \
         `mark_skills_changed` and each engine reads it at its next turn \
         (`read_changes_if_any`). Found: {drops:#?}"
    );
    assert!(
        drops[0].contains("retain"),
        "the one drop is `remove_session_engine`'s"
    );
}

/// Who marks a change of settings: the two places the config or the model
/// list is replaced — `apply_config` and the server's `reload_models`.
#[test]
fn a_change_of_settings_is_marked_by_the_two_places_that_replace_them() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut found = Vec::new();
    for dir in ["src", "crates/linggen-engine/src"] {
        collect(&root.join(dir), &root, &mut found);
    }
    found.sort();
    assert_eq!(
        found,
        [
            (
                "crates/linggen-engine/src/engine/agent/mod.rs".to_string(),
                1
            ),
            ("src/server/api/config.rs".to_string(), 1)
        ],
        "mark a change of settings only where the config or models are replaced \
         (`apply_config`, `reload_models`); never drop engines to apply it"
    );
}

fn collect(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<(String, usize)>) {
    let needle = ["mark_settings_changed", "()"].concat();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .to_string();
            if rel.ends_with("settings_change_tests.rs") {
                continue;
            }
            let n = std::fs::read_to_string(&path)
                .unwrap()
                .matches(needle.as_str())
                .count();
            if n > 0 {
                out.push((rel, n));
            }
        }
    }
}

// The entrance of a session turn (`turn_place::entrance`): each step a door
// may not take is named for the reason it does not.
mod entrance {
    use super::*;
    use crate::engine::turn_place::entrance::{
        LiveState, MaySwitch, ModelStep, Seat, SessionOnTools, SessionTurn,
    };
    use crate::engine::model_choice::AskedBy;
    use crate::engine::turn_place::TurnSettings;

    fn turn<'a>(manager: &'a AgentManager, session: Option<&'a str>) -> SessionTurn<'a> {
        SessionTurn {
            manager,
            session,
            settings: TurnSettings::default(),
            session_on_tools: SessionOnTools::NotBound,
            live_state: LiveState::ReadAtTheDoor,
            model: ModelStep::ChosenByTheDoor,
            seat: Seat::NoSession,
        }
    }

    /// A door that does not read the live state keeps what the engine last
    /// read — what a task's end, a delivery and a plan's run do before their
    /// loop (which reads a change itself when it starts); one that reads gets
    /// the new settings, and the model list.
    #[tokio::test]
    async fn the_entrance_reads_live_state_only_for_a_door_that_asks() {
        let dir = scratch("entrance-live");
        let manager = manager_of(config(&["old", "keep"], &["old"], 10), &dir);
        let mut engine = built(&manager, "old").await;
        engine.agent_id = Some("ling".into());
        manager.apply_config(config(&["keep"], &["keep"], 20)).await.unwrap();

        engine.open_session_turn(turn(&manager, None)).await;
        assert_eq!(engine.cfg.max_iters, 10);
        assert!(listed(&engine, "old"));
        engine.end_turn();

        let mut reading = turn(&manager, None);
        reading.live_state = LiveState::Read;
        engine.open_session_turn(reading).await;
        assert_eq!(engine.cfg.max_iters, 20);
        assert!(!listed(&engine, "old"));
    }

    /// The session goes on the engine's tools only when the door binds it.
    #[tokio::test]
    async fn the_entrance_binds_the_session_on_the_tools_only_for_a_door_that_asks() {
        let dir = scratch("entrance-session");
        let manager = manager_of(config(&["m"], &["m"], 10), &dir);
        let mut engine = built(&manager, "m").await;
        engine.open_session_turn(turn(&manager, Some("sess-x"))).await;
        assert_eq!(engine.tools.builtins.session_id, None);
        let mut binding = turn(&manager, Some("sess-x"));
        binding.session_on_tools = SessionOnTools::Bound;
        engine.open_session_turn(binding).await;
        assert_eq!(engine.tools.builtins.session_id.as_deref(), Some("sess-x"));
    }

    /// An asked model that cannot run gives way to the agent's default
    /// (`default_model_id`), never to the model the engine was left on.
    #[tokio::test]
    async fn the_entrance_gives_an_unrunnable_model_way_to_the_agents_default() {
        let dir = scratch("entrance-standing");
        let manager = manager_of(config(&["a", "b"], &["a"], 10), &dir);
        // An engine on "b" whose agent's default is "a".
        let mut engine = built(&manager, "b").await;
        engine.default_model_id = "a".into();
        let mut turn = turn(&manager, None);
        turn.model = ModelStep::Chosen {
            asked: Some("ghost"),
            asked_by: None,
            may_switch: MaySwitch::SessionAllows,
            place: "test".into(),
        };
        engine.open_session_turn(turn).await;
        assert_eq!(engine.model_id, "a");
    }

    /// Who asks is what the door hands the entrance, else what the engine's
    /// policy says: a guest is given the models the room shares.
    #[tokio::test]
    async fn the_entrance_asks_as_the_door_hands_it() {
        let dir = scratch("entrance-asker");
        let manager = manager_of(config(&["a", "b"], &["a"], 10), &dir);
        let asking = |by: Option<AskedBy>| {
            let mut t = turn(&manager, None);
            t.model = ModelStep::Chosen {
                asked: Some("a"),
                asked_by: by,
                may_switch: MaySwitch::SessionAllows,
                place: "test".into(),
            };
            t
        };
        let mut engine = built(&manager, "b").await;
        engine.default_model_id = "b".into();
        engine.open_session_turn(asking(None)).await;
        assert_eq!(engine.model_id, "a", "the engine's policy names the owner");
        let mut engine = built(&manager, "b").await;
        engine.default_model_id = "b".into();
        engine.open_session_turn(asking(Some(AskedBy::of(Some(vec!["b".into()]))))).await;
        assert_eq!(engine.model_id, "b", "a guest is given only what the room shares");
    }

    /// The turn's own settings come first and are put back by `end_turn`,
    /// whatever a door does with the rest.
    #[tokio::test]
    async fn the_entrance_begins_the_turn_with_the_doors_settings() {
        let dir = scratch("entrance-begin");
        let manager = manager_of(config(&["m"], &["m"], 10), &dir);
        let mut engine = built(&manager, "m").await;
        let mut person = turn(&manager, None);
        person.settings = TurnSettings { person_spoke: true, language: Some("zh".into()), ..Default::default() };
        engine.open_session_turn(person).await;
        assert!(engine.turn_settings().person_spoke);
        engine.end_turn();
        assert_eq!(engine.turn_settings(), TurnSettings::default());
    }
}
