//! A model's call of an app's declared action, judged by the permission
//! gate (`gate_permission`): every class — the words a skill writes as
//! `tier:` — against every grant the session holds on its folder, for a
//! session nobody can ask, for a person who is asked, and for a turn under
//! a ceiling.

use super::declined_gate_tests::{answer_first, engine, Pending};
use super::*;
use crate::engine::permission::{PermissionMode, SessionPermissions};
use crate::engine::skill_tool::SkillToolDef;

/// One tool per way of writing a tier, and what each needs.
const WORDS: [(&str, Option<&str>, PermissionMode); 7] = [
    ("TRead", Some("read"), PermissionMode::Read),
    ("TEdit", Some("edit"), PermissionMode::Edit),
    ("TWrite", Some("write"), PermissionMode::Edit),
    ("TAdmin", Some("admin"), PermissionMode::Admin),
    ("TAbsent", None, PermissionMode::Admin),
    ("TChat", Some("chat"), PermissionMode::Admin),
    ("TNonsense", Some("nonsense"), PermissionMode::Admin),
];

fn declared(name: &str, word: Option<&str>) -> SkillToolDef {
    let mut def = serde_json::json!({ "name": name, "description": "d", "cmd": "echo ok" });
    if let Some(word) = word {
        def["tier"] = word.into();
    }
    serde_json::from_value(def).unwrap()
}

/// An engine whose session holds `grant` on its folder, with every
/// fixture tool registered. `asks`: who answers a request.
fn session(root: &std::path::Path, grant: PermissionMode, interactive: bool, ask_caller: bool, pending: &Pending) -> AgentEngine {
    let mut engine = engine(root, pending, ask_caller);
    engine.session_permissions = SessionPermissions { interactive, ask_caller, ..Default::default() };
    engine.session_permissions.set_path_mode(&root.to_string_lossy(), grant);
    for (name, word, _) in WORDS {
        engine.tools.skill_tools.insert(name.to_string(), declared(name, word));
    }
    engine
}

/// The gate's answer: `None` lets the call run, `Some(text)` is what the
/// model is told when it does not.
async fn judged(engine: &mut AgentEngine, tool: &str) -> Option<String> {
    let args = serde_json::json!({});
    let id = Some("c1".to_string());
    let t = ToolTurn { tool, canonical: tool, args: &args, session_id: None, tool_call_id: &id };
    let mut messages = Vec::new();
    let stop = engine.gate_permission(&t, &mut messages, &mut Vec::new()).await;
    stop.map(|_| messages.last().map(|m| m.content.clone()).unwrap_or_default())
}

fn in_home() -> tempfile::TempDir {
    tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap()
}

/// A session nobody can ask runs what its grant covers and is refused the
/// rest, without a question.
#[tokio::test]
async fn an_unattended_session_runs_what_its_grant_covers() {
    let work = in_home();
    for grant in [PermissionMode::Read, PermissionMode::Edit, PermissionMode::Admin] {
        let pending: Pending = Default::default();
        let mut engine = session(work.path(), grant, false, false, &pending);
        for (name, word, needs) in WORDS {
            let got = judged(&mut engine, name).await;
            let at = format!("{name} ({word:?}) under {grant}");
            if needs <= grant {
                assert!(got.is_none(), "{at}: {got:?}");
            } else {
                let told = got.unwrap_or_else(|| panic!("{at}: let through"));
                assert!(told.contains(name), "{at}: {told}");
            }
        }
        assert!(pending.lock().await.is_empty(), "nobody is asked");
    }
}

