//! Two turns of one session in two places, at the engine level: nothing of
//! the first place reaches the second, and the first is whole again after.

use crate::engine::skill::{Skill, SkillSource};
use crate::engine::skill_activation::ActivationMode;
use crate::engine::skill::QueueMode;
use crate::engine::turn_place::running::{meets_running, Meets};
use crate::engine::turn_place::TurnPlace;
use crate::engine::{AgentEngine, AgentRole, EngineConfig, InterfaceMode};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const LING: &str = include_str!("../../../../../../agents/ling.md");
const SID: &str = "sess-1-two-places";

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
    let mut engine = AgentEngine::new(cfg, models, "m".to_string(), AgentRole::Lead).unwrap();
    let (spec, body) = crate::extensions::agents::parse_agent_markdown(LING).unwrap();
    engine.set_spec("ling".to_string(), spec, body);
    engine.tools.builtins.set_session_id(Some(SID.to_string()));
    crate::engine::session_policy::SessionPolicy::owner().apply(&mut engine);
    engine
}

/// A made-up app: its own rules, tools, folder and memory.
fn x_app(folder: &Path) -> Skill {
    let text = format!(
        "---\nname: zz-x\ndescription: The x app.\nallowed-tools: [Read]\n\
         memory-context: What the x app remembers.\ncwd: {}\nqueue: after-turn\n\
         tools:\n  - name: XLook\n    description: Look at x.\n    cmd: \"echo x\"\n\
         ---\nTHE X APP'S OWN RULES.",
        folder.display()
    );
    in_folder(crate::extensions::skills::parse_skill_text(&text, SkillSource::Global).unwrap(), folder)
}

/// Another made-up app: a different tool set, no memory, its own folder.
fn y_app(folder: &Path) -> Skill {
    let text = format!(
        "---\nname: zz-y\ndescription: The y app.\nallowed-tools: [Grep]\ncwd: {}\n\
         ---\nTHE Y APP'S OWN RULES.",
        folder.display()
    );
    in_folder(crate::extensions::skills::parse_skill_text(&text, SkillSource::Global).unwrap(), folder)
}

fn in_folder(mut skill: Skill, folder: &Path) -> Skill {
    skill.skill_dir = Some(folder.to_path_buf());
    skill
}

fn place(skill: &str, own: bool) -> TurnPlace {
    let mut params = BTreeMap::new();
    if own {
        params.insert("save".to_string(), "main".to_string());
    }
    TurnPlace {
        skill: Some(skill.to_string()),
        params,
        own,
    }
}

fn offered(engine: &AgentEngine) -> Vec<String> {
    engine
        .offered_tool_definitions(engine.allowed_tool_names().as_ref())
        .iter()
        .filter_map(|d| d.pointer("/function/name")?.as_str().map(String::from))
        .collect()
}

/// What a turn in a place could see of it.
#[derive(Debug, PartialEq)]
struct Seen {
    prompt_has_x: bool,
    prompt_has_y: bool,
    offers_xlook: bool,
    offers_read: bool,
    offers_grep: bool,
    cwd: PathBuf,
    memory_app_dir: Option<PathBuf>,
    include_memory: bool,
    params: BTreeMap<String, String>,
    tool_skill: Option<String>,
}

fn seen(engine: &AgentEngine) -> Seen {
    let prompt = engine.system_prompt();
    let tools = offered(engine);
    Seen {
        prompt_has_x: prompt.contains("THE X APP'S OWN RULES."),
        prompt_has_y: prompt.contains("THE Y APP'S OWN RULES."),
        offers_xlook: tools.iter().any(|t| t == "XLook"),
        offers_read: tools.iter().any(|t| t == "Read"),
        offers_grep: tools.iter().any(|t| t == "Grep"),
        cwd: engine.tools.builtins.cwd(),
        memory_app_dir: engine.prompt_profile.memory_app_dir.clone(),
        include_memory: engine.prompt_profile.include_memory,
        params: engine.tools.builtins.session_params_for(Some("zz-x")),
        tool_skill: engine.tools.builtins.active_skill().map(|(n, _)| n.clone()),
    }
}

