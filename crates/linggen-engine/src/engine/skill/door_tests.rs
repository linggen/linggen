//! Who may run an app's declared action, and what is said when not — pinned
//! at every way in that is not a model's call: the page's door
//! (`tools::run_at`), the companion's (`tools::run_for_pet`), the lists the
//! door and the companion offer (`remote_tools`, `pet_tools`). Each class
//! (the words a skill writes as `tier:`) meets each grant the skill gives
//! its folder and each level a caller stands at.

use super::outside::{host_tools, run_for_host};
use super::tools::{pet_tools, remote_tools, run_at, run_for_pet, Refusal};
use super::params::SessionParams;
use super::{Skill, SkillSource};
use crate::engine::permission::PermissionMode;
use std::path::Path;

/// One tool per way of writing a tier: a word, or none at all. What each
/// needs of a grant and a level is `needs`.
const WORDS: [(&str, Option<&str>, PermissionMode); 8] = [
    ("TRead", Some("read"), PermissionMode::Read),
    ("TEdit", Some("edit"), PermissionMode::Edit),
    ("TWrite", Some("write"), PermissionMode::Edit),
    ("TDestroy", Some("destructive"), PermissionMode::Edit),
    ("TAdmin", Some("admin"), PermissionMode::Admin),
    ("TAbsent", None, PermissionMode::Admin),
    ("TChat", Some("chat"), PermissionMode::Admin),
    ("TNonsense", Some("nonsense"), PermissionMode::Admin),
];

fn skill(dir: &Path, grant: &str) -> Skill {
    let mut tools = String::new();
    for (name, word, _) in WORDS {
        let tier = word.map(|w| format!("    tier: {w}\n")).unwrap_or_default();
        tools.push_str(&format!(
            "  - name: {name}\n    description: d\n    cmd: \"echo ok\"\n{tier}    pet: true\n    remote: true\n"
        ));
    }
    let text = format!(
        "---\nname: zz-door\ndescription: t\npermission:\n  paths:\n    - {{ path: \"{}\", mode: {grant} }}\ntools:\n{tools}  - name: Note\n    description: d\n    cmd: \"echo ok\"\n    tier: read\n  - name: Shown\n    description: d\n    tier: read\n    pet: true\n---\nbody\n",
        dir.to_string_lossy()
    );
    let mut s = crate::extensions::skills::parse_skill_text(&text, SkillSource::Project).expect("parses");
    s.skill_dir = Some(dir.to_path_buf());
    s
}

fn args() -> serde_json::Value {
    serde_json::json!({})
}

const GRANTS: [(&str, PermissionMode); 3] = [
    ("read", PermissionMode::Read),
    ("edit", PermissionMode::Edit),
    ("admin", PermissionMode::Admin),
];

/// The page's door: the grant of the skill's own folder first, then the
/// level the caller stands at; each refusal says which, in these words.
#[tokio::test]
async fn the_page_door_judges_every_class_by_the_grant_then_the_level() {
    let dir = tempfile::tempdir().unwrap();
    for (grant_word, granted) in GRANTS {
        let s = skill(dir.path(), grant_word);
        for (name, word, needs) in WORDS {
            for level in PermissionMode::ALL {
                let got = run_at(&s, name, &args(), &SessionParams::new(), level).await;
                let at = format!("{name} ({word:?}) grant {granted} caller {level}");
                if needs > granted {
                    match got {
                        Err(Refusal::BeyondGrant(why)) => assert_eq!(
                            why,
                            format!("'{name}' needs {needs} on {}; the skill grants {granted}", dir.path().display()),
                            "{at}"
                        ),
                        other => panic!("{at}: {other:?}"),
                    }
                } else if needs > level {
                    match got {
                        Err(refusal @ Refusal::BeyondLevel { .. }) => {
                            let Refusal::BeyondLevel { level: l, needs: n } = &refusal else { unreachable!() };
                            assert_eq!((*l, *n), (level, needs), "{at}");
                            assert_eq!(
                                refusal.to_string(),
                                format!("beyond_level: the caller stands at {level}; the tool needs {needs}"),
                                "{at}"
                            );
                        }
                        other => panic!("{at}: {other:?}"),
                    }
                } else {
                    assert!(got.is_ok(), "{at}: {got:?}");
                }
            }
        }
    }
}

