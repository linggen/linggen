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

// ---- a sign-in makes work possible ----

/// A model signing in with ChatGPT, or (`false`) holding its own key.
fn signed(id: &str, chatgpt: bool) -> ModelConfig {
    let (provider, auth) = if chatgpt {
        ("chatgpt", crate::config::AuthMode::ChatgptOauth.as_str())
    } else {
        ("acme", "api_key")
    };
    serde_json::from_value(serde_json::json!({
        "id": id, "provider": provider, "url": "https://x", "model": id, "auth_mode": auth,
    }))
    .unwrap()
}

/// Rule: the default cannot run and a ChatGPT sign-in is made — the first
/// ChatGPT model that can run becomes the default, whatever other ready
/// models come before it.
#[test]
fn a_chatgpt_sign_in_makes_the_first_chatgpt_model_the_default() {
    let key = signed("my-key-model", false);
    let sol = signed("gpt-x-sol", true);
    let luna = signed("gpt-x-luna", true);
    let ready = [&key, &sol, &luna];
    assert_eq!(
        pick_after_sign_in(false, &ready, SignIn::ChatGpt).as_deref(),
        Some("gpt-x-sol")
    );
}

/// Rule: a default that can run is never changed by a sign-in.
#[test]
fn a_default_that_can_run_is_kept_through_a_sign_in() {
    let sol = signed("gpt-x-sol", true);
    assert_eq!(pick_after_sign_in(true, &[&sol], SignIn::ChatGpt), None);
}

/// A sign-in that made nothing ready changes nothing.
#[test]
fn a_sign_in_that_readies_no_model_changes_nothing() {
    let key = signed("my-key-model", false);
    assert_eq!(pick_after_sign_in(false, &[&key], SignIn::ChatGpt), None);
    assert_eq!(pick_after_sign_in(false, &[], SignIn::LinggenAccount), None);
}

/// On the real registry: the default (Linggen Cloud, signed out in the test
/// home) cannot run, the keyed model can; a sign-in of the keyed kind picks
/// it, a missing default is "cannot run", and a ready default stays.
#[test]
fn after_sign_in_reads_the_registry() {
    let _board = crate::provider::models::board_test_guard();
    let keyed: ModelConfig = serde_json::from_value(serde_json::json!({
        "id": "after-sign-in-keyed", "provider": "acme",
        "url": "https://api.acme.example/v1", "model": "k", "api_key": "k",
    }))
    .unwrap();
    let m = ModelManager::new_with_credentials(vec![keyed], &crate::credentials::Credentials::default());
    let cloud = crate::provider::models::LINGGEN_CLOUD_MODEL_ID;
    assert_eq!(after_sign_in(&m, Some(cloud), SignIn::Key).as_deref(), Some("after-sign-in-keyed"));
    assert_eq!(after_sign_in(&m, None, SignIn::Key).as_deref(), Some("after-sign-in-keyed"));
    assert_eq!(after_sign_in(&m, Some("after-sign-in-keyed"), SignIn::Key), None);
}

/// Rule: the line a sign-in says is short.
#[test]
fn the_default_line_is_at_most_twenty_words() {
    let line = made_default_line("gpt-6.2-sol");
    assert_eq!(line, "gpt-6.2-sol is now your default model.");
    assert!(line.split_whitespace().count() <= 20);
}

/// Rule: a run that cannot start for want of a sign-in names a ready model,
/// or says what to do when none is; a guest is told nothing of the owner's
/// models; another refusal is left alone.
#[test]
fn a_sign_in_refusal_says_the_next_step_to_the_owner_only() {
    let _board = crate::provider::models::board_test_guard();
    let keyed: ModelConfig = serde_json::from_value(serde_json::json!({
        "id": "next-step-keyed", "provider": "acme",
        "url": "https://api.acme.example/v1", "model": "k", "api_key": "k",
    }))
    .unwrap();
    let refusal = || anyhow::anyhow!("AUTH_REQUIRED: Not signed in to linggen.dev for model 'x'.");
    let with_one = ModelManager::new_with_credentials(vec![keyed], &crate::credentials::Credentials::default());
    let said = say_next_step(refusal(), &with_one, Asker::Owner).to_string();
    assert!(said.ends_with("next-step-keyed is ready now; choose it as the model."), "{said}");
    let shared = vec!["gpt-6.2-sol".to_string()];
    let to_guest = say_next_step(refusal(), &with_one, Asker::Guest(&shared)).to_string();
    assert!(!to_guest.contains("next-step-keyed"), "{to_guest}");
    let other = say_next_step(anyhow::anyhow!("overloaded"), &with_one, Asker::Owner).to_string();
    assert_eq!(other, "overloaded");
    let none = ModelManager::new_with_credentials(vec![], &crate::credentials::Credentials::default());
    let said = say_next_step(refusal(), &none, Asker::Owner).to_string();
    assert!(said.ends_with(crate::provider::models::NONE_READY), "{said}");
}