#[tokio::test]
async fn a_second_place_sees_nothing_of_the_first() {
    let dir = tempfile::tempdir().unwrap();
    let (x_dir, y_dir) = (dir.path().join("x"), dir.path().join("y"));
    let mut engine = engine();
    let at_home = seen(&engine);
    let ws_root_before = engine.cfg.ws_root.clone();

    // Turn 1: the session's own app.
    engine
        .enter_place(place("zz-x", true), Some(x_app(&x_dir)), ActivationMode::SessionBound)
        .await;
    let in_x = seen(&engine);
    assert!(in_x.prompt_has_x && in_x.offers_xlook && in_x.offers_read && !in_x.offers_grep, "{in_x:?} {:?}", offered(&engine));
    assert_eq!(in_x.cwd, x_dir);
    assert!(in_x.memory_app_dir.is_some() && !in_x.include_memory);
    assert_eq!(in_x.params.get("save").map(String::as_str), Some("main"));
    assert_eq!(in_x.tool_skill.as_deref(), Some("zz-x"));
    engine.leave_place();

    // Turn 2: another app's page, in the same session, on the same engine.
    engine
        .enter_place(place("zz-y", false), Some(y_app(&y_dir)), ActivationMode::SessionBound)
        .await;
    let in_y = seen(&engine);
    assert!(!in_y.prompt_has_x, "x's rules in y's prompt");
    assert!(in_y.prompt_has_y);
    assert!(!in_y.offers_xlook && !in_y.offers_read, "x's tools offered in y");
    assert!(in_y.offers_grep);
    assert_eq!(in_y.cwd, y_dir, "y works in its own folder");
    // The prompt's workspace (environment, project instructions) is y's.
    assert!(!engine.cfg.ws_root.starts_with(&x_dir), "x's folder is y's workspace");
    assert!(engine.cfg.ws_root.ends_with("y"), "{:?}", engine.cfg.ws_root);
    assert!(!engine.system_prompt().contains(&x_dir.display().to_string()));
    assert_eq!(in_y.memory_app_dir, None, "x's memory recalled in y");
    assert!(in_y.params.is_empty(), "x's save read in y");
    assert_eq!(in_y.tool_skill.as_deref(), Some("zz-y"));
    assert_eq!(engine.turn_place().and_then(|p| p.skill.as_deref()), Some("zz-y"));
    // Queue: a message in y meets y's turn by y's rule, never x's after-turn;
    // one from x's page waits for y's turn to end.
    let y_rule = y_app(&y_dir).queue;
    assert_eq!(
        meets_running(engine.turn_place(), &place("zz-y", false), y_rule),
        Meets::Rule(QueueMode::Steer)
    );
    assert_eq!(
        meets_running(engine.turn_place(), &place("zz-x", true), x_app(&x_dir).queue),
        Meets::AfterRun
    );
    engine.leave_place();

    // Back in the session's own app: as it was, and the folder its own.
    engine
        .enter_place(place("zz-x", true), Some(x_app(&x_dir)), ActivationMode::SessionBound)
        .await;
    assert_eq!(seen(&engine), in_x);
    engine.leave_place();

    // Between turns the engine stands nowhere: as it was before any place,
    // its workspace root its own again.
    assert_eq!(engine.cfg.ws_root, ws_root_before);
    let after = seen(&engine);
    assert_eq!(Seen { cwd: at_home.cwd.clone(), ..after }, at_home);
    assert!(engine.turn_place().is_none());
}

/// Home in a session made for an app works in the default workspace, and
/// takes nothing of the app.
#[tokio::test]
async fn home_in_an_apps_session_takes_nothing_of_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine();
    engine
        .enter_place(place("zz-x", true), Some(x_app(&dir.path().join("x"))), ActivationMode::SessionBound)
        .await;
    engine.leave_place();
    let home = TurnPlace { own: false, ..TurnPlace::home() };
    engine.enter_place(home, None, ActivationMode::SessionBound).await;
    let at_home = seen(&engine);
    assert!(!at_home.prompt_has_x && !at_home.offers_xlook);
    assert!(at_home.include_memory && at_home.memory_app_dir.is_none());
    assert_eq!(at_home.tool_skill, None);
    let workspace = crate::util::resolve_path(&crate::paths::default_workspace());
    assert_eq!(at_home.cwd, workspace);
}

/// Entering a place leaves the one before first: a turn that ended without
/// leaving (a stopped turn) hands nothing on.
#[tokio::test]
async fn entering_leaves_the_place_before() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine();
    engine
        .enter_place(place("zz-x", true), Some(x_app(&dir.path().join("x"))), ActivationMode::SessionBound)
        .await;
    engine
        .enter_place(place("zz-y", false), Some(y_app(&dir.path().join("y"))), ActivationMode::SessionBound)
        .await;
    let in_y = seen(&engine);
    assert!(!in_y.prompt_has_x && !in_y.offers_xlook && in_y.memory_app_dir.is_none());
}

/// A skill taken up mid-turn by the Skill tool keeps its tools for later
/// turns, as before places: only what the place registered is taken away.
#[tokio::test]
async fn a_skill_borrowed_mid_turn_keeps_its_tools() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = engine();
    engine.enter_place(TurnPlace::home(), None, ActivationMode::SessionBound).await;
    engine.activate_skill(x_app(&dir.path().join("x")), ActivationMode::ToolInvocation).await;
    engine.leave_place();
    assert!(engine.tools.skill_tools.contains_key("XLook"));
}

/// In a day's session a private app (`private-talk: true`) never becomes a
/// turn's place nor its active skill, whoever takes it up — a page, a
/// slash command, a trigger, the Skill tool. A shared app still does, and
/// outside a day a private one is taken up as before.
#[tokio::test]
async fn a_private_app_is_never_taken_up_in_a_day() {
    let dir = tempfile::tempdir().unwrap();
    let private = || {
        let mut s = x_app(&dir.path().join("x"));
        s.private_talk = true;
        s
    };
    let mut engine = engine();
    engine.tools.builtins.set_session_id(Some("sess-day-2026-11-16".to_string()));
    engine
        .enter_place(place("zz-x", false), Some(private()), ActivationMode::SessionBound)
        .await;
    let there = seen(&engine);
    assert!(!there.prompt_has_x && !there.offers_xlook, "{there:?}");
    for mode in [
        ActivationMode::SlashCommand,
        ActivationMode::Trigger,
        ActivationMode::ToolInvocation,
    ] {
        engine.activate_skill(private(), mode).await;
        assert!(engine.active_skill.is_none(), "{mode:?} took it up");
    }
    engine.activate_skill(y_app(&dir.path().join("y")), ActivationMode::ToolInvocation).await;
    assert!(engine.active_skill.is_some(), "a shared app is taken up");

    let mut apart = engine_in(SID);
    apart.activate_skill(private(), ActivationMode::SlashCommand).await;
    assert!(apart.active_skill.is_some(), "outside a day it is taken up as before");
}

fn engine_in(sid: &str) -> AgentEngine {
    let mut engine = engine();
    engine.tools.builtins.set_session_id(Some(sid.to_string()));
    engine
}