/// This machine's own call (`run_at` at admin) and the plain `run` agree.
#[tokio::test]
async fn this_machines_own_call_stands_at_admin() {
    let dir = tempfile::tempdir().unwrap();
    let s = skill(dir.path(), "admin");
    for (name, _, _) in WORDS {
        assert!(super::tools::run(&s, name, &args(), &SessionParams::new()).await.is_ok(), "{name}");
    }
    let s = skill(dir.path(), "edit");
    assert!(super::tools::run(&s, "TEdit", &args(), &SessionParams::new()).await.is_ok());
    assert!(matches!(
        super::tools::run(&s, "TAdmin", &args(), &SessionParams::new()).await,
        Err(Refusal::BeyondGrant(_))
    ));
}

/// The companion runs only what the skill offers her (`pet: true`) and
/// whose class is read; the offer and the class are asked before anything
/// else about the call, in this order.
#[tokio::test]
async fn the_companion_runs_only_offered_read_tools() {
    let dir = tempfile::tempdir().unwrap();
    for (grant_word, _) in GRANTS {
        let s = skill(dir.path(), grant_word);
        for (name, word, needs) in WORDS {
            let got = run_for_pet(&s, name, &args(), &SessionParams::new(), false).await;
            let at = format!("{name} ({word:?}) grant {grant_word}");
            if needs > PermissionMode::Read {
                match got {
                    Err(Refusal::NotForPet(why)) => {
                        assert_eq!(why, format!("'{name}' may change 'zz-door'; you may only read"), "{at}")
                    }
                    other => panic!("{at}: {other:?}"),
                }
            } else {
                assert!(got.is_ok(), "{at}: {got:?}");
            }
        }
        // Not offered to her (no `pet: true`): refused before the class.
        match run_for_pet(&s, "Note", &args(), &SessionParams::new(), false).await {
            Err(Refusal::NotForPet(why)) => assert_eq!(why, "'zz-door' does not offer 'Note' to you"),
            other => panic!("{other:?}"),
        }
        // Unknown: no such tool, before anything about her.
        match run_for_pet(&s, "Nope", &args(), &SessionParams::new(), false).await {
            Err(Refusal::NoTool(why)) => assert_eq!(why, "'zz-door' declares no tool 'Nope'"),
            other => panic!("{other:?}"),
        }
        // Offered, read, but it runs nothing: the offer and the class pass,
        // the door then says it is not runnable.
        match run_for_pet(&s, "Shown", &args(), &SessionParams::new(), false).await {
            Err(Refusal::NotRunnable(why)) => assert_eq!(why, "'Shown' runs no command"),
            other => panic!("{other:?}"),
        }
    }
}

/// What is listed to the companion and to another device's agent.
#[test]
fn the_lists_the_door_and_the_companion_offer() {
    let dir = tempfile::tempdir().unwrap();
    for (grant_word, granted) in GRANTS {
        let s = skill(dir.path(), grant_word);
        let pets: Vec<&str> = pet_tools(&s, false).map(|t| t.name.as_str()).collect();
        // `Shown` is a data tool, so not listed; every shell tool whose
        // class is read, whatever the grant.
        assert_eq!(pets, ["TRead"], "grant {grant_word}");
        let remote: Vec<&str> = remote_tools(&s).map(|t| t.name.as_str()).collect();
        let expect: Vec<&str> = WORDS.iter().filter(|(_, _, needs)| *needs <= granted).map(|(n, _, _)| *n).collect();
        assert_eq!(remote, expect, "grant {grant_word}");
    }
}

/// The person's own act — the app's page, a tap on an offer — runs a
/// destructive action as it runs an edit one: their act is their yes, and
/// the page is what confirms. It needs what an edit action needs.
#[tokio::test]
async fn a_persons_own_call_of_a_destructive_action_runs_as_an_edit_one_does() {
    let dir = tempfile::tempdir().unwrap();
    let s = skill(dir.path(), "edit");
    for level in [PermissionMode::Edit, PermissionMode::Admin] {
        assert!(run_at(&s, "TDestroy", &args(), &SessionParams::new(), level).await.is_ok(), "{level}");
    }
    assert!(matches!(
        run_at(&s, "TDestroy", &args(), &SessionParams::new(), PermissionMode::Read).await,
        Err(Refusal::BeyondLevel { needs: PermissionMode::Edit, .. })
    ));
    let read = skill(dir.path(), "read");
    assert!(matches!(
        run_at(&read, "TDestroy", &args(), &SessionParams::new(), PermissionMode::Admin).await,
        Err(Refusal::BeyondGrant(_))
    ));
    assert!(super::tools::run(&s, "TDestroy", &args(), &SessionParams::new()).await.is_ok(), "this machine's own call");
}

