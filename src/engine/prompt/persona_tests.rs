//! Prompt assembly per surface: one soul everywhere, a place per surface.

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

/// Seat the engine's agent as a member at a table `lead` leads.
fn seat_at(engine: &mut AgentEngine, lead: &str) {
    engine.session_lead = Some(lead.to_string());
}

/// Where `needle` sits in `hay`; fails the test when it isn't there.
fn at(hay: &str, needle: &str) -> usize {
    hay.find(needle)
        .unwrap_or_else(|| panic!("missing {needle:?} in:\n{hay}"))
}

const MAC_CHAT: &str = "Linggen's main chat";
const DESKTOP: &str = "a body on the desktop";
const MEMBER: &str = "At a table someone else leads";

#[test]
fn ling_at_home_gets_soul_then_the_main_chat() {
    let p = engine_as("ling", LING).system_prompt();
    let identity = at(&p, "## Identity");
    let body = at(&p, "## How you work");
    let place = at(&p, "## Where you are");
    assert!(identity < body && body < place);
    at(&p, MAC_CHAT);
    // No platform voice layer: a plain session answers in the model's own
    // style; an agent, skill or app opts into a voice by including one.
    assert!(!p.contains("## Voice"), "voice layer leaked back in:\n{p}");
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

/// A session bound to a skill with no app is that skill's, as an app's is:
/// its SKILL.md is the place — no main-chat block, no list of other skills.
/// The same skill taken up mid-chat leaves the agent where it was.
#[test]
fn a_skill_bound_session_has_its_skill_for_a_place_even_without_an_app() {
    let text = "---\nname: zz\ndescription: A plain skill.\n---\nTHE SKILL'S OWN RULES.";
    let plain = crate::extensions::skills::parse_skill_text(text, SkillSource::Global).unwrap();
    let mut engine = engine_as("ling", LING);
    engine.available_skills_metadata = vec![("other".into(), "OTHER SKILL.".into(), true)];
    engine.active_skill = Some(plain);
    engine.skill_bound = true;
    let p = engine.system_prompt();
    at(&p, "THE SKILL'S OWN RULES.");
    assert!(!p.contains(MAC_CHAT) && !p.contains("OTHER SKILL."));

    engine.skill_bound = false;
    let p = engine.system_prompt();
    at(&p, MAC_CHAT);
}

#[test]
fn yinyue_at_home_is_on_the_desktop_and_a_member_at_anothers_table() {
    let home = engine_as("yinyue", YINYUE).system_prompt();
    at(&home, DESKTOP);
    at(&home, "## How you talk");
    assert!(!home.contains(MEMBER));

    let mut member = engine_as("yinyue", YINYUE);
    seat_at(&mut member, "ling");
    let p = member.system_prompt();
    at(&p, MEMBER);
    at(&p, "## How you talk");
    assert!(!p.contains(DESKTOP) && !p.contains("Express"));

    // Her own thread, where she leads: home again.
    seat_at(&mut member, "yinyue");
    at(&member.system_prompt(), DESKTOP);
}

/// The member block holds at any table — an app's chat, the main chat, a
/// mission's session — so it never says she is in an app, or that its
/// agent runs one.
#[test]
fn the_member_block_is_true_at_any_table() {
    let block =
        crate::engine::prompt::place::engine_place("yinyue", place::Surface::Member).unwrap();
    for app_only in ["apps' chats", "runs the app", "You don't run the app"] {
        assert!(
            !block.contains(app_only),
            "{app_only:?} is not true at every table"
        );
    }
}

/// At a skill's table she leads nothing: the skill's place for her, never
/// its SKILL.md (the lead's rules), no list of skills to take up.
#[test]
fn a_member_reads_the_skills_place_for_her_never_its_rules() {
    let table = skill(
        "place:\n  ling: HIS WORLD.\n  yinyue:\n    text: AT THE PLAYER'S SIDE.\n    absent_until: {file: s.json, path: a}\n",
    );
    let mut engine = engine_as("yinyue", YINYUE);
    seat_at(&mut engine, "ling");
    engine.available_skills_metadata = vec![("other".into(), "OTHER SKILL.".into(), true)];
    engine.active_skill = Some(table.clone());
    engine.skill_bound = true;
    let p = engine.system_prompt();
    at(&p, "AT THE PLAYER'S SIDE.");
    assert!(!p.contains("HIS WORLD.") && !p.contains(MEMBER));
    assert!(!p.contains("THE APP'S OWN RULES."), "the lead's rules");
    assert!(!p.contains("OTHER SKILL."));

    // The same skill, Ling leading: its SKILL.md is his place.
    let mut ling = engine_as("ling", LING);
    seat_at(&mut ling, "ling");
    ling.active_skill = Some(table);
    ling.skill_bound = true;
    at(&ling.system_prompt(), "THE APP'S OWN RULES.");
}

/// Her line landing in an app chat from her own thread: she speaks from
/// that table — the table's declaration for her, else the member block.
#[test]
fn a_line_landing_at_a_table_speaks_from_it() {
    let table = skill("place:\n  yinyue: AT THE PLAYER'S SIDE.\n");
    let mut engine = engine_as("yinyue", YINYUE);
    engine.speaks_at_table = true;
    let p = engine.system_prompt();
    at(&p, MEMBER);
    assert!(!p.contains(DESKTOP));
    engine.seat_places = table.place.clone();
    let p = engine.system_prompt();
    at(&p, "AT THE PLAYER'S SIDE.");
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
