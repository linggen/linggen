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
        .watch
        .names
        .contains_key(&root.join("nested/second.md").canonicalize().unwrap()));
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
fn changing_only_an_import_makes_the_snapshot_stale() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root.join(CLAUDE), "@notes.md").unwrap();
    write(root.join("notes.md"), "before").unwrap();
    let scope = scope(root);
    let first = scope.load();
    assert!(!first.watch.is_stale(&scope));
    write(root.join("notes.md"), "after").unwrap();
    assert!(first.watch.is_stale(&scope));
    let second = scope.load();
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
    assert_eq!(section(CURSOR), "hard-linked contents");

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
fn changes_to_personal_rules_and_their_imports_make_the_snapshot_stale() {
    let (_dir, root, personal) = personal_fixture();
    let home = root.parent().unwrap();
    let style = home.join("style.md");
    write(&style, "original style").unwrap();
    write(&personal, "original rule\n@~/style.md").unwrap();
    let scope = scope(&root);
    let load = || scope.load_with_home(Some(home));
    let first = load();
    write(&personal, "updated personal rule\n@~/style.md").unwrap();
    assert!(first.watch.is_stale(&scope));
    let second = load();
    write(&style, "updated style").unwrap();
    assert!(second.watch.is_stale(&scope));
    let third = load();
    assert!(rendered(&third).contains("updated style"));
}

#[test]
#[cfg(unix)]
fn review_discovered_hard_links_load_but_imported_hard_links_do_not() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root.join(AGENTS), "shared rule").unwrap();
    fs::hard_link(root.join(AGENTS), root.join(CLAUDE)).unwrap();
    fs::hard_link(root.join(AGENTS), root.join("alias.md")).unwrap();
    write(root.join(LOCAL), "@alias.md").unwrap();
    std::os::unix::fs::symlink("alias.md", root.join(CURSOR)).unwrap();
    let text = rendered(&scope(root).load_with_home(Some(root)));
    assert_eq!(text.matches("shared rule").count(), 2, "{text}");
    assert!(
        text.contains("[Import @alias.md not loaded: hard-linked file.]"),
        "{text}"
    );
    assert!(
        text.contains("[Rule not loaded: hard-linked file.]"),
        "{text}"
    );
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn review_discovered_rules_load_through_search_only_directories() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let search = dir.path().join("search");
    let root = search.join("work");
    fs::create_dir_all(&root).unwrap();
    write(root.join(AGENTS), "search-only rule\n@notes.md").unwrap();
    write(root.join("notes.md"), "search-only import").unwrap();
    // 711 reproduces the mount layout; 111 also takes listing permission
    // away from the test process, which owns the fixture directory.
    for mode in [0o711, 0o111] {
        fs::set_permissions(&search, fs::Permissions::from_mode(mode)).unwrap();
        let text = rendered(&scope(&root).load_with_home(Some(&root)));
        fs::set_permissions(&search, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(text.contains("search-only rule"), "{mode:o}: {text}");
        assert!(text.contains("search-only import"), "{mode:o}: {text}");
    }
}

#[test]
#[cfg(target_os = "macos")]
fn review_case_insensitive_rule_name_is_not_a_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("work");
    fs::create_dir(&root).unwrap();
    write(
        dir.path().join(AGENTS.to_ascii_lowercase()),
        "case variant rule",
    )
    .unwrap();
    if !dir.path().join(AGENTS).exists() {
        return; // This volume is case-sensitive.
    }
    let text = rendered(&scope(&root).load_with_home(Some(dir.path())));
    assert!(text.contains("case variant rule"), "{text}");
}

#[test]
#[cfg(unix)]
fn review_personal_rule_is_plain_under_a_symlinked_home() {
    let (dir, root, personal) = personal_fixture();
    let home = root.parent().unwrap();
    let alias = dir.path().join("mounted-home");
    std::os::unix::fs::symlink(home, &alias).unwrap();
    write(&personal, "personal rule\n@~/style.md").unwrap();
    write(home.join("style.md"), "personal import").unwrap();
    let mut scope = scope(&root);
    scope.ceiling = Some(PermissionMode::Chat);
    let plain = rendered(&scope.load_with_home(Some(home)));
    let linked = rendered(&scope.load_with_home(Some(&alias)));
    assert!(plain.contains("personal rule"));
    assert_eq!(linked, plain);
    assert!(linked.contains("protected or requires approval"));
    assert!(!linked.contains("personal import"));
}

#[test]
fn review_rejected_text_does_not_spend_size_allowance() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root.join(AGENTS), vec![0xff; MAX_FILE]).unwrap();
    write(root.join(LOCAL), vec![0; MAX_FILE]).unwrap();
    write(root.join(CLAUDE), "accepted rule").unwrap();
    let text = rendered(&scope(root).load_with_home(Some(root)));
    assert_eq!(text.matches("not UTF-8 text").count(), 2, "{text}");
    assert!(text.contains("accepted rule"), "{text}");
}

#[test]
#[cfg(unix)]
fn review_personal_rules_do_not_spend_project_size_or_import_allowance() {
    let (_dir, root, personal) = personal_fixture();
    let home = root.parent().unwrap();
    write(home.join("style.md"), "personal import").unwrap();
    write(
        &personal,
        format!(
            "{}{}",
            "@~/style.md ".repeat(MAX_IMPORTS),
            "p".repeat(MAX_FILE / 2)
        ),
    )
    .unwrap();
    write(root.join(AGENTS), vec![b'a'; MAX_FILE]).unwrap();
    write(
        root.join(CLAUDE),
        format!("@project.md\n{}", "b".repeat(MAX_FILE - 64)),
    )
    .unwrap();
    write(root.join("project.md"), "project import").unwrap();
    let result = scope(&root).load_with_home(Some(home));
    assert!(result.sections[0].1.contains("personal import"));
    let text = rendered(&result);
    assert!(!text.contains("total size limit"));
    assert!(text.contains("project import"));
}

#[test]
#[cfg(unix)]
fn review_staleness_detects_a_replaced_inode_without_reloading_text() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let rule = root.join(AGENTS);
    write(&rule, "same rule").unwrap();
    let scope = scope(root);
    let first = scope.load_with_home(Some(root));
    let replacement = root.join("replacement");
    write(&replacement, "same rule").unwrap();
    assert!(!first.watch.is_stale(&scope));
    fs::rename(replacement, &rule).unwrap();
    assert!(first.watch.is_stale(&scope));

    write(&rule, "@later.md").unwrap();
    let pending = scope.load_with_home(Some(root));
    assert!(!pending.watch.is_stale(&scope));
    write(root.join("unrelated.txt"), "not watched").unwrap();
    assert!(!pending.watch.is_stale(&scope));
    write(root.join("later.md"), "now exists").unwrap();
    assert!(pending.watch.is_stale(&scope));
    let loaded = scope.load_with_home(Some(root));
    assert!(rendered(&loaded).contains("now exists"));
    fs::remove_file(root.join("later.md")).unwrap();
    assert!(loaded.watch.is_stale(&scope));
    fs::remove_file(rule).unwrap();
    assert!(pending.watch.is_stale(&scope));
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
                for name in NAMES.iter().copied().chain(personal.to_str()) {
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
