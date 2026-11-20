use super::*;
use std::fs::{self, write};

fn scope(root: &Path) -> InstructionScope {
    InstructionScope {
        root: root.to_owned(),
        enabled: true,
        permissions: SessionPermissions {
            path_modes: vec![permission::PathMode {
                path: root.display().to_string(),
                mode: PermissionMode::Edit,
            }],
            ..Default::default()
        },
        declined: DeclinedScopes::default(),
        origin: ReadOrigin::LocalOwner,
        coding: true,
        personal: true,
        ceiling: None,
        place_rules: Vec::new(),
    }
}

fn rendered(result: &Instructions) -> String {
    result
        .sections
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
#[cfg(unix)]
fn imports_expand_relative_to_each_source_with_labels_and_cycles_end() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("nested")).unwrap();
    write(root.join(CLAUDE), "@nested/first.md @nested/first.md").unwrap();
    write(root.join("nested/first.md"), "First rule. @second.md").unwrap();
    write(root.join("nested/second.md"), "Second rule. @first.md").unwrap();
    let result = scope(root).load();
    let text = rendered(&result);
    assert!(
        text.contains("[Imported from \"nested/first.md\"]\nFirst rule."),
        "{text}"
    );
    assert!(
        text.contains("[Imported from \"nested/second.md\"]\nSecond rule."),
        "{text}"
    );
    assert_eq!(text.matches("First rule.").count(), 1);
    assert!(!text.contains("@first.md"));
    assert!(result
        .files
        .contains(&root.join("nested/second.md").canonicalize().unwrap()));
}

#[test]
fn mentions_missing_files_and_markdown_code_stay_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root.join("rule.md"), "MUST NOT LOAD").unwrap();
    let text = "@银月 @coder me@example.com @missing.md\n\n`@rule.md` ``code: @rule.md``\n\n```md\n@rule.md\n```\n\n> ~~~~\n> @rule.md\n> ~~~~";
    write(root.join(AGENTS), text).unwrap();
    let result = scope(root).load();
    assert_eq!(
        result
            .sections
            .iter()
            .find(|(name, _)| name == AGENTS)
            .unwrap()
            .1,
        text
    );
    assert!(!rendered(&result).contains("MUST NOT LOAD"));
}

#[test]
#[cfg(unix)]
fn an_upward_escape_absolute_secret_and_outward_symlink_each_leave_a_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    fs::create_dir(&root).unwrap();
    let outside = dir.path().join("outside.md");
    write(&outside, "OUTSIDE CONTENT").unwrap();
    std::os::unix::fs::symlink(outside, root.join("link.md")).unwrap();
    let secret = crate::paths::linggen_home().join("credentials.json");
    // Do not create or read the engine's credentials. The absolute fixture
    // is another protected path, with invented contents in this test's home.
    let fake_home = tempfile::tempdir().unwrap();
    let fake_secret = fake_home
        .path()
        .join(".ssh")
        .join(secret.file_name().unwrap());
    fs::create_dir(fake_secret.parent().unwrap()).unwrap();
    write(&fake_secret, "INVENTED SECRET").unwrap();
    write(
        root.join(CLAUDE),
        format!("@../outside.md\n@{}\n@link.md", fake_secret.display()),
    )
    .unwrap();
    let text = rendered(&scope(&root).load());
    for token in ["../outside.md", fake_secret.to_str().unwrap(), "link.md"] {
        assert!(
            text.contains(&format!(
                "[Import @{token} not loaded: outside the project.]"
            )),
            "{text}"
        );
    }
    assert!(!text.contains("OUTSIDE CONTENT") && !text.contains("INVENTED SECRET"));
}

#[test]
#[cfg(unix)]
fn protected_places_are_not_imported_even_with_a_named_grant() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root.join(".env"), "INVENTED SECRET").unwrap();
    std::os::unix::fs::symlink(root.join(".env"), root.join("alias.md")).unwrap();
    write(root.join(AGENTS), "@alias.md").unwrap();
    let mut scope = scope(root);
    scope.permissions.path_modes.push(permission::PathMode {
        path: root.join(".env").display().to_string(),
        mode: PermissionMode::Read,
    });
    let text = rendered(&scope.load());
    assert!(
        text.contains("[Import @alias.md not loaded: protected or requires approval.]"),
        "{text}"
    );
    assert!(!text.contains("INVENTED SECRET"));
}

