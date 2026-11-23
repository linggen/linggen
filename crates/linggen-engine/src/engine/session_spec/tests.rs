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
        [("test-openai", "openai"), ("test-claude", "anthropic"), ("test-deepseek", "deepseek")].into_iter().map(|(id, provider)| crate::config::ModelConfig {
            id: id.into(), provider: provider.into(), url: "http://localhost:1".into(), model: id.into(),
            api_key: None, keep_alive: None, context_window: None, tags: vec![], supports_tools: Some(true),
            auth_mode: None, reasoning_effort: None, max_output_tokens: None, provided_by: None, is_builtin: false, family_binding: None,
            price: None,
        }).collect(),
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

fn code_meta(id: &str, creator: &str, skill: Option<&str>) -> SessionMeta {
    let plain = meta(id, creator, skill);
    let yaml = format!("{}\nwork: code\n", serde_norway::to_string(&plain).unwrap());
    serde_norway::from_str(&yaml).unwrap()
}

#[test]
fn project_instructions_reach_native_owner_delegate_mission_and_skill_but_not_consumer() {
    let chat = SessionSpec::of(&code_meta("sess-chat", "user", None));
    let delegate = SessionSpec::of(&meta("sess-delegate", "external", None));
    let skill = SessionSpec::of(&meta("sess-app", "user", Some("dice")));
    let mission = SessionSpec::of(
        &crate::extensions::missions::scheduler::mission_session_meta(&mission(), "sess-9-m"),
    );
    for (spec, preset) in [(&chat, Preset::Chat), (&delegate, Preset::Delegate),
        (&mission, Preset::Mission), (&skill, Preset::App)]
    {
        assert_eq!(spec.preset, preset);
        assert!(spec.profile().include_project_context, "{preset:?}");
    }
    let consumer = chat.typed(SessionPolicy::consumer(Default::default()), Memory::None, None);
    assert!(!consumer.profile().include_project_context);
}

#[test]
fn a_delegates_typed_followup_does_not_change_the_sessions_origin() {
    let mut e = engine();
    let m = meta("sess-delegate", "external", None);
    let spec = SessionSpec::of(&m);
    spec.apply(&mut e);
    assert!(e.tools.builtins.delegated_session);
    assert!(!e.tools.builtins.person_spoke);

    spec.typed(SessionPolicy::owner(), Memory::Full, None).apply(&mut e);
    e.begin_turn(crate::engine::turn_place::TurnSettings {
        person_spoke: true,
        ..Default::default()
    });
    assert!(e.tools.builtins.delegated_session);
    assert!(e.tools.builtins.person_spoke);
    e.end_turn();
    assert!(!e.tools.builtins.person_spoke);
}

#[test]
fn work_code_chat_prompt() {
    let tagged = typed_by_spec(&code_meta("sess-c", "user", None), "user");
    let plain = typed_by_spec(&meta("sess-c", "user", None), "user");
    assert!(!plain.system_prompt().contains("## Working on code"));
    assert!(tagged.system_prompt().contains("## Working on code"));
}

#[test]
fn work_code_model_family_prompt() {
    let mut e = typed_by_spec(&code_meta("sess-c", "user", None), "user");
    e.model_id = "test-openai".into();
    assert!(e.system_prompt().contains("not apply_patch"));
    e.model_id = "test-claude".into();
    let prompt = e.system_prompt();
    assert!(!prompt.contains("not apply_patch"));
    assert!(!prompt.contains("Use absolute paths in every tool call"));
    assert!(!prompt.contains("Do not describe an edit you have not made"));
}

#[test]
fn work_code_switch_model_prompt() {
    use crate::engine::prompt::PromptPurpose;
    let mut e = typed_by_spec(&code_meta("sess-c", "user", None), "user");
    e.model_id = "test-openai".into();
    let first = e.prepare_loop_messages("hi", true, PromptPurpose::Preview).0[0].content.clone();
    assert!(first.contains("not apply_patch"));
    e.model_id = "test-deepseek".into();
    let next = e.prepare_loop_messages("hi", true, PromptPurpose::Preview).0[0].content.clone();
    assert!(next.contains("Do not describe an edit you have not made"));
    assert!(!next.contains("not apply_patch"));
}

