//! A coding session (`work: code`) runs any program in its own folder
//! without asking (`permission-spec.md` § Coding sessions): a program the
//! tables do not know is judged at edit, not admin — and nothing else moves.

use super::*;
use std::path::Path;

/// A project folder outside the OS temp roots (those are never gated).
const WD: &str = "/coding-session-test/wd";

fn session(cwd_mode: PermissionMode) -> SessionPermissions {
    let mut perms = SessionPermissions::default();
    perms.set_path_mode("*", PermissionMode::Read);
    perms.set_path_mode(WD, cwd_mode);
    perms
}

fn check(cmd: &str, perms: &SessionPermissions, coding: bool) -> PermissionCheckResult {
    check_permission_for("Bash", Some(cmd), None, Path::new(WD), perms, None, coding)
}

fn allowed(result: &PermissionCheckResult) -> bool {
    matches!(result, PermissionCheckResult::Allowed)
}

fn asks_at(result: &PermissionCheckResult) -> Option<PermissionMode> {
    match result {
        PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling {
            target_mode,
            outward: false,
            ..
        }) => Some(*target_mode),
        _ => None,
    }
}

const UNKNOWN_PROGRAMS: &[&str] = &[
    "rustc main.rs",
    "/opt/tools/bin/flutter test",
    "./tool",
    "./tool --flag x",
    "flutter test",
    "dart run",
    "swift build",
    "cd /coding-session-test/wd && flutter build ios",
    "flutter test > out.log 2>&1",
    "rustc main.rs && ./main",
];

#[test]
fn a_coding_session_runs_an_unknown_program_where_it_may_edit() {
    let perms = session(PermissionMode::Edit);
    for cmd in UNKNOWN_PROGRAMS {
        assert!(allowed(&check(cmd, &perms, true)), "{cmd}");
    }
}

#[test]
fn with_coding_off_the_same_program_asks_for_admin() {
    let perms = session(PermissionMode::Edit);
    for cmd in UNKNOWN_PROGRAMS {
        assert_eq!(
            asks_at(&check(cmd, &perms, false)),
            Some(PermissionMode::Admin),
            "{cmd}"
        );
    }
}

#[test]
fn a_coding_session_still_needs_edit_where_the_program_runs() {
    let perms = session(PermissionMode::Read);
    for cmd in [
        "rustc main.rs",
        // Its own file is read, and the run is still judged where it runs.
        "/opt/tools/bin/flutter test",
        "./tool",
        // A scratch path is free; the run is not.
        "flutter create /tmp/x",
        "./tool > /tmp/out",
    ] {
        assert_eq!(
            asks_at(&check(cmd, &perms, true)),
            Some(PermissionMode::Edit),
            "{cmd}"
        );
    }
}

#[test]
fn every_other_path_the_command_names_is_still_judged() {
    let perms = session(PermissionMode::Edit);
    let result = check("rustc main.rs -o /outside/x", &perms, true);
    let PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling {
        target_mode, path, ..
    }) = result
    else {
        panic!("{result:?}");
    };
    assert_eq!(
        (target_mode, path.as_str()),
        (PermissionMode::Edit, "/outside/x")
    );
}

#[test]
fn a_known_program_keeps_its_rule_in_a_coding_session() {
    let perms = session(PermissionMode::Edit);
    for cmd in [
        "git -c x=y status",
        "/usr/bin/git status",
        "git config user.name x",
        "python3 x.py",
        "node x.js",
        "rm build.log",
        "curl -X POST https://example.com",
        "kill 123",
        "chmod +x tool",
        "brew install x",
        "docker run x",
    ] {
        assert_eq!(
            asks_at(&check(cmd, &perms, true)),
            Some(PermissionMode::Admin),
            "{cmd}"
        );
    }
    assert!(allowed(&check("git status", &perms, true)));
}

#[test]
fn only_a_program_really_run_is_lowered() {
    let perms = session(PermissionMode::Edit);
    for cmd in [
        // A word before the program, a wrapper, or a shell given the
        // command as text: the program run is not the first word.
        "FOO=1 rm -rf build",
        "FOO=1 python3 x.py",
        "nice python3 x.py",
        "nohup rm -rf build",
        "time rm x",
        "exec rm x",
        "env FOO=1 rustc main.rs",
        "sh -c 'rm -rf ~/x'",
        "bash -c 'rm -rf ~/x'",
        "zsh script.zsh",
        "eval rm x",
        "xargs rm < list.txt",
        "perl -e 'unlink q(x)'",
        // Dynamic syntax and a second segment are judged as before.
        "rustc $FLAGS main.rs",
        "rustc main.rs <(cat x)",
        "rustc main.rs && rm x",
    ] {
        assert_eq!(
            asks_at(&check(cmd, &perms, true)),
            Some(PermissionMode::Admin),
            "{cmd}"
        );
    }
}

#[test]
fn the_deny_floor_and_outward_commands_come_first() {
    let perms = session(PermissionMode::Admin);
    for cmd in [
        "rustc main.rs && sudo rm -rf /x",
        "/opt/tools/bin/flutter test; sudo ls",
        "rm -rf / && ./tool",
    ] {
        assert!(
            matches!(check(cmd, &perms, true), PermissionCheckResult::Blocked(_)),
            "{cmd}"
        );
    }
    for cmd in ["./tool && cargo publish", "flutter pub publish", "ssh host"] {
        assert!(
            matches!(
                check(cmd, &perms, true),
                PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling {
                    outward: true,
                    ..
                })
            ),
            "{cmd}"
        );
    }
}