#[test]
#[cfg(unix)]
fn imports_respect_the_turn_ceiling_and_reject_binary_and_oversize_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root.join(CLAUDE), "@notes.md @binary @large").unwrap();
    write(root.join("notes.md"), "rule text").unwrap();
    write(root.join("binary"), b"contains\0nul").unwrap();
    write(root.join("large"), vec![b'x'; MAX_FILE + 1]).unwrap();
    let mut scope = scope(root);
    let text = rendered(&scope.load());
    assert!(text.contains("rule text"));
    assert!(text.contains("[Import @binary not loaded: not UTF-8 text.]"));
    assert!(text.contains("[Import @large not loaded: over file size limit.]"));
    scope.ceiling = Some(PermissionMode::Chat);
    let text = rendered(&scope.load());
    assert!(!text.contains("rule text"));
    assert!(text.contains("[Import @notes.md not loaded: protected or requires approval.]"));
}

#[test]
#[cfg(unix)]
fn changing_only_an_import_changes_the_instruction_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root.join(CLAUDE), "@notes.md").unwrap();
    write(root.join("notes.md"), "before").unwrap();
    let scope = scope(root);
    let first = scope.load();
    assert_eq!(first.fingerprint(), scope.load().fingerprint());
    write(root.join("notes.md"), "after").unwrap();
    let second = scope.load();
    assert_ne!(first.fingerprint(), second.fingerprint());
    assert!(rendered(&second).contains("after"));
}

#[test]
fn root_home_and_machine_local_rules_load_but_home_parents_do_not() {
    let home = dirs::home_dir().unwrap();
    let dir = tempfile::tempdir_in(home).unwrap();
    let home = dir.path().join("home");
    let root = home.join("project");
    fs::create_dir_all(&root).unwrap();
    write(dir.path().join(CLAUDE), "above home").unwrap();
    write(home.join(AGENTS), "home rule").unwrap();
    write(root.join(CLAUDE), "root rule").unwrap();
    write(root.join(LOCAL), "local rule").unwrap();
    let mut scope = scope(&root);
    let text = rendered(&scope.load_with_home(Some(&home)));
    assert!(text.find("home rule").unwrap() < text.find("root rule").unwrap());
    assert!(text.find("root rule").unwrap() < text.find("local rule").unwrap());
    assert!(!text.contains("above home"));
    scope.enabled = false;
    assert!(scope.load_with_home(Some(&home)).sections.is_empty());
}

#[test]
#[cfg(unix)]
fn discovered_symlinks_follow_the_import_gate_and_explain_refusals() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    fs::create_dir_all(root.join("docs")).unwrap();
    write(root.join("docs/agents.md"), "Linked project rule").unwrap();
    write(dir.path().join("outside.md"), "outside contents").unwrap();
    write(root.join(".env"), "protected contents").unwrap();
    symlink("docs/agents.md", root.join(AGENTS)).unwrap();
    symlink("../outside.md", root.join(CLAUDE)).unwrap();
    symlink(".env", root.join(LOCAL)).unwrap();
    symlink("missing.md", root.join(CURSOR)).unwrap();
    let result = scope(&root).load_with_home(Some(&root));
    let section = |name| {
        result
            .sections
            .iter()
            .find(|(label, _)| label == name)
            .map(|(_, text)| text.as_str())
    };
    assert_eq!(section(AGENTS), Some("Linked project rule"));
    assert!(section(CLAUDE).unwrap().contains("outside the project"));
    assert!(section(LOCAL)
        .unwrap()
        .contains("protected or requires approval"));
    assert!(section(CURSOR).unwrap().contains("unreadable"));
    let text = rendered(&result);
    assert!(!text.contains("outside contents") && !text.contains("protected contents"));
}

#[test]
#[cfg(unix)]
fn discovered_rules_load_ordinary_large_text_and_explain_each_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let ordinary = "a".repeat(60 * 1024);
    write(root.join(AGENTS), &ordinary).unwrap();
    write(root.join(LOCAL), vec![b'b'; 256 * 1024 + 1]).unwrap();
    write(root.join(CLAUDE), b"binary\0contents").unwrap();
    write(root.join("alias"), "hard-linked contents").unwrap();
    fs::hard_link(root.join("alias"), root.join(CURSOR)).unwrap();
    let result = scope(root).load_with_home(Some(root));
    let section = |name| {
        result
            .sections
            .iter()
            .find(|(label, _)| label == name)
            .unwrap()
            .1
            .as_str()
    };
    assert_eq!(section(AGENTS), ordinary);
    assert!(section(LOCAL).contains("file size limit"));
    assert!(section(CLAUDE).contains("not UTF-8 text"));
    assert!(section(CURSOR).contains("hard-linked"));

    write(root.join(AGENTS), vec![b'a'; 256 * 1024]).unwrap();
    write(root.join(LOCAL), vec![b'b'; 256 * 1024]).unwrap();
    let text = rendered(&scope(root).load_with_home(Some(root)));
    assert!(text.contains("total size limit"));
}