/// A call that falls back mid-way to a model of another family carries that
/// family's lines, not the first model's; the same family changes nothing.
#[test]
fn work_code_a_fallback_to_another_family_gets_its_lines() {
    use crate::engine::prompt::PromptPurpose;
    let mut e = typed_by_spec(&code_meta("sess-c", "user", None), "user");
    e.model_id = "test-openai".into();
    let built = e.prepare_loop_messages("hi", true, PromptPurpose::Preview).0;
    assert!(e.messages_for_model("test-openai", &built).is_none());

    let deepseek = e.messages_for_model("test-deepseek", &built).unwrap();
    assert!(deepseek[0].content.contains("Do not describe an edit you have not made"));
    assert!(!deepseek[0].content.contains("not apply_patch"));
    assert!(deepseek[0].content.contains("## Working on code"));
    assert_eq!(deepseek.len(), built.len());

    let claude = e.messages_for_model("test-claude", &built).unwrap();
    assert!(claude[0].content.contains("## Working on code"));
    assert!(!claude[0].content.contains("not apply_patch"));

    // A session with no work tag is never touched.
    let mut plain = typed_by_spec(&meta("sess-c", "user", None), "user");
    plain.model_id = "test-openai".into();
    let built = plain.prepare_loop_messages("hi", true, PromptPurpose::Preview).0;
    assert!(plain.messages_for_model("test-deepseek", &built).is_none());
}

#[test]
fn work_code_daily_and_app_ignore_tag() {
    for m in [code_meta("sess-day-2026-11-17", "agent", None), code_meta("sess-app", "skill", Some("dice"))] {
        let e = typed_by_spec(&m, "user");
        assert!(!e.system_prompt().contains("## Working on code"));
    }
    // The tag must still be recognized on the eligible control.
    assert!(typed_by_spec(&code_meta("sess-c", "user", None), "user").system_prompt().contains("## Working on code"));
}

#[test]
fn work_code_task_child_inherits() {
    let parent = typed_by_spec(&code_meta("sess-c", "user", None), "user");
    let mut child = engine();
    let spec = SessionSpec::delegated_child(parent.tools.builtins.session_policy.clone(), true, parent.tools.builtins.parent_work, None);
    spec.apply(&mut child);
    assert!(child.system_prompt().contains("## Working on code"));
}

/// A coding session runs any program in its folder unasked; the gate reads
/// it from the spec (`AgentEngine::is_coding_session`).
#[test]
fn work_code_makes_a_coding_session_only_where_the_preset_takes_it() {
    assert!(typed_by_spec(&code_meta("sess-c", "user", None), "user").is_coding_session());
    assert!(!typed_by_spec(&meta("sess-c", "user", None), "user").is_coding_session());
    // An app's session, the daily session and a mission never are.
    let mut mission_meta =
        crate::extensions::missions::scheduler::mission_session_meta(&mission(), "sess-9-m");
    mission_meta.work = Some(Work::Code);
    for m in [
        code_meta("sess-day-2026-11-17", "agent", None),
        code_meta("sess-app", "skill", Some("dice")),
        mission_meta,
    ] {
        let mut e = engine_for(&m);
        SessionSpec::of(&m).apply(&mut e);
        assert!(!e.is_coding_session(), "{}", m.id);
        let typed = typed_by_spec(&m, "user");
        assert!(!typed.is_coding_session(), "{}", m.id);
    }
}

#[test]
fn work_code_a_task_child_of_a_coding_session_is_one() {
    for (work, coding) in [(Some(Work::Code), true), (None, false)] {
        let mut child = engine();
        SessionSpec::delegated_child(Some(SessionPolicy::owner()), true, work, None).apply(&mut child);
        assert_eq!(child.is_coding_session(), coding, "{work:?}");
    }
}

/// Turning Coding off is seen at the next check, not only the next turn.
#[test]
fn work_code_turned_off_is_seen_at_the_next_check() {
    let dir = tempfile::tempdir().unwrap();
    let store = crate::state_fs::sessions::SessionStore::with_sessions_dir(dir.path().into());
    let m = code_meta("sess-c", "user", None);
    store.add_session(&m).unwrap();
    let mut e = typed_by_spec(&m, "user");
    e.session_dir = Some(store.session_dir("sess-c"));
    assert!(e.is_coding_session());
    store.edit("sess-c", |meta| ask_work(meta, "none")).unwrap().unwrap();
    assert!(!e.is_coding_session());
    // Back on: this turn's spec still says code. A session whose spec was
    // made without the tag is not one until its next turn's spec is.
    store.edit("sess-c", |meta| ask_work(meta, "code")).unwrap().unwrap();
    assert!(e.is_coding_session());
    let mut plain = typed_by_spec(&meta("sess-c", "user", None), "user");
    plain.session_dir = Some(store.session_dir("sess-c"));
    assert!(!plain.is_coding_session());
}

