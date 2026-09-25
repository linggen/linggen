//! Prompt assembly per surface: one soul everywhere, a place per surface.

use crate::engine::permission::{PermissionMode, SessionPermissions};
use crate::engine::prompt::place;
use crate::engine::skill::{Skill, SkillSource};
use crate::engine::{AgentEngine, AgentRole, EngineConfig, InterfaceMode};

const LING: &str = include_str!("../../../agents/ling.md");
const YINYUE: &str = include_str!("../../../agents/yinyue.md");

fn engine_as(agent: &str, spec_md: &str) -> AgentEngine {
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
    let (spec, body) = crate::extensions::agents::parse_agent_markdown(spec_md).unwrap();
    engine.set_spec(agent.to_string(), spec, body);
    engine
}

fn skill(frontmatter_extra: &str) -> Skill {
    let text = format!(
        "---\nname: zz\ndescription: A test app.\napp:\n  launcher: web\n  entry: index.html\n{frontmatter_extra}---\nTHE APP'S OWN RULES."
    );
    crate::extensions::skills::parse_skill_text(&text, SkillSource::Global).unwrap()
}

fn seat_as_guest(engine: &mut AgentEngine) {
    engine.seat_permissions = Some(SessionPermissions::seat("/tmp", PermissionMode::Read));
}

/// Where `needle` sits in `hay`; fails the test when it isn't there.
fn at(hay: &str, needle: &str) -> usize {
    hay.find(needle)
        .unwrap_or_else(|| panic!("missing {needle:?} in:\n{hay}"))
}

const MAC_CHAT: &str = "Linggen's main chat";
const DESKTOP: &str = "a body on the desktop";
const GUEST: &str = "A guest in someone else's chat";

#[test]
fn ling_at_home_gets_soul_then_voice_then_the_main_chat() {
    let p = engine_as("ling", LING).system_prompt();
    let identity = at(&p, "## Identity");
    let body = at(&p, "## How you work");
    let voice = at(&p, "## Voice");
    let place = at(&p, "## Where you are");
    assert!(identity < body && body < voice && voice < place);
    at(&p, MAC_CHAT);
}

#[test]
fn an_app_session_keeps_the_soul_and_the_skill_is_its_place() {
    let mut engine = engine_as("ling", LING);
    engine.active_skill = Some(skill("place:\n  ling: IGNORED FOR THE APP'S OWN AGENT.\n"));
    let p = engine.system_prompt();
    let body = at(&p, "## How you work");
    let rules = at(&p, "THE APP'S OWN RULES.");
    assert!(body < rules, "soul → skill");
    assert!(!p.contains(MAC_CHAT) && !p.contains("## Where you are"));
    assert!(!p.contains("IGNORED FOR THE APP'S OWN AGENT."));
}

#[test]
fn yinyue_at_home_is_on_the_desktop_and_a_guest_at_an_app_table() {
    let home = engine_as("yinyue", YINYUE).system_prompt();
    at(&home, DESKTOP);
    at(&home, "## How you talk");
    assert!(!home.contains(GUEST));

    let mut guest = engine_as("yinyue", YINYUE);
    seat_as_guest(&mut guest);
    let p = guest.system_prompt();
    at(&p, GUEST);
    at(&p, "## How you talk");
    assert!(!p.contains(DESKTOP) && !p.contains("Express"));
}

/// The guest block holds at any table — an app's chat, the main chat, a
/// mission's session — so it never says she is in an app, or that its
/// agent runs one.
#[test]
fn the_guest_block_is_true_at_any_table() {
    let block =
        crate::engine::prompt::place::engine_place("yinyue", place::Surface::Guest).unwrap();
    for app_only in ["apps' chats", "runs the app", "You don't run the app"] {
        assert!(
            !block.contains(app_only),
            "{app_only:?} is not true at every table"
        );
    }
}

#[test]
fn a_guest_reads_the_tables_skill_for_her_own_place_only() {
    let table = skill(
        "place:\n  ling: HIS WORLD.\n  yinyue:\n    text: AT THE PLAYER'S SIDE.\n    absent_until: {file: s.json, path: a}\n",
    );
    let mut engine = engine_as("yinyue", YINYUE);
    seat_as_guest(&mut engine);
    engine.seat_places = table.place.clone();
    let p = engine.system_prompt();
    at(&p, "AT THE PLAYER'S SIDE.");
    assert!(!p.contains("HIS WORLD.") && !p.contains(GUEST));
    assert!(
        !p.contains("THE APP'S OWN RULES."),
        "a guest takes up no skill"
    );
}

#[test]
fn a_consumer_frame_and_a_delegate_have_no_place() {
    let mut consumer = engine_as("ling", LING);
    consumer.prompt_profile.consumer_frame = true;
    let p = consumer.system_prompt();
    assert!(!p.contains("## Where you are") && !p.contains("## How you work"));

    let mut delegate = engine_as("ling", LING);
    delegate.set_parent_agent(Some("ling".into()));
    let p = delegate.system_prompt();
    assert!(!p.contains("## Where you are"));
    at(&p, "## How you work");
}