/// A person is asked for what the grant does not cover: a grant of the
/// level for the session, or deny — no "allow once" for an app's action.
#[tokio::test]
async fn a_person_is_asked_for_what_the_grant_does_not_cover() {
    let work = in_home();
    let wait = std::time::Duration::from_secs(5);

    let pending: Pending = Default::default();
    let seen = answer_first(&pending, "Deny");
    let mut engine = session(work.path(), PermissionMode::Read, true, false, &pending);
    assert!(judged(&mut engine, "TEdit").await.is_some(), "denied");
    let (labels, request) = tokio::time::timeout(wait, seen).await.expect("asked").unwrap();
    assert!(request.is_none(), "the person, not a caller");
    let folder = work.path().to_string_lossy().to_string();
    assert!(labels.contains(&format!("grant:at:edit:{folder}")), "{labels:?}");
    assert!(labels.contains(&"grant:all:edit".to_string()), "{labels:?}");
    assert_eq!(labels.last().map(String::as_str), Some("Deny"), "{labels:?}");
    assert!(!labels.contains(&crate::engine::permission::ALLOW_ONCE.to_string()), "{labels:?}");

    let pending: Pending = Default::default();
    let seen = answer_first(&pending, "grant:all:edit");
    let mut engine = session(work.path(), PermissionMode::Read, true, false, &pending);
    assert!(judged(&mut engine, "TEdit").await.is_none(), "the grant lets it run");
    tokio::time::timeout(wait, seen).await.expect("asked").unwrap();
    assert!(judged(&mut engine, "TEdit").await.is_none(), "and holds for the session");
}

/// A turn under a ceiling is refused above it, whatever the grants say,
/// and asks no one.
#[tokio::test]
async fn a_turn_under_a_ceiling_does_no_more_than_the_ceiling() {
    let work = in_home();
    for (ceiling, allowed) in [
        (PermissionMode::Read, ["TRead"].as_slice()),
        (PermissionMode::Edit, ["TRead", "TEdit", "TWrite"].as_slice()),
    ] {
        let pending: Pending = Default::default();
        let mut engine = session(work.path(), PermissionMode::Admin, false, false, &pending);
        engine.session_spec.ceiling = Some(ceiling);
        for (name, word, _) in WORDS {
            let got = judged(&mut engine, name).await;
            let at = format!("{name} ({word:?}) under a ceiling of {ceiling}");
            if allowed.contains(&name) {
                assert!(got.is_none(), "{at}: {got:?}");
            } else {
                let told = got.unwrap_or_else(|| panic!("{at}: let through"));
                assert!(told.starts_with(&format!("Tool {name}: Beyond this turn's level")), "{at}: {told}");
            }
        }
    }
}

// ── A destructive action: a model that asks for one always asks the person ──

fn destroy_engine(root: &std::path::Path, grant: PermissionMode, interactive: bool, ask_caller: bool, pending: &Pending) -> AgentEngine {
    let mut engine = session(root, grant, interactive, ask_caller, pending);
    engine.tools.skill_tools.insert("TDestroy".into(), declared("TDestroy", Some("destructive")));
    engine
}

const ONCE_OR_DENY: [&str; 2] = [crate::engine::permission::ALLOW_ONCE, "Deny"];