#[test]
#[cfg(unix)]
fn ordinary_mentions_do_not_spend_the_import_budget() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mentions = "@coder @银月 ".repeat(MAX_IMPORTS);
    write(root.join(CLAUDE), format!("{mentions}@style.md")).unwrap();
    write(root.join("style.md"), "Use the style guide").unwrap();
    let text = rendered(&scope(root).load_with_home(Some(root)));
    assert!(text.contains(mentions.trim_end()));
    assert!(text.contains("Use the style guide"));
}

#[test]
#[cfg(unix)]
fn sentence_end_imports_keep_punctuation_unless_it_names_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root.join(CLAUDE),
        "Follow @style.md.\nThen (@other.md)!\nRead @literal.md.",
    )
    .unwrap();
    for (name, text) in [
        ("style.md", "Style rule"),
        ("other.md", "Other rule"),
        ("literal.md.", "Literal filename rule"),
        ("literal.md", "Wrong file"),
    ] {
        write(root.join(name), text).unwrap();
    }
    let text = rendered(&scope(root).load_with_home(Some(root)));
    assert!(text.contains("Style rule\n[End import]\n\n."), "{text}");
    assert!(text.contains("Other rule\n[End import]\n\n)!"), "{text}");
    assert!(
        text.ends_with("Literal filename rule\n[End import]"),
        "{text}"
    );
    assert!(!text.contains("Wrong file"));
}

fn personal_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let root = home.join("work");
    let personal = personal_rule_path(&home);
    fs::create_dir_all(&root).unwrap();
    fs::create_dir(personal.parent().unwrap()).unwrap();
    (dir, root, personal)
}

#[test]
#[cfg(unix)]
fn personal_rules_precede_project_rules_and_only_they_can_import_outside() {
    let (_dir, root, personal) = personal_fixture();
    let home = root.parent().unwrap();
    write(home.join("style.md"), "PERSONAL STYLE\n@details.md").unwrap();
    write(home.join("details.md"), "PERSONAL DETAILS").unwrap();
    write(home.join("project-escape.md"), "PROJECT ESCAPE").unwrap();
    write(root.join("shared.md"), "SHARED RULE\n@~/project-escape.md").unwrap();
    write(&personal, "PERSONAL RULE\n@~/style.md\n@~/work/shared.md").unwrap();
    write(root.join(AGENTS), "PROJECT RULE\n@~/project-escape.md").unwrap();

    let mut scope = scope(&root);
    let result = scope.load_with_home(Some(home));
    let (label, text) = &result.sections[0];
    assert!(
        label.contains("Personal") && label.contains(personal.to_str().unwrap()),
        "{label}"
    );
    for expected in [
        "PERSONAL RULE",
        "PERSONAL STYLE",
        "PERSONAL DETAILS",
        "SHARED RULE",
    ] {
        assert!(text.contains(expected), "{text}");
    }
    assert!(text.contains("[Import @~/project-escape.md not loaded: outside the project.]"));
    assert!(result.sections[1].1.contains("PROJECT RULE"));
    assert!(result.sections[1]
        .1
        .contains("[Import @~/project-escape.md not loaded: outside the project.]"));
    assert!(!rendered(&result).contains("PROJECT ESCAPE"));

    scope.personal = false;
    let text = rendered(&scope.load_with_home(Some(home)));
    assert!(text.contains("PROJECT RULE"));
    assert!(!text.contains("PERSONAL RULE") && !text.contains("PERSONAL STYLE"));
}

