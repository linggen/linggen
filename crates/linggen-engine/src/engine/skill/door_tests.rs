//! Who may run an app's declared action, and what is said when not — pinned
//! at every way in that is not a model's call: the page's door
//! (`tools::run_at`), the companion's (`tools::run_for_pet`), the lists the
//! door and the companion offer (`remote_tools`, `pet_tools`). Each class
//! (the words a skill writes as `tier:`) meets each grant the skill gives
//! its folder and each level a caller stands at.

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
            let got = run_for_pet(&s, name, &args(), &SessionParams::new()).await;
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
        match run_for_pet(&s, "Note", &args(), &SessionParams::new()).await {
            Err(Refusal::NotForPet(why)) => assert_eq!(why, "'zz-door' does not offer 'Note' to you"),
            other => panic!("{other:?}"),
        }
        // Unknown: no such tool, before anything about her.
        match run_for_pet(&s, "Nope", &args(), &SessionParams::new()).await {
            Err(Refusal::NoTool(why)) => assert_eq!(why, "'zz-door' declares no tool 'Nope'"),
            other => panic!("{other:?}"),
        }
        // Offered, read, but it runs nothing: the offer and the class pass,
        // the door then says it is not runnable.
        match run_for_pet(&s, "Shown", &args(), &SessionParams::new()).await {
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
        let pets: Vec<&str> = pet_tools(&s).map(|t| t.name.as_str()).collect();
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
