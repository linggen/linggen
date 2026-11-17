//! The table test: for every kind of session made by hand today, the spec
//! built from its preset sets what the hand-made path sets.

use super::*;
use crate::engine::{AgentEngine, AgentRole, EngineConfig, InterfaceMode};
use crate::state_fs::sessions::SessionMember;
use std::collections::BTreeSet;
// The day as `server::day_session` keeps it; its test lists this file as
// one that makes a day's meta (a fixture, twice).
use crate::state_fs::day_session;

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

/// A fresh engine for `meta`, its creator written by the spec.
fn engine_for(meta: &SessionMeta) -> AgentEngine {
    let mut engine = engine();
    SessionSpec::unclaimed(Some(meta.creator.clone())).apply(&mut engine);
    engine
}

fn meta(id: &str, creator: &str, skill: Option<&str>) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        creator: creator.into(),
        skill: skill.map(str::to_string),
        agents: vec![SessionMember::new("ling")],
        ..Default::default()
    }
}

fn mission() -> crate::engine::mission::record::Mission {
    let md =
        "---\nschedule: \"0 9 * * *\"\nenabled: true\nallowed-tools: [Read, Bash]\n---\nBody\n";
    crate::extensions::missions::parser::parse_mission_md("nightly", md).unwrap()
}

/// What a turn of the session runs under once its loop has read the
/// session's `permission.json`.
#[derive(Debug, PartialEq)]
struct Seen {
    memory: bool,
    core: bool,
    task: bool,
    tools: Option<BTreeSet<String>>,
    interactive: bool,
    world: bool,
}

impl Seen {
    /// `on_disk`: the `interactive` the session's `permission.json` holds;
    /// `None` for a run that never loads it (a `Task` child).
    fn of(engine: &AgentEngine, on_disk: Option<bool>) -> Self {
        let restricted = engine.cfg.consumer_allowed_tools.is_some()
            || engine.cfg.mission_allowed_tools.is_some();
        let interactive = match on_disk {
            Some(file) => file && !restricted,
            None => engine.session_permissions.interactive,
        };
        let p = &engine.prompt_profile;
        Self {
            memory: p.include_memory,
            core: p.include_core,
            task: p.task_bootstrap,
            tools: engine
                .cfg
                .mission_allowed_tools
                .clone()
                .map(|s| s.into_iter().collect()),
            interactive,
            world: crate::engine::prompt::world::world_rides(
                engine.session_creator.as_deref(),
                p.consumer_frame,
            ),
        }
    }
}

fn set(names: &[&str]) -> Option<BTreeSet<String>> {
    Some(names.iter().map(|s| s.to_string()).collect())
}

/// The literal each kind of session is expected to run under.
fn expected(
    memory: bool,
    core: bool,
    task: bool,
    tools: Option<BTreeSet<String>>,
    interactive: bool,
    world: bool,
) -> Seen {
    Seen {
        memory,
        core,
        task,
        tools,
        interactive,
        world,
    }
}

fn delegate_read() -> Option<BTreeSet<String>> {
    delegate::tool_scope(false).map(|s| s.into_iter().collect())
}

fn typed_by_spec(meta: &SessionMeta, turn_owner: &str) -> AgentEngine {
    let mut e = engine_for(meta);
    SessionSpec::of(meta)
        .typed(SessionPolicy::owner(), Memory::of_turn(turn_owner))
        .apply(&mut e);
    e
}

/// The rows of the inventory a person's turn drives: the meta as its
/// path keeps it, who its turns belong to, and the literal.
fn typed_rows() -> Vec<(Preset, SessionMeta, &'static str, Seen)> {
    let day = day_session::new_meta(
        "sess-day-2026-11-17",
        chrono::NaiveDate::from_ymd_opt(2026, 11, 17).unwrap(),
        "/w",
        ("ling", None),
    );
    let mut one_off = meta("sess-3-o", "skill", Some("dice"));
    one_off.withheld_tools = vec!["mcp__memory".into()];
    let mut companion = meta("sess-yinyue-2026-11-17", "agent", None);
    companion.agents = vec![SessionMember::new("yinyue")];
    let person = expected(true, true, false, None, true, true);
    let app = expected(false, true, false, None, true, true);
    vec![
        (Preset::Chat, meta("sess-1-c", "user", None), "user", person),
        (
            Preset::App,
            meta("sess-2-a", "skill", Some("dice")),
            "skill",
            app,
        ),
        (
            Preset::Daily,
            day.clone(),
            "user",
            expected(true, true, false, None, true, true),
        ),
        (
            Preset::Daily,
            day,
            "skill",
            expected(false, true, false, None, true, true),
        ),
        (
            Preset::Companion,
            companion,
            "user",
            expected(true, true, false, None, true, true),
        ),
        (
            Preset::Handoff,
            meta("sess-handoff-x", "agent", Some("dice")),
            "skill",
            expected(false, true, false, None, true, true),
        ),
        (
            Preset::OneOff,
            one_off,
            "skill",
            expected(false, true, false, None, true, true),
        ),
    ]
}

#[test]
fn a_persons_turn_runs_as_today() {
    for (preset, meta, owner, want) in typed_rows() {
        assert_eq!(preset_of(&meta), preset, "{}", meta.id);
        let spec = Seen::of(&typed_by_spec(&meta, owner), Some(true));
        assert_eq!(spec, want, "{preset:?} by its spec");
    }
}