/// A default only rate-limited for a minute still runs: a sign-in leaves it.
#[test]
fn a_default_that_is_only_rate_limited_is_kept_through_a_sign_in() {
    let _board = crate::provider::models::board_test_guard();
    let own = |id: &str| -> ModelConfig {
        serde_json::from_value(serde_json::json!({
            "id": id, "provider": "acme",
            "url": "https://api.acme.example/v1", "model": "k", "api_key": "k",
        }))
        .unwrap()
    };
    let m = ModelManager::new_with_credentials(
        vec![own("sign-in-limited"), own("sign-in-other")],
        &crate::credentials::Credentials::default(),
    );
    crate::provider::models::note_unavailable(
        "sign-in-limited",
        &anyhow::anyhow!("openai error (429 Too Many Requests): rate limit"),
    );
    assert_eq!(after_sign_in(&m, Some("sign-in-limited"), SignIn::Key), None);
    crate::provider::models::note_healthy("sign-in-limited");
}

/// The two switches, one answer: a turn may switch only when the routing
/// config's and the session's are both on; the session's alone is what says
/// "switching is off for this session". No session, or none stored, never
/// forbids; an engine with no manager reads only the config's.
#[tokio::test]
async fn the_two_switches_make_one_answer() {
    use crate::engine::agent::AgentManager;
    use crate::engine::{AgentEngine, AgentRole, EngineConfig, InterfaceMode};
    let root = tempfile::tempdir().unwrap();
    let config = crate::config::Config::default();
    let (mut manager, _events) = AgentManager::new(
        config.clone(),
        None,
        std::sync::Arc::new(crate::engine::test_registries::Empty),
        std::sync::Arc::new(crate::engine::test_registries::Empty),
        std::sync::Arc::new(crate::engine::test_registries::Empty),
        InterfaceMode::Web,
    );
    std::sync::Arc::get_mut(&mut manager)
        .expect("a fresh manager")
        .global_sessions = crate::state_fs::SessionStore::leaked(root.path().join("sessions"));
    for (id, flag) in [("s-on", None), ("s-yes", Some(true)), ("s-off", Some(false))] {
        manager
            .global_sessions
            .add_session(&crate::state_fs::sessions::SessionMeta {
                id: id.into(),
                model_fallback: flag,
                ..Default::default()
            })
            .unwrap();
    }
    let engine_with = |config_on: bool, with_manager: bool| {
        let cfg = EngineConfig::from_app_config(&config, root.path().into(), InterfaceMode::Web);
        let mut engine =
            AgentEngine::new(cfg, std::sync::Arc::new(models()), STANDING.into(), AgentRole::Lead)
                .unwrap();
        engine.agent_id = Some("ling".into());
        engine.allow_model_fallback = config_on;
        if with_manager {
            engine.set_manager_context(manager.clone());
        }
        engine
    };
    // (config, session) -> (may switch, the session's own)
    for (config_on, session, may, session_allows) in [
        (true, Some("s-on"), true, true),
        (true, Some("s-yes"), true, true),
        (true, Some("s-off"), false, false),
        (false, Some("s-on"), false, true),
        (false, Some("s-off"), false, false),
        (true, None, true, true),
        (false, None, false, true),
        (true, Some("nowhere"), true, true),
    ] {
        let got = engine_with(config_on, true).switching(session);
        assert_eq!(
            (got.may(), got.session_allows()),
            (may, session_allows),
            "config {config_on}, session {session:?}"
        );
    }
    let bare = engine_with(true, false).switching(Some("s-off"));
    assert!(bare.may(), "no manager, no session store to read");
}

/// R4 pin (e): an `agent_run` asks with the delegated run's rule, which
/// never switches (`DELEGATED_RUN_MAY_SWITCH`), as the owner. Whatever
/// standing model its door hands `resolve` (`engine.model_id` there, where a
/// chat's, a mission's and her door pass `default_model_id`) is never the
/// answer: an asked model that cannot run is kept and fails, named. So at
/// that door the two ids differing could not change what runs.
#[test]
fn a_delegated_runs_rule_never_answers_with_the_standing_model() {
    let m = models();
    let rule = owner(DELEGATED_RUN_MAY_SWITCH);
    for standing in ["gpt-6-luna", "gpt-6.1-terra", "ghost-standing"] {
        let kept = resolve(&m, Some("ghost"), standing, rule);
        assert_eq!((kept.model.as_str(), kept.outcome), ("ghost", Outcome::Kept(Why::NotHere)), "{standing}");
        let asked = resolve(&m, Some("gpt-6.1-terra"), standing, rule);
        assert_eq!((asked.model.as_str(), asked.outcome), ("gpt-6.1-terra", Outcome::Asked), "{standing}");
    }
}