#[test]
#[cfg(unix)]
fn personal_rule_symlinks_follow_the_gate_and_missing_rules_leave_nothing() {
    use std::os::unix::fs::symlink;
    let (_dir, root, personal) = personal_fixture();
    let home = root.parent().unwrap();
    let scope = scope(&root);
    assert!(scope.load_with_home(Some(home)).sections.is_empty());
    write(home.join("dotfile.md"), "LINKED PERSONAL RULE").unwrap();
    symlink("../dotfile.md", &personal).unwrap();
    assert!(rendered(&scope.load_with_home(Some(home))).contains("LINKED PERSONAL RULE"));

    fs::remove_file(&personal).unwrap();
    write(home.join(".env"), "PROTECTED PERSONAL TARGET").unwrap();
    symlink("../.env", &personal).unwrap();
    let text = rendered(&scope.load_with_home(Some(home)));
    assert!(
        text.contains("[Rule not loaded: protected or requires approval.]"),
        "{text}"
    );
    assert!(!text.contains("PROTECTED PERSONAL TARGET"));
}

#[test]
#[cfg(unix)]
fn changes_to_personal_rules_and_their_imports_change_the_fingerprint() {
    let (_dir, root, personal) = personal_fixture();
    let home = root.parent().unwrap();
    let style = home.join("style.md");
    write(&style, "original style").unwrap();
    write(&personal, "original rule\n@~/style.md").unwrap();
    let scope = scope(&root);
    let load = || scope.load_with_home(Some(home));
    let first = load().fingerprint();
    write(&personal, "updated rule\n@~/style.md").unwrap();
    let second = load().fingerprint();
    assert_ne!(first, second);
    write(&style, "updated style").unwrap();
    let third = load();
    assert_ne!(second, third.fingerprint());
    assert!(rendered(&third).contains("updated style"));
}

fn place_fixture() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("project")).unwrap();
    let root = dir.path().join("project").canonicalize().unwrap();
    fs::create_dir_all(root.join("ui/src")).unwrap();
    fs::create_dir_all(root.join("crates/x/src")).unwrap();
    fs::create_dir_all(rules_dir(&root).join("lang")).unwrap();
    write(root.join(CLAUDE), "ROOT RULE").unwrap();
    write(root.join("ui").join(CLAUDE), "UI RULE").unwrap();
    write(root.join("crates/x").join(AGENTS), "CRATE RULE").unwrap();
    write(
        rules_dir(&root).join("api.md"),
        "---\npaths:\n  - \"src/api/**/*.ts\"\n---\nAPI RULE",
    )
    .unwrap();
    write(
        rules_dir(&root).join("lang/docs.md"),
        "---\npaths: \"docs/*.md, *.txt\"\n---\nDOCS RULE",
    )
    .unwrap();
    write(rules_dir(&root).join("style.md"), "STYLE EVERYWHERE").unwrap();
    (dir, root)
}

fn met(scope: &InstructionScope, file: &Path) -> Vec<PathBuf> {
    scope.place_rules_for(file)
}

#[test]
fn a_sub_folder_rule_reaches_only_work_under_its_folder() {
    let (_dir, root) = place_fixture();
    let mut scope = scope(&root);
    let base = rendered(&scope.load_with_home(None));
    assert!(
        base.contains("ROOT RULE") && !base.contains("UI RULE") && !base.contains("CRATE RULE"),
        "{base}"
    );

    assert_eq!(
        met(&scope, &root.join("ui/src/new-file.ts")),
        [root.join("ui").join(CLAUDE)]
    );
    assert_eq!(
        met(&scope, &root.join("crates/x/src/lib.rs")),
        [root.join("crates/x").join(AGENTS)]
    );
    assert!(met(&scope, &root.join("README.md")).is_empty());
    assert!(met(&scope, &root.join("uix/a.ts")).is_empty());

    scope.place_rules = met(&scope, &root.join("ui/src/a.ts"));
    let result = scope.load_with_home(None);
    let text = rendered(&result);
    assert_eq!(text.matches("UI RULE").count(), 1, "{text}");
    assert!(!text.contains("CRATE RULE"));
    assert!(result
        .sections
        .iter()
        .any(|(label, _)| label == "ui/CLAUDE.md (for files under ui/)"));
}

