use super::*;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Outcome { Allow, Ask, Refuse }
use Outcome::*;

fn outcome(r: PermissionCheckResult) -> Outcome {
    match r {
        PermissionCheckResult::Allowed => Allow,
        PermissionCheckResult::NeedsPrompt(_) => Ask,
        PermissionCheckResult::Blocked(_) => Refuse,
    }
}

fn bash(cmd: &str, cwd: &Path, grants: &SessionPermissions) -> Outcome {
    outcome(check_permission_at_turn("Bash", Some(cmd), None, cwd, grants, None, true, None,
        ReadOrigin::LocalDelegate, None))
}

fn grant(root: &str, mode: PermissionMode) -> SessionPermissions {
    let mut grants = SessionPermissions::default();
    grants.set_path_mode(root, mode);
    grants
}

fn file(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "fixture\n").unwrap();
    path
}

#[test]
fn read_policy_command_fixtures() {
    let home = dirs::home_dir().unwrap();
    let world = tempfile::tempdir_in(home).unwrap();
    let root = world.path();
    let cargo = root.join("cargo");
    let cwd = root.join("project");
    file(&cwd, "scripts/check.sh");
    for name in [
        "reqwest-0.12.28/src/async_impl/client.rs", "reqwest-0.12.28/src/proxy.rs",
        "hyper-util-0.1.21/src/client/proxy/matcher.rs", "hyper-util-0.1.21/src/client/proxy/mod.rs",
    ] {
        file(&cargo.join("registry/src/index"), name);
    }
    let scratch = tempfile::tempdir().unwrap();
    let tmp = file(scratch.path(), "build.log");
    let state = crate::paths::linggen_home();
    let sessions = tempfile::tempdir_in(state).unwrap();
    let own = sessions.path().join("own");
    let other = sessions.path().join("other");
    for id in ["9a0f4a5f", "e19e1fde", "d1edfcef", "b8c5684d"] {
        file(&own, &format!("tasks/sh-{id}.log"));
    }
    file(&other, "tasks/sh-6e48fc54.log");
    let grants = SessionPermissions::load(&own);
    let tests = [
        ("sed -n '1410,1434p' C/registry/src/*/reqwest-0.12.28/src/async_impl/client.rs", Allow),
        ("sed -n '280,325p;375,412p' C/registry/src/*/reqwest-0.12.28/src/proxy.rs", Allow),
        ("sed -n '90,155p;175,222p' C/registry/src/*/hyper-util-0.1.21/src/client/proxy/matcher.rs", Allow),
        ("grep -n 'matcher' C/registry/src/*/hyper-util-0.1.21/src/client/proxy/mod.rs", Allow),
        ("grep -nE 'pub fn.*auth|socks' C/registry/src/*/reqwest-0.12.28/src/proxy.rs | head -25", Allow),
        ("grep -nE 'pub (struct|fn)|ALL_PROXY|REQUEST_METHOD' C/registry/src/*/hyper-util-*/src/client/proxy/matcher.rs | head -70", Allow),
        ("tail -10 O/tasks/sh-9a0f4a5f.log", Allow),
        ("tail -10 T; grep '^not ok' T", Allow),
        ("sed -n '280,325p;375,412p' C/registry/src/*/reqwest-0.12.28/src/proxy.rs; sed -n '62,82p' scripts/check.sh; tail -8 O/tasks/sh-9a0f4a5f.log", Allow),
        ("sed -n '1410,1434p' C/registry/src/*/reqwest-0.12.28/src/async_impl/client.rs; grep -n 'matcher' C/registry/src/*/hyper-util-0.1.21/src/client/proxy/mod.rs", Allow),
        ("tail -12 X/tasks/sh-6e48fc54.log", Ask),
        ("for x in e19e1fde d1edfcef b8c5684d; do echo ===$x; tail -5 O/tasks/sh-$x.log; done", Allow),
        ("for x in e19e1fde d1edfcef b8c5684d; do echo ===$x; tail -5 X/tasks/sh-$x.log; done", Ask),
        ("kill -TERM 46284; sleep 12", Ask),
        ("tail -15 O/tasks/sh-9a0f4a5f.log; kill -TERM 46284", Ask),
        ("cat S/account.toml", Refuse),
        ("grep -r token S", Ask),
    ];
    let failures: Vec<_> = tests.into_iter().filter_map(|(cmd, want)| {
        let cmd = cmd.split(' ').map(|word| {
            for (marker, root) in [("C/", cargo.as_path()), ("O/", own.as_path()), ("X/", other.as_path()), ("S/", state.as_path())] {
                if let Some(suffix) = word.strip_prefix(marker) {
                    return root.join(suffix).display().to_string();
                }
            }
            match word {
                "S" => state.display().to_string(),
                "T" => tmp.display().to_string(),
                "T;" => format!("{};", tmp.display()),
                _ => word.to_owned(),
            }
        }).collect::<Vec<_>>().join(" ");
        let got = bash(&cmd, &cwd, &grants);
        (want != got).then(|| format!("{cmd}: expected {want:?}, got {got:?}"))
    }).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn read_policy_protected_places_require_a_specific_grant() {
    let home = dirs::home_dir().unwrap();
    let world = tempfile::tempdir_in(&home).unwrap();
    let root = world.path();
    file(root, ".ssh/config");
    file(root, ".SSH/config");
    let cwd = root.join("project");
    std::fs::create_dir(&cwd).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join(".ssh"), root.join("alias")).unwrap();
    let mut paths = vec![".ssh/config", "project/../.ssh/config", ".SSH/config"];
    #[cfg(unix)]
    paths.push("alias/config");
    let mut failures = Vec::new();
    for (broad, mode) in [("*", PermissionMode::Read), (home.to_str().unwrap(), PermissionMode::Read), ("*", PermissionMode::Admin)] {
        let grants = grant(broad, mode);
        for name in &paths {
            let path = root.join(name);
            for (tool, cmd) in [("Read", None), ("Bash", Some(format!("cat {}", path.display())))] {
                let got = outcome(check_permission(tool, cmd.as_deref(), path.to_str(), &cwd, &grants, None));
                if got != Ask { failures.push(format!("{broad} {mode} {tool} {name}: {got:?}")); }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn read_policy_account_file_requires_local_owner_or_refuses() {
    let root = crate::paths::linggen_home();
    let path = root.join("account.toml");
    let read = |grants: &SessionPermissions| outcome(check_permission_at_turn(
        "Read", None, path.to_str(), root, grants, None, false, None,
        if grants.interactive { ReadOrigin::LocalOwner } else { ReadOrigin::LocalDelegate }, None,
    ));
    let broad = grant("*", PermissionMode::Admin);
    let named = grant(path.to_str().unwrap(), PermissionMode::Read);
    let parent = grant(root.to_str().unwrap(), PermissionMode::Read);
    let delegate = |grants: SessionPermissions| SessionPermissions {
        interactive: false,
        ask_caller: true,
        ..grants
    };
    assert_eq!(
        [read(&broad), read(&named), read(&parent), read(&delegate(broad)), read(&delegate(named))],
        [Ask, Allow, Allow, Refuse, Refuse]
    );
}

fn fill_large_tree(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    for n in 0..20_000 {
        std::fs::write(root.join(format!("entry-{n:05}")), "x").unwrap();
    }
}

#[test]
fn read_policy_recursive_search_on_large_trees() {
    let world = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
    let cwd = world.path().join("project");
    let outside = world.path().join("outside");
    fill_large_tree(&cwd);
    fill_large_tree(&outside);
    let grants = grant(cwd.to_str().unwrap(), PermissionMode::Edit);
    for with_env in [false, true] {
        if with_env { file(&cwd, ".env"); }
        for cmd in ["rg foo", "grep -rn foo .", "find . -name x"] {
            assert_eq!(bash(cmd, &cwd, &grants), Allow, "{cmd}, with .env: {with_env}");
        }
    }
    assert_eq!(bash(&format!("grep -rn foo {}", outside.display()), &cwd, &grants), Ask);
    let private = world.path().join("private");
    file(&private, ".ssh/config");
    assert_eq!(bash(&format!("grep -rn foo {}", private.display()), &cwd, &grants), Ask);
}

#[test]
fn read_policy_recursive_shell_search_requires_a_bounded_scope() {
    let world = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
    let cwd = world.path();
    file(cwd, "source/public.txt");
    file(cwd, "source/.ssh/private.txt");
    let grants = grant("*", PermissionMode::Admin);
    for cmd in ["grep -rn needle source", "grep -r needle", "rg --hidden needle ."] {
        assert_ne!(bash(cmd, cwd, &grants), Allow, "{cmd}");
    }
}

#[test]
fn read_policy_skill_keeps_its_named_grant() {
    let sandbox = tempfile::tempdir_in(crate::paths::linggen_home()).unwrap();
    let folder = sandbox.path().join("skill-data");
    let path = file(&folder, "facts.txt");
    let grants = grant(folder.to_str().unwrap(), PermissionMode::Read);
    let got = outcome(check_permission("Read", None, path.to_str(), &folder, &grants, None));
    assert_eq!(got, Allow);
}

#[test]
fn read_policy_shell_globs_check_matches_in_a_large_prefix() {
    let world = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
    let cwd = world.path();
    let prefix = cwd.join("registry");
    fill_large_tree(&prefix);
    file(&prefix, "index/crate/file.rs");
    let grants = SessionPermissions::default();
    for cmd in ["sed -n '1,2p' registry/*/crate/file.rs", "grep -n foo registry/*/crate/file.rs"] {
        assert_eq!(bash(cmd, cwd, &grants), Allow, "{cmd}");
    }
    #[cfg(unix)]
    {
        file(cwd, ".ssh/crate/file.rs");
        std::os::unix::fs::symlink(cwd.join(".ssh"), prefix.join("linked")).unwrap();
        for cmd in ["sed -n '1,2p' registry/*/crate/file.rs", "grep -n foo registry/*/crate/file.rs"] {
            assert_eq!(bash(cmd, cwd, &grants), Ask, "{cmd}");
        }
    }
}

#[test]
fn read_policy_shell_globs_need_a_safe_fixed_prefix() {
    let world = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
    let cwd = world.path();
    file(cwd, "safe/index/file.rs");
    file(cwd, "mixed/index/file.rs");
    file(cwd, "mixed/.ssh/config");
    file(cwd, "private/.ssh/index/file.rs");
    let grants = SessionPermissions::default();
    let cases = [
        ("safe/*/file.rs", Allow),
        ("safe/inde?/file.rs", Allow),
        ("safe/inde[xy]/file.rs", Allow),
        ("mixed/*/file.rs", Allow),
        ("private/.ssh/*/file.rs", Ask),
        ("safe/*/../index/file.rs", Ask),
        ("missing/*/file.rs", Ask),
        ("safe/[.]*/file.rs", Ask),
    ];
    let mut failures = Vec::new();
    for (path, want) in cases {
        let cmd = format!("sed -n '1,2p' {path}");
        let got = bash(&cmd, cwd, &grants);
        if got != want {
            failures.push(format!("{cmd}: expected {want:?}, got {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn read_policy_mutations_keep_their_class_and_need_grants() {
    use super::model::BashClass::{Admin, Write};
    let cwd = Path::new("/read-policy-test/wd");
    let grants = grant("*", PermissionMode::Read);
    for (cmd, tier) in [
        ("grep x doc > output", Write), ("cat $(printf x)", Admin),
        ("cat $UNKNOWN", Admin), ("sed -i.bak 's/a/b/' doc", Write),
        ("find . -delete", Admin), ("find . -exec touch x +", Admin),
        ("git checkout main", Write), ("git branch branch-name", Write),
    ] {
        assert_eq!(super::classify::classify_bash_command(cmd), tier, "{cmd}");
        assert_eq!(bash(cmd, cwd, &grants), Ask, "{cmd}");
    }
}
