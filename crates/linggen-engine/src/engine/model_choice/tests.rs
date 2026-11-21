//! The rule, one row per case.

use super::*;

fn models() -> ModelManager {
    // The built-ins only: gpt-6.2-sol, gpt-6.1-terra, gpt-6-luna, cloud.
    ModelManager::new_with_credentials(vec![], &crate::credentials::Credentials::default())
}

const STANDING: &str = "gpt-6-luna";

fn owner(may_switch: bool) -> Rule<'static> {
    Rule {
        asker: Asker::Owner,
        may_switch,
    }
}

fn guest(shared: &[String], may_switch: bool) -> Rule<'_> {
    Rule {
        asker: Asker::Guest(shared),
        may_switch,
    }
}

#[test]
fn the_rule_one_row_per_case() {
    let m = models();
    let shared = vec!["gpt-6-sol".to_string()]; // a retired spelling of sol
    let none: Vec<String> = vec![];
    let rows: Vec<(&str, Option<&str>, Rule<'_>, &str, Outcome)> = vec![
        (
            "owner, asked, here",
            Some("gpt-6.1-terra"),
            owner(true),
            "gpt-6.1-terra",
            Outcome::Asked,
        ),
        (
            "owner, asked a retired id",
            Some("gpt-6-sol"),
            owner(true),
            "gpt-6.2-sol",
            Outcome::Asked,
        ),
        (
            "owner, asked, not here",
            Some("ghost"),
            owner(true),
            STANDING,
            Outcome::Replaced(Why::NotHere),
        ),
        (
            "owner, asked, not here, switching off",
            Some("ghost"),
            owner(false),
            "ghost",
            Outcome::Kept(Why::NotHere),
        ),
        (
            "owner, nothing asked",
            None,
            owner(true),
            STANDING,
            Outcome::Standing,
        ),
        (
            "owner, nothing asked, switching off",
            None,
            owner(false),
            STANDING,
            Outcome::Standing,
        ),
        (
            "guest, asked, shared",
            Some("gpt-6.2-sol"),
            guest(&shared, true),
            "gpt-6.2-sol",
            Outcome::Asked,
        ),
        (
            "guest, asked, not shared",
            Some("gpt-6.1-terra"),
            guest(&shared, true),
            "gpt-6.2-sol",
            Outcome::Replaced(Why::NotShared),
        ),
        (
            "guest, asked, not shared, switching off",
            Some("gpt-6.1-terra"),
            guest(&shared, false),
            "gpt-6.2-sol",
            Outcome::Replaced(Why::NotShared),
        ),
        (
            "guest, asked, not here",
            Some("ghost"),
            guest(&shared, true),
            "gpt-6.2-sol",
            Outcome::Replaced(Why::NotHere),
        ),
        (
            "guest, nothing asked",
            None,
            guest(&shared, true),
            "gpt-6.2-sol",
            Outcome::Standing,
        ),
        (
            "guest, room shares nothing, asked",
            Some("gpt-6.1-terra"),
            guest(&none, true),
            STANDING,
            Outcome::Replaced(Why::NothingShared),
        ),
        (
            "guest, room shares nothing, nothing asked",
            None,
            guest(&none, true),
            STANDING,
            Outcome::Replaced(Why::NothingShared),
        ),
    ];
    for (case, asked, rule, model, outcome) in rows {
        let choice = resolve(&m, asked, STANDING, rule);
        assert_eq!(
            (choice.model(), choice.outcome()),
            (model, outcome),
            "{case}"
        );
        assert_eq!(choice.is_asked(), outcome == Outcome::Asked, "{case}");
        assert_eq!(
            choice.clears_pin(),
            matches!(outcome, Outcome::Replaced(_)),
            "{case}"
        );
    }
}

/// Moved from the chat handler (`allowed_model`): a retired id runs, and is
/// pinned, as its successor; a guest runs only what the room shares.
#[test]
fn a_retired_model_runs_and_pins_as_its_successor() {
    let m = models();
    let shared = vec!["gpt-6-sol".to_string()];
    let live_sol = Ok("gpt-6.2-sol".to_string());
    assert_eq!(live(&m, Asker::Owner, "gpt-6-sol"), live_sol);
    assert_eq!(live(&m, Asker::Owner, "gpt-6.1-sol"), live_sol);
    assert_eq!(live(&m, Asker::Guest(&shared), "gpt-6.2-sol"), live_sol);
    assert_eq!(
        live(&m, Asker::Guest(&[]), "gpt-6.2-sol"),
        Err(Why::NotShared)
    );
    assert_eq!(live(&m, Asker::Owner, "no-such-model"), Err(Why::NotHere));
}