/// One tool per class, each offered `anywhere` and not `pet`, and one of
/// each class `pet` only.
fn anywhere_skill(dir: &Path) -> Skill {
    let mut tools = String::new();
    for (name, word, _) in WORDS {
        let tier = word.map(|w| format!("    tier: {w}\n")).unwrap_or_default();
        tools.push_str(&format!(
            "  - name: A{name}\n    description: d\n    cmd: \"echo ok\"\n{tier}    reach: anywhere\n  - name: P{name}\n    description: d\n    cmd: \"echo ok\"\n{tier}    pet: true\n  - name: H{name}\n    description: d\n    cmd: \"echo ok\"\n{tier}"
        ));
    }
    let text = format!(
        "---\nname: zz-door\ndescription: t\npermission:\n  paths:\n    - {{ path: \"{}\", mode: edit }}\ntools:\n{tools}---\nbody\n",
        dir.to_string_lossy()
    );
    let mut s = crate::extensions::skills::parse_skill_text(&text, SkillSource::Project).expect("parses");
    s.skill_dir = Some(dir.to_path_buf());
    s
}

/// What an action offered `anywhere` is to the companion, by its class and
/// by who started the turn: a read always; an edit action only when the
/// person asked her; a destructive one never, the person does it in the
/// app; anything with no class she may run, never. What is offered to her
/// alone (`pet: true`) stays a read, and what is offered by nobody is not
/// hers at all.
#[tokio::test]
async fn the_companion_runs_an_anywhere_action_by_its_class_and_who_asked() {
    let dir = tempfile::tempdir().unwrap();
    let s = anywhere_skill(dir.path());
    let run = |name: String, asked: bool| {
        let s = s.clone();
        async move { run_for_pet(&s, &name, &args(), &SessionParams::new(), asked).await }
    };
    for asked in [false, true] {
        for (name, word, needs) in WORDS {
            let at = format!("A{name} ({word:?}) asked={asked}");
            let got = run(format!("A{name}"), asked).await;
            match (word, needs) {
                (Some("read"), _) => assert!(got.is_ok(), "{at}: {got:?}"),
                (Some("destructive"), _) => match got {
                    Err(Refusal::NotForPet(why)) => assert_eq!(
                        why,
                        format!("'A{name}' of 'zz-door' destroys what it acts on: the person does this themselves, in the app"),
                        "{at}"
                    ),
                    other => panic!("{at}: {other:?}"),
                },
                (Some("edit" | "write"), _) if asked => assert!(got.is_ok(), "{at}: {got:?}"),
                (Some("edit" | "write"), _) => match got {
                    Err(Refusal::NotForPet(why)) => assert!(
                        why.contains("this turn is yours") && why.contains(&format!("'A{name}'")),
                        "{at}: {why}"
                    ),
                    other => panic!("{at}: {other:?}"),
                },
                _ => assert!(matches!(got, Err(Refusal::NotForPet(_))), "{at}: {got:?}"),
            }
            // Her own read offer stays what it was, whoever started the turn.
            let pet = run(format!("P{name}"), asked).await;
            match needs > PermissionMode::Read {
                true => assert!(matches!(pet, Err(Refusal::NotForPet(_))), "P{name} asked={asked}: {pet:?}"),
                false => assert!(pet.is_ok(), "P{name} asked={asked}: {pet:?}"),
            }
            // Offered by nobody: not hers.
            let hidden = run(format!("H{name}"), asked).await;
            assert!(
                matches!(&hidden, Err(Refusal::NotForPet(why)) if why.contains("does not offer")),
                "H{name} asked={asked}: {hidden:?}"
            );
        }
    }
}

/// What is listed to her is what she may call in this turn.
#[test]
fn what_is_listed_to_the_companion_is_what_she_may_call_now() {
    let dir = tempfile::tempdir().unwrap();
    let s = anywhere_skill(dir.path());
    let names = |asked: bool| -> Vec<String> { pet_tools(&s, asked).map(|t| t.name.clone()).collect() };
    assert_eq!(names(false), ["ATRead", "PTRead"]);
    assert_eq!(names(true), ["ATRead", "PTRead", "ATEdit", "ATWrite"]);
}

