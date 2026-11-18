//! A turn's settings at the engine level: set when a turn begins, put back
//! when it ends — and put back when the next begins, whatever the last left.

use super::{Kickoff, RunSetup, TurnSettings};
use crate::engine::{AgentEngine, AgentRole, EngineConfig, InterfaceMode};

fn engine() -> AgentEngine {
    let cfg = EngineConfig::from_app_config(
        &crate::config::Config::default(),
        std::env::temp_dir(),
        InterfaceMode::Web,
    );
    let models = std::sync::Arc::new(crate::provider::models::ModelManager::new_with_credentials(
        Vec::new(),
        &crate::credentials::Credentials::default(),
    ));
    AgentEngine::new(cfg, models, "m".to_string(), AgentRole::Lead).unwrap()
}

/// A moment's turn: quiet, in another language, telling a fault, with a
/// mission's kickoff waiting.
fn moment() -> TurnSettings {
    TurnSettings {
        withheld: ["agent_chat".to_string()].into(),
        allowlist: Some(["sense".to_string()].into()),
        language: Some("zh-Hans".into()),
        fault: true,
        followups: true,
        kickoff: Kickoff {
            queue: vec!["next".into()],
            stop: vec!["DONE".into()],
            fresh: true,
            then: [("MORE".to_string(), vec!["again".to_string()])].into(),
        },
    }
}

#[test]
fn a_turn_runs_with_what_it_was_started_with() {
    let mut engine = engine();
    engine.begin_turn(moment());
    assert_eq!(engine.turn_settings(), moment());
    assert!(engine.is_withheld("agent_chat"));
    assert!(engine.is_withheld("Read"), "outside the allowlist");
}

#[test]
fn an_ended_turn_leaves_nothing_of_its_settings() {
    let mut engine = engine();
    engine.begin_turn(moment());
    engine.end_turn();
    assert_eq!(engine.turn_settings(), TurnSettings::default());
    assert!(!engine.is_withheld("agent_chat"));
}

#[test]
fn a_turn_begun_after_one_that_never_ended_has_only_its_own() {
    let mut engine = engine();
    engine.begin_turn(moment());
    engine.begin_turn(TurnSettings::default());
    assert_eq!(engine.turn_settings(), TurnSettings::default());
}

#[test]
fn what_a_turn_left_for_its_caller_stays_until_the_next_begins() {
    let mut engine = engine();
    engine.begin_turn(TurnSettings::default());
    engine.run_as(Some("run-1".into()));
    engine.last_assistant_text = Some("done".into());
    engine.end_turn();
    assert_eq!(engine.last_run_id.as_deref(), Some("run-1"));
    assert_eq!(engine.last_assistant_text.as_deref(), Some("done"));
    engine.begin_turn(TurnSettings::default());
    assert_eq!(engine.last_run_id, None);
    assert_eq!(engine.last_assistant_text, None);
}

#[test]
fn the_sessions_withheld_tools_outlive_a_turn() {
    let dir = tempfile::tempdir().unwrap();
    let store = crate::state_fs::sessions::SessionStore::with_sessions_dir(dir.path().into());
    let meta = crate::state_fs::sessions::SessionMeta {
        id: "sess-1-withheld".into(),
        withheld_tools: vec!["Write".into()],
        ..Default::default()
    };
    store.add_session(&meta).unwrap();
    let mut engine = engine();
    engine.begin_turn(moment());
    engine.withhold_session_tools(&store, Some("sess-1-withheld"));
    engine.end_turn();
    assert!(engine.is_withheld("Write"));
    assert!(!engine.is_withheld("agent_chat"));
}

#[tokio::test]
async fn a_run_that_fails_puts_back_its_wiring() {
    let mut engine = engine();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    engine.begin_run(RunSetup::task("x").silence_ok(true).thinking(Some(tx)));
    assert!(engine.silence_ok && engine.thinking_tx.is_some());
    engine.task = None;
    assert!(engine.run_agent_loop(None).await.is_err());
    assert!(!engine.silence_ok);
    assert!(engine.thinking_tx.is_none());
}

/// Whoever ran a planning loop still sees it planning when the loop returns
/// — no next-prompt hint is offered for a plan — and the turn's end ends it.
#[tokio::test]
async fn plan_mode_holds_past_the_loop_until_the_turn_ends() {
    let mut engine = engine();
    engine.begin_turn(TurnSettings::default());
    engine.begin_run(RunSetup::task("x").planning());
    engine.task = None;
    assert!(engine.run_agent_loop(None).await.is_err());
    assert!(engine.plan_mode, "the caller's check after the loop");
    engine.end_turn();
    assert!(!engine.plan_mode);
    engine.begin_run(RunSetup::task("y").planning());
    engine.begin_run(RunSetup::task("z"));
    assert!(!engine.plan_mode, "the next run sets its own");
}

/// A moment named its language; a light line after the moment's turn ended
/// answers in the Linggen language, not the moment's.
#[test]
fn a_moments_language_does_not_reach_a_light_line_after_it() {
    let mut engine = engine();
    let linggen = engine.light_system_prompt();
    engine.begin_turn(TurnSettings {
        language: Some("zz-Test".into()),
        ..Default::default()
    });
    assert_ne!(
        engine.light_system_prompt(),
        linggen,
        "the moment's own line"
    );
    engine.end_turn();
    assert_eq!(engine.light_system_prompt(), linggen);
}

#[test]
fn a_run_takes_its_task_and_drops_what_the_last_observed() {
    let mut engine = engine();
    engine
        .observations
        .push(crate::engine::types::ObservationRecord {
            observation_type: "t".into(),
            name: "n".into(),
            content: "c".into(),
        });
    engine.begin_run(RunSetup::task("now").images(vec!["img".into()]));
    assert_eq!(engine.task.as_deref(), Some("now"));
    assert!(engine.observations.is_empty());
    assert_eq!(engine.pending_images, vec!["img".to_string()]);
    engine.begin_run(RunSetup::task("again"));
    assert_eq!(
        engine.pending_images,
        vec!["img".to_string()],
        "kept when none given"
    );
}