/// The same stored grants stand for the owner and the household member;
/// only the owner may be asked to confirm the model's destructive action.
#[tokio::test]
async fn stored_interactivity_cannot_make_a_capped_turn_ask() {
    use crate::engine::events::ServerEvent;
    use crate::engine::session_policy::SessionPolicy;
    use crate::engine::session_spec::Memory;

    let work = in_home();
    let sessions = crate::paths::global_sessions_dir();
    std::fs::create_dir_all(&sessions).unwrap();
    let saved = tempfile::tempdir_in(sessions).unwrap();
    let sid = saved.path().file_name().unwrap().to_str().unwrap();
    let pending: Pending = Default::default();
    let level = PermissionMode::Edit;
    let mut capped = destroy_engine(work.path(), level, true, false, &pending);
    capped.session_permissions.try_save(saved.path()).unwrap();
    assert!(SessionPermissions::load(saved.path()).interactive);
    capped.respec(|spec| {
        *spec = spec.clone().typed(
            SessionPolicy::person(vec![], vec![]),
            Memory::None,
            Some(level),
        );
    });
    capped.load_session_permissions(sid);

    let mut events = capped.tools.ask_user_bridge().unwrap().events_tx.listen();
    let wait = std::time::Duration::from_secs(5);
    let denied = tokio::time::timeout(wait, async {
        tokio::select! {
            biased;
            event = events.recv() => panic!("capped turn emitted {event:?}"),
            result = judged(&mut capped, "TDestroy") => result.expect("refused"),
        }
    })
    .await
    .expect("refused without waiting for a confirmation");
    assert!(denied.contains("destroys"), "{denied}");
    assert!(events.try_recv().is_err(), "no confirmation was issued");
    assert!(pending.lock().await.is_empty(), "no pending confirmation");
    assert!(!capped.session_permissions.interactive, "settled after load");

    let mut owner = destroy_engine(work.path(), level, true, false, &pending);
    owner.load_session_permissions(sid);
    let mut events = owner.tools.ask_user_bridge().unwrap().events_tx.listen();
    let answer = answer_first(&pending, "Deny");
    assert!(tokio::time::timeout(wait, judged(&mut owner, "TDestroy"))
        .await
        .unwrap()
        .is_some());
    let (labels, caller) = tokio::time::timeout(wait, answer).await.unwrap().unwrap();
    assert_eq!(labels, ONCE_OR_DENY);
    assert!(caller.is_none(), "the owner, not a delegated caller");
    assert!(matches!(events.try_recv(), Ok(ServerEvent::AskUser { .. })));
    assert!(pending.lock().await.is_empty(), "the owner answered");

    // The page has already confirmed the same action. It still runs at edit.
    let app = format!(
        "---\nname: zz-confirmation\ndescription: test app\npermission:\n  paths:\n    - {{ path: \"{}\", mode: edit }}\ntools:\n  - name: TDestroy\n    description: d\n    cmd: echo ok\n    tier: destructive\n---\nbody\n",
        work.path().display()
    );
    let mut app = crate::extensions::skills::parse_skill_text(
        &app,
        crate::engine::skill::SkillSource::Project,
    )
    .unwrap();
    app.skill_dir = Some(work.path().to_path_buf());
    let page = crate::engine::skill::tools::run_at(
        &app,
        "TDestroy",
        &serde_json::json!({}),
        &Default::default(),
        level,
    )
    .await
    .expect("an edit-level page caller can run the confirmed action");
    let output = crate::engine::skill::tools::output(page);
    assert_eq!(output["exit_code"], 0);
    assert_eq!(output["stdout"].as_str().unwrap().trim(), "ok");
}

/// Whatever was granted — admin on the folder and on everything — the person
/// is asked, and only "Allow once" or "Deny" answers.
#[tokio::test]
async fn a_model_that_asks_for_a_destructive_action_asks_the_person_even_with_admin_granted() {
    let work = in_home();
    let wait = std::time::Duration::from_secs(5);
    for answer in ["Deny", crate::engine::permission::ALLOW_ONCE] {
        let pending: Pending = Default::default();
        let seen = answer_first(&pending, answer);
        let mut engine = destroy_engine(work.path(), PermissionMode::Admin, true, false, &pending);
        engine.session_permissions.set_path_mode("*", PermissionMode::Admin);
        let got = judged(&mut engine, "TDestroy").await;
        let (labels, request) = tokio::time::timeout(wait, seen).await.expect("asked").unwrap();
        assert_eq!(labels, ONCE_OR_DENY, "no grant is offered");
        assert!(request.is_none(), "the person, not a caller");
        match answer {
            "Deny" => assert!(got.is_some(), "Deny refuses"),
            _ => assert!(got.is_none(), "Allow once runs it"),
        }
    }
}

/// An earlier yes stands for nothing: the same call asks again.
#[tokio::test]
async fn a_yes_to_one_destructive_call_is_not_a_yes_to_the_next() {
    let work = in_home();
    let pending: Pending = Default::default();
    let mut engine = destroy_engine(work.path(), PermissionMode::Admin, true, false, &pending);
    for round in 1..=2 {
        let seen = answer_first(&pending, crate::engine::permission::ALLOW_ONCE);
        assert!(judged(&mut engine, "TDestroy").await.is_none(), "round {round}");
        tokio::time::timeout(std::time::Duration::from_secs(5), seen).await.unwrap_or_else(|_| panic!("round {round}: not asked")).unwrap();
    }
}