#[test]
fn work_code_preset_table_and_unknown_value() {
    for (preset, accepts) in [
        (Preset::Chat, true), (Preset::Delegate, true), (Preset::Handoff, true),
        (Preset::App, false), (Preset::OneOff, false), (Preset::Daily, false),
        (Preset::Companion, false), (Preset::Mission, false), (Preset::OldClaude, false),
    ] { assert_eq!(preset.takes_work(), accepts, "{preset:?}"); }
    let value: SessionMeta = serde_norway::from_str("id: x\ntitle: x\ncreated_at: 0\nwork: future\n").unwrap();
    assert_eq!(value.work, None);
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
        .typed(SessionPolicy::owner(), Memory::of_turn(turn_owner), None)
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

/// An attended mission run asks through AskUser: it joins the run's tools
/// in its spec, so a later change of the spec keeps it.
#[test]
fn an_attended_missions_askuser_is_in_its_spec() {
    let mission = mission();
    let meta = crate::extensions::missions::scheduler::mission_session_meta(&mission, "sess-9-m");
    let mut engine = engine_for(&meta);
    let mut spec = SessionSpec::of(&meta);
    spec.tools = Tools::Mission(crate::extensions::missions::enter::tool_scope(&mission, None));
    spec.apply(&mut engine);
    engine.respec(|spec| spec.tools.permit_in_mission("AskUser"));
    engine.respec(|spec| spec.memory = Memory::None);
    let tools = engine.cfg.mission_allowed_tools.clone().unwrap();
    assert!(tools.contains("AskUser") && tools.contains("Read"));
    assert!(!engine.session_permissions.interactive);
}

/// A mission taken up again with the same spec still drops the prompt
/// cached before: the cache predates the mission's body.
#[test]
fn taking_up_a_mission_drops_the_cached_prompt() {
    let mut engine = engine();
    engine.cached_system_prompt = Some(crate::engine::types::CachedSystemPrompt {
        input_hash: 1,
        instruction_watch: Default::default(),
        content: "before".into(),
    });
    let frame = crate::engine::ActiveMission {
        body: "Body".into(),
        mission_dir: None,
    };
    engine.take_up_mission(frame, |_| {});
    assert!(engine.runs_mission());
    assert!(engine.cached_system_prompt.is_none());
}

/// A session made by someone else now (a mission's promoted to a person's)
/// builds its prompt again: who made it decides what the prompt carries.
#[test]
fn a_new_creator_drops_the_cached_prompt() {
    let mut engine = engine_for(&meta("sess-1-m", "mission", None));
    engine.cached_system_prompt = Some(crate::engine::types::CachedSystemPrompt {
        input_hash: 1,
        instruction_watch: Default::default(),
        content: "before".into(),
    });
    engine.respec(|spec| spec.creator = Some("mission".into()));
    assert!(engine.cached_system_prompt.is_some(), "the same creator keeps it");
    engine.respec(|spec| spec.creator = Some("user".into()));
    assert!(engine.cached_system_prompt.is_none());
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
        SessionSpec::delegated_child(Some(SessionPolicy::owner()), parent_interactive, None, None)
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

    let claude = SessionMeta { id: "sess-1-claude".into(), runtime: crate::state_fs::sessions::SessionRuntime::ClaudeAgentSdk, ..Default::default() };
    let spec = SessionSpec::of(&claude);
    assert_eq!(spec.preset, Preset::OldClaude);

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
    let spec = SessionSpec::of(&meta).typed(SessionPolicy::owner(), Memory::of_turn("user"), None);
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

/// A turn a device sent is typed at its level: it asks no one and carries
/// none of the owner's memory; at admin it has no ceiling. The owner's
/// next turn on the same engine has none either, and keeps the session's
/// grants. A `Task` child takes the turn's ceiling.
#[test]
fn a_devices_turn_runs_under_its_level() {
    use crate::engine::permission::PermissionMode::*;
    let m = meta("sess-5-c", "user", None);
    let typed = |level| SessionSpec::of(&m).typed(SessionPolicy::owner(), Memory::of_turn("user"), level);
    let read = typed(Some(Read));
    assert_eq!((read.ceiling, read.permission, read.memory), (Some(Read), Permission::Unattended, Memory::None));
    let edit = typed(Some(Edit));
    assert_eq!((edit.ceiling, edit.permission, edit.memory), (Some(Edit), Permission::Unattended, Memory::None));
    let owners = typed(Some(Admin));
    assert_eq!((owners.ceiling, owners.permission, owners.memory), (None, Permission::Interactive, Memory::Full));
    assert_eq!(typed(None).ceiling, None);

    let mut e = engine_for(&m);
    e.session_permissions.set_path_mode("*", Edit);
    read.apply(&mut e);
    assert_eq!(e.session_spec().ceiling, Some(Read));
    let mut child = engine();
    SessionSpec::delegated_child(e.tools.builtins.session_policy.clone(), true, None, e.tools.builtins.parent_ceiling)
        .apply(&mut child);
    assert_eq!(child.session_spec().ceiling, Some(Read), "a Task child keeps the turn's ceiling");
    typed(None).apply(&mut e);
    assert_eq!(e.session_spec().ceiling, None);
    assert!(e.session_permissions.interactive);
    let grant = crate::engine::permission::effective_mode_for_path(&e.session_permissions.path_modes, std::path::Path::new("/x"));
    assert_eq!(grant, Some(Edit), "the session's grants are the owner's still");
}
