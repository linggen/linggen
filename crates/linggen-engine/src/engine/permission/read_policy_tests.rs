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
    outcome(check_permission_for("Bash", Some(cmd), None, cwd, grants, None, true, None))
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
    file(&own, "tasks/sh-9a0f4a5f.log");
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
        ("for x in e19e1fde d1edfcef b8c5684d; do echo ===$x; tail -5 O/tasks/sh-$x.log; done", Ask),
        ("kill -TERM 46284; sleep 12", Ask),
        ("tail -15 O/tasks/sh-9a0f4a5f.log; kill -TERM 46284", Ask),
        ("cat S/account.toml", Refuse),
        ("grep -r token S", Refuse),
    ];
    let failures: Vec<_> = tests.into_iter().filter_map(|(cmd, want)| {
        let cmd = cmd.replace("C/", &format!("{}/", cargo.display()))
            .replace("O/", &format!("{}/", own.display()))
            .replace("X/", &format!("{}/", other.display()))
            .replace("S/", &format!("{}/", state.display()))
            .replace(" S", &format!(" {}", state.display()))
            .replace(" T", &format!(" {}", tmp.display()));
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
fn read_policy_account_file_is_refused_with_a_named_grant() {
    let root = crate::paths::linggen_home();
    let path = root.join("account.toml");
    let grants = grant(path.to_str().unwrap(), PermissionMode::Admin);
    let got = outcome(check_permission("Read", None, path.to_str(), root, &grants, None));
    assert_eq!(got, Refuse);
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