/// A model replaced before the turn is told as one replaced during it: the
/// same ModelFallback event, naming both models and why.
#[tokio::test]
async fn a_model_replaced_before_the_turn_is_told() {
    use crate::engine::agent::{AgentEvent, AgentManager};
    use crate::engine::{AgentEngine, AgentRole, EngineConfig, InterfaceMode};
    let root = tempfile::tempdir().unwrap();
    let config = crate::config::Config::default();
    let cfg = EngineConfig::from_app_config(&config, root.path().into(), InterfaceMode::Web);
    let mut engine = AgentEngine::new(
        cfg,
        std::sync::Arc::new(models()),
        STANDING.into(),
        AgentRole::Lead,
    )
    .unwrap();
    engine.agent_id = Some("ling".into());
    let (manager, mut events) = AgentManager::new(
        config,
        None,
        std::sync::Arc::new(crate::engine::test_registries::Empty),
        std::sync::Arc::new(crate::engine::test_registries::Empty),
        std::sync::Arc::new(crate::engine::test_registries::Empty),
        InterfaceMode::Web,
    );
    engine.set_manager_context(manager);
    let told = |events: &mut tokio::sync::mpsc::UnboundedReceiver<_>| {
        std::iter::from_fn(|| events.try_recv().ok())
            .filter_map(|(event, _)| match event {
                AgentEvent::ModelFallback {
                    preferred_model,
                    actual_model,
                    reason,
                    ..
                } => Some((preferred_model, actual_model, reason)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    let replaced = resolve(
        engine.model_manager.as_ref(),
        Some("ghost"),
        STANDING,
        owner(true),
    );
    engine.run_on(&replaced, "test").await;
    assert_eq!(
        told(&mut events),
        [(
            "ghost".to_string(),
            STANDING.to_string(),
            Why::NotHere.says().to_string()
        )]
    );

    let asked = resolve(
        engine.model_manager.as_ref(),
        Some("gpt-6-sol"),
        STANDING,
        owner(true),
    );
    engine.run_on(&asked, "test").await;
    let kept = resolve(
        engine.model_manager.as_ref(),
        Some("ghost"),
        STANDING,
        owner(false),
    );
    engine.run_on(&kept, "test").await;
    assert!(
        told(&mut events).is_empty(),
        "nothing replaced, nothing told"
    );
}

/// An engine that starts on its own model (a Task child) under a guest's
/// policy runs on a model the room shares, as its parent does; an owner's
/// stays on its own.
#[tokio::test]
async fn an_engine_on_its_own_model_runs_a_shared_one_for_a_guest() {
    use crate::engine::session_policy::{ConsumerCeiling, SessionPolicy};
    use crate::engine::{AgentEngine, AgentRole, EngineConfig, InterfaceMode};
    let engine_on = |model: &str| {
        let root = std::env::temp_dir();
        let cfg = EngineConfig::from_app_config(
            &crate::config::Config::default(),
            root,
            InterfaceMode::Web,
        );
        AgentEngine::new(
            cfg,
            std::sync::Arc::new(models()),
            model.into(),
            AgentRole::Lead,
        )
        .unwrap()
    };
    let mut owner = engine_on("gpt-6.1-terra");
    owner.run_on_own_model("test").await;
    assert_eq!(owner.model_id, "gpt-6.1-terra");

    let mut child = engine_on("gpt-6.1-terra");
    SessionPolicy::consumer(ConsumerCeiling {
        models: vec!["gpt-6-sol".into()],
        ..Default::default()
    })
    .apply(&mut child);
    child.run_on_own_model("test").await;
    assert_eq!(child.model_id, "gpt-6.2-sol");
}

fn claude_config(json: serde_json::Value) -> ModelConfig {
    let mut base = serde_json::json!({
        "id": "claude-x", "provider": "anthropic",
        "url": "https://api.anthropic.com", "model": "claude-x",
    });
    base.as_object_mut().unwrap().extend(json.as_object().unwrap().clone());
    serde_json::from_value(base).unwrap()
}

/// A Claude conversation goes on only on an Anthropic model that has what it
/// needs: a key, or a Claude sign-in that is good now.
#[test]
fn only_an_anthropic_model_with_a_key_or_a_sign_in_runs_claude() {
    let keyed = claude_config(serde_json::json!({"api_key": "sk-test"}));
    let keyless = claude_config(serde_json::json!({"api_key": ""}));
    let mut signed = claude_config(serde_json::json!({}));
    signed.auth_mode = Some(crate::config::AuthMode::ClaudeOauth);
    let other = claude_config(serde_json::json!({"provider": "openai", "api_key": "sk-test"}));
    assert!(can_run_claude(&keyed, || false));
    assert!(!can_run_claude(&keyless, || true), "no key is no model");
    assert!(can_run_claude(&signed, || true));
    assert!(!can_run_claude(&signed, || false), "an expired sign-in is no model");
    assert!(!can_run_claude(&other, || true), "another vendor is not Claude");
}

/// The chain is walked in order; an id this machine does not have is passed
/// over, and none found is none.
#[test]
fn the_first_claude_model_of_the_chain_that_can_run_is_found() {
    let m = ModelManager::new_with_credentials(
        vec![
            claude_config(serde_json::json!({"id": "claude-b", "api_key": "sk-test-b"})),
            claude_config(serde_json::json!({"id": "claude-a", "api_key": "sk-test-a"})),
        ],
        &crate::credentials::Credentials::default(),
    );
    let chain = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect::<Vec<_>>();
    assert_eq!(first_claude(&m, &chain(&["gpt-6.2-sol", "ghost", "claude-a", "claude-b"])).as_deref(), Some("claude-a"));
    assert_eq!(first_claude(&m, &chain(&["gpt-6.2-sol", "gpt-6.1-terra"])), None);
}