/// What an outside host (the MCP door's `app` tool) is to an action
/// offered `anywhere`, by its class and the app's own grant: a read and an
/// edit action run under the grant; a destructive one never, the person
/// does it in the app; one above an edit needs, or with no class, never.
/// What is hers alone or kept in the app is not offered to it at all.
#[tokio::test]
async fn an_outside_host_runs_an_anywhere_action_by_its_class_and_the_apps_grant() {
    let dir = tempfile::tempdir().unwrap();
    let s = anywhere_skill(dir.path());
    let run = |name: String| {
        let s = s.clone();
        async move { run_for_host(&s, &name, &args(), &[]).await }
    };
    for (name, word, needs) in WORDS {
        let at = format!("A{name} ({word:?})");
        let got = run(format!("A{name}")).await;
        match word {
            Some("read" | "edit" | "write") => assert!(got.is_ok(), "{at}: {got:?}"),
            Some("destructive") => match got {
                Err(Refusal::NotForHost(why)) => assert_eq!(
                    why,
                    format!("'A{name}' of 'zz-door' destroys what it acts on: the person does this themselves, in the app"),
                    "{at}"
                ),
                other => panic!("{at}: {other:?}"),
            },
            // Above what the app's own grant covers: refused by the grant.
            _ => assert!(
                matches!(got, Err(Refusal::BeyondGrant(_))) && needs > PermissionMode::Edit,
                "{at}: {got:?}"
            ),
        }
        for kept in [format!("P{name}"), format!("H{name}")] {
            let got = run(kept.clone()).await;
            assert!(
                matches!(&got, Err(Refusal::NotForPet(why)) if why.contains("does not offer")),
                "{kept}: {got:?}"
            );
        }
    }
    let names: Vec<&str> = host_tools(&s, &[]).iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, WORDS.iter().map(|(n, _, _)| format!("A{n}")).collect::<Vec<_>>());
}

/// Above the grant the app gives an outside host nothing, an admin grant
/// included: the most it stands at is what an edit needs.
#[tokio::test]
async fn an_outside_host_never_runs_above_an_edit_even_where_the_app_grants_admin() {
    let dir = tempfile::tempdir().unwrap();
    let s = {
        let text = format!(
            "---\nname: zz-door\ndescription: t\npermission:\n  paths:\n    - {{ path: \"{}\", mode: admin }}\ntools:\n  - name: Root\n    description: d\n    cmd: \"echo ok\"\n    tier: admin\n    reach: anywhere\n  - name: Plain\n    description: d\n    cmd: \"echo ok\"\n    reach: anywhere\n---\nbody\n",
            dir.path().to_string_lossy()
        );
        let mut s = crate::extensions::skills::parse_skill_text(&text, SkillSource::Project).unwrap();
        s.skill_dir = Some(dir.path().to_path_buf());
        s
    };
    for name in ["Root", "Plain"] {
        let got = run_for_host(&s, name, &args(), &[]).await;
        assert!(
            matches!(got, Err(Refusal::BeyondLevel { level: PermissionMode::Edit, needs: PermissionMode::Admin })),
            "{name}: {got:?}"
        );
    }
}

/// An app kept private is closed to an outside host until it is opened:
/// not listed, not run, and the reason names the setting.
#[tokio::test]
async fn a_private_app_is_closed_to_an_outside_host_until_opened() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = anywhere_skill(dir.path());
    s.private_talk = true;
    assert!(host_tools(&s, &[]).is_empty());
    assert!(host_tools(&s, &["other".to_string()]).is_empty());
    match run_for_host(&s, "ATRead", &args(), &[]).await {
        Err(Refusal::NotYours(why)) => assert!(why.contains("Settings → Tools & MCP"), "{why}"),
        other => panic!("{other:?}"),
    }
    let open = ["zz-door".to_string()];
    assert!(!host_tools(&s, &open).is_empty());
    assert!(run_for_host(&s, "ATRead", &args(), &open).await.is_ok());
    // A user-only app is no app to a model, opened or not.
    s.disable_model_invocation = true;
    assert!(matches!(run_for_host(&s, "ATRead", &args(), &open).await, Err(Refusal::NotYours(_))));
}