/// A grant below what it needs does not turn the ask into a grant offer.
#[tokio::test]
async fn a_destructive_action_beyond_the_grant_is_the_same_one_ask() {
    let work = in_home();
    let pending: Pending = Default::default();
    let seen = answer_first(&pending, "Deny");
    let mut engine = destroy_engine(work.path(), PermissionMode::Read, true, false, &pending);
    assert!(judged(&mut engine, "TDestroy").await.is_some());
    let (labels, _) = tokio::time::timeout(std::time::Duration::from_secs(5), seen).await.expect("asked").unwrap();
    assert_eq!(labels, ONCE_OR_DENY);
}

/// A run whose caller answers asks the caller, once only.
#[tokio::test]
async fn a_delegated_run_asks_its_caller_once_only_for_a_destructive_action() {
    let work = in_home();
    let pending: Pending = Default::default();
    let seen = answer_first(&pending, crate::engine::permission::ALLOW_ONCE);
    let mut engine = destroy_engine(work.path(), PermissionMode::Admin, false, true, &pending);
    assert!(judged(&mut engine, "TDestroy").await.is_none());
    let (labels, request) = tokio::time::timeout(std::time::Duration::from_secs(5), seen).await.expect("asked").unwrap();
    assert_eq!(labels, ONCE_OR_DENY);
    let request = request.expect("the caller is asked");
    assert!(request.outward && request.dir.is_none(), "only allow_once or deny answers it: {request:?}");
}

/// Nobody to ask — an unattended run, a mission, a turn under a ceiling:
/// refused, with the reason, and nobody is asked.
#[tokio::test]
async fn where_nobody_can_be_asked_a_destructive_action_is_refused_with_its_reason() {
    let work = in_home();
    for ceiling in [None, Some(PermissionMode::Edit)] {
        let pending: Pending = Default::default();
        let mut engine = destroy_engine(work.path(), PermissionMode::Admin, false, false, &pending);
        engine.session_spec.ceiling = ceiling;
        let told = judged(&mut engine, "TDestroy").await.expect("refused");
        assert!(told.contains("TDestroy") && told.contains("destroys"), "{ceiling:?}: {told}");
        assert!(pending.lock().await.is_empty(), "nobody is asked");
    }
    // Above the turn's ceiling it is the ceiling that refuses, as for any action.
    let pending: Pending = Default::default();
    let mut engine = destroy_engine(work.path(), PermissionMode::Admin, false, false, &pending);
    engine.session_spec.ceiling = Some(PermissionMode::Read);
    let told = judged(&mut engine, "TDestroy").await.expect("refused");
    assert!(told.starts_with("Tool TDestroy: Beyond this turn's level"), "{told}");
}

/// A "no" to a destructive action is a no like any other: the folder is
/// closed to what would be asked next in the run, nothing the grants
/// already allow.
#[tokio::test]
async fn a_no_to_a_destructive_action_closes_the_folder_to_other_asks_for_the_run() {
    let work = in_home();
    let pending: Pending = Default::default();
    let seen = answer_first(&pending, "Deny");
    let mut engine = destroy_engine(work.path(), PermissionMode::Read, true, false, &pending);
    assert!(judged(&mut engine, "TDestroy").await.is_some());
    tokio::time::timeout(std::time::Duration::from_secs(5), seen).await.expect("asked").unwrap();
    let again = judged(&mut engine, "TEdit").await.expect("refused");
    assert!(again.contains("declined access to"), "{again}");
    assert!(pending.lock().await.is_empty(), "and not asked again");
    assert!(judged(&mut engine, "TRead").await.is_none(), "what the grant allows still runs");
}