#[test]
fn a_path_scoped_rule_reaches_only_files_its_globs_match() {
    let (_dir, root) = place_fixture();
    let mut scope = scope(&root);
    let base = rendered(&scope.load_with_home(None));
    assert!(base.contains("STYLE EVERYWHERE"), "{base}");
    assert!(
        !base.contains("API RULE") && !base.contains("DOCS RULE"),
        "{base}"
    );

    let api = rules_dir(&root).join("api.md");
    assert_eq!(
        met(&scope, &root.join("src/api/v1/users.ts")),
        [api.clone()]
    );
    assert!(met(&scope, &root.join("src/api/v1/users.js")).is_empty());
    assert!(met(&scope, &root.join("src/other/users.ts")).is_empty());
    assert_eq!(
        met(&scope, &root.join("notes.txt")),
        [rules_dir(&root).join("lang/docs.md")]
    );
    assert!(met(&scope, &root.join("docs/deep/a.md")).is_empty());

    scope.place_rules = vec![api];
    let result = scope.load_with_home(None);
    let (label, text) = result
        .sections
        .iter()
        .find(|(_, text)| text.contains("API RULE"))
        .unwrap();
    assert_eq!(label, ".claude/rules/api.md (for src/api/**/*.ts)");
    assert_eq!(text, "API RULE", "front matter is not rule text");
}

#[test]
fn place_rules_are_for_coding_sessions_only() {
    let (_dir, root) = place_fixture();
    let mut scope = scope(&root);
    scope.personal = false;
    assert!(met(&scope, &root.join("ui/src/a.ts")).is_empty());
    scope.place_rules = vec![root.join("ui").join(CLAUDE)];
    let text = rendered(&scope.load_with_home(None));
    assert!(
        text.contains("ROOT RULE")
            && !text.contains("UI RULE")
            && !text.contains("STYLE EVERYWHERE")
    );
}

#[test]
#[cfg(unix)]
fn a_place_rule_imports_only_inside_the_project_and_never_a_protected_file() {
    let (dir, root) = place_fixture();
    write(dir.path().join("outside.md"), "OUTSIDE CONTENT").unwrap();
    write(root.join(".env"), "INVENTED SECRET").unwrap();
    write(root.join("ui/shared.md"), "SHARED UI").unwrap();
    write(
        root.join("ui").join(CLAUDE),
        "UI RULE\n@shared.md\n@../../outside.md\n@../.env",
    )
    .unwrap();
    write(
        rules_dir(&root).join("api.md"),
        "---\npaths: src/**\n---\nAPI RULE @../../../outside.md",
    )
    .unwrap();
    let mut scope = scope(&root);
    scope.place_rules = vec![
        root.join("ui").join(CLAUDE),
        rules_dir(&root).join("api.md"),
    ];
    let text = rendered(&scope.load_with_home(None));
    assert!(text.contains("SHARED UI"), "{text}");
    assert!(
        text.contains("[Import @../../outside.md not loaded: outside the project.]"),
        "{text}"
    );
    assert!(
        text.contains("[Import @../../../outside.md not loaded: outside the project.]"),
        "{text}"
    );
    assert!(
        text.contains("[Import @../.env not loaded: protected or requires approval.]"),
        "{text}"
    );
    assert!(!text.contains("OUTSIDE CONTENT") && !text.contains("INVENTED SECRET"));
}

#[test]
fn a_path_rule_that_cannot_be_read_says_so_and_a_change_to_a_met_rule_is_picked_up() {
    let (_dir, root) = place_fixture();
    write(
        rules_dir(&root).join("broken.md"),
        "---\npaths: [unclosed\n",
    )
    .unwrap();
    let mut scope = scope(&root);
    let base = rendered(&scope.load_with_home(None));
    assert!(
        base.contains("[Rule not loaded: front matter not closed.]"),
        "{base}"
    );
    scope.place_rules = vec![root.join("ui").join(CLAUDE)];
    let first = scope.load_with_home(None).fingerprint();
    write(root.join("ui").join(CLAUDE), "UI RULE two").unwrap();
    let second = scope.load_with_home(None);
    assert_ne!(first, second.fingerprint());
    assert!(rendered(&second).contains("UI RULE two"));
}

#[test]
fn native_instruction_filenames_live_in_one_place() {
    fn check(dir: &Path) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                check(&path);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && !path.ends_with("engine/instruction_files.rs")
            {
                let source = fs::read_to_string(&path).unwrap();
                let personal = personal_rule_path(Path::new(""));
                let rules = rules_dir(Path::new(""));
                for name in NAMES
                    .iter()
                    .copied()
                    .chain(personal.to_str())
                    .chain(rules.to_str())
                {
                    assert!(
                        !source.contains(&format!("\"{name}\"")),
                        "{} repeats the native instruction name {name}",
                        path.display()
                    );
                }
            }
        }
    }
    check(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src/engine"));
}