/// A mission's run (`missions::scheduler`, `missions::enter`): its tools,
/// no memory, framed as a task, never prompting.
#[test]
fn a_missions_run_runs_as_today() {
    let mission = mission();
    let meta = crate::extensions::missions::scheduler::mission_session_meta(&mission, "sess-9-m");
    assert_eq!(preset_of(&meta), Preset::Mission);
    let scope = crate::extensions::missions::enter::tool_scope(&mission, None);
    let mut by_spec = engine_for(&meta);
    let mut spec = SessionSpec::of(&meta);
    spec.tools = Tools::Mission(scope);
    spec.apply(&mut by_spec);
    let want = expected(false, false, true, set(&["Bash", "Read"]), false, false);
    assert_eq!(Seen::of(&by_spec, Some(false)), want);
    assert!(!by_spec.session_permissions.interactive);
}

/// An `agent_run` delegate (`server::mcp_agent`): its toolset, the owner's
/// sections framed as a task, its caller answering beyond the grants.
#[test]
fn a_delegates_run_runs_as_today() {
    let mut meta = meta("sess-4-d", "external", None);
    meta.params
        .insert(delegate::TOOLS_PARAM.into(), "read".into());
    meta.model_fallback = Some(false);
    assert_eq!(preset_of(&meta), Preset::Delegate);
    let mut by_spec = engine_for(&meta);
    let spec = SessionSpec::of(&meta);
    spec.apply(&mut by_spec);
    let want = expected(true, true, true, delegate_read(), false, false);
    assert_eq!(Seen::of(&by_spec, Some(false)), want);
    assert_eq!(spec.permission, Permission::AskCaller);
    assert_eq!(spec.model_fallback, Some(false));
    assert!(!spec.may_ask_person());
}

/// A `Task` child (`tools::delegation`): the parent's policy, a task,
/// prompting as its parent could; no world.
#[test]
fn a_task_child_runs_as_today() {
    for parent_interactive in [true, false] {
        let mut by_spec = engine();
        SessionSpec::delegated_child(Some(SessionPolicy::owner()), parent_interactive)
            .apply(&mut by_spec);
        let want = expected(true, true, true, None, parent_interactive, false);
        assert_eq!(Seen::of(&by_spec, None), want);
        assert_eq!(
            by_spec.tools.builtins.session_policy,
            Some(SessionPolicy::owner())
        );
    }
}

/// A fresh engine is what an unclaimed spec writes.
#[test]
fn a_fresh_engine_is_unclaimed() {
    let fresh = engine();
    let mut claimed = engine();
    SessionSpec::unclaimed(None).apply(&mut claimed);
    assert_eq!(Seen::of(&fresh, None), Seen::of(&claimed, None));
    assert_eq!(fresh.prompt_profile, claimed.prompt_profile);
}

/// What each kind keeps beside its engine: who leads, its thread, its runtime.
#[test]
fn each_kind_keeps_its_seat_thread_and_runtime() {
    let day = day_session::new_meta(
        "sess-day-2026-11-17",
        chrono::NaiveDate::from_ymd_opt(2026, 11, 17).unwrap(),
        "/w",
        ("ling", None),
    );
    let daily = SessionSpec::of(&day);
    assert_eq!(
        (daily.preset, daily.seat, daily.context),
        (Preset::Daily, Seat::Ling, Context::Daily)
    );
    assert_eq!(day.agents[0].id, "ling");

    let thread = SessionSpec::of(&meta("sess-yinyue-2026-11-17-2", "agent", None));
    assert_eq!(thread.preset, Preset::Companion);
    assert_eq!(
        thread.context,
        Context::Rolling {
            fraction: 0.7,
            live: 10
        }
    );

    let claude = crate::engine::claude_sdk::new_meta("t".into(), "/w".into(), None);
    let spec = SessionSpec::of(&claude);
    assert_eq!(
        (spec.preset, spec.runtime),
        (Preset::Claude, SessionRuntime::ClaudeAgentSdk)
    );

    let mut chat = meta("sess-1-c", "user", None);
    chat.compact_threshold = Some(0.8);
    let spec = SessionSpec::of(&chat);
    assert_eq!(
        spec.context,
        Context::Own {
            threshold: Some(0.8)
        }
    );
    assert_eq!((spec.work, spec.ceiling), (None, None));

    let mission =
        crate::extensions::missions::scheduler::mission_session_meta(&mission(), "sess-9-m");
    let spec = SessionSpec::of(&mission);
    assert_eq!(
        (spec.seat, spec.folder),
        (Seat::MissionAgent, Folder::Mission)
    );
    // Promoted to a person's: still the mission's session.
    let mut promoted = mission;
    promoted.creator = "user".into();
    assert_eq!(preset_of(&promoted), Preset::Mission);
}

/// A person typing into a delegate's session: its toolset and its caller
/// stay; the words reach the model verbatim.
#[test]
fn a_typed_turn_in_a_delegates_session_keeps_its_spec() {
    let mut meta = meta("sess-4-d", "external", None);
    meta.params
        .insert(delegate::TOOLS_PARAM.into(), "write".into());
    let spec = SessionSpec::of(&meta).typed(SessionPolicy::owner(), Memory::of_turn("user"));
    assert_eq!(spec.tools, Tools::Delegate { write: true });
    assert_eq!(spec.permission, Permission::AskCaller);
    let mut e = engine_for(&meta);
    spec.apply(&mut e);
    let want = expected(
        true,
        true,
        false,
        delegate::tool_scope(true).map(|s| s.into_iter().collect()),
        false,
        false,
    );
    assert_eq!(Seen::of(&e, Some(false)), want);
}
