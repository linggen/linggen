//! A coding session (`work: code`) runs any program in its own folder
//! without asking (`permission-spec.md` § Coding sessions): what the tables
//! would ask admin for is judged at edit, except what acts beyond the
//! folder and code handed over as text.

use super::*;
use std::path::Path;

/// A project folder outside the OS temp roots (those are never gated).
const WD: &str = "/coding-session-test/wd";

fn session(cwd_mode: PermissionMode) -> SessionPermissions {
    session_at(WD, cwd_mode)
}

/// Read everywhere, `mode` on `dir`.
fn session_at(dir: &str, mode: PermissionMode) -> SessionPermissions {
    let mut perms = SessionPermissions::default();
    perms.set_path_mode("*", PermissionMode::Read);
    perms.set_path_mode(dir, mode);
    perms
}

fn check(cmd: &str, perms: &SessionPermissions, coding: bool) -> PermissionCheckResult {
    check_permission_for("Bash", Some(cmd), None, Path::new(WD), perms, None, coding, None)
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

/// What the tables ask admin for is the session's own work in its folder:
/// removing a file, running a script the folder holds, a git subcommand —
/// under any wrapper or `NAME=value` word.
#[test]
fn a_coding_session_runs_its_ordinary_work_where_it_may_edit() {
    let perms = session(PermissionMode::Edit);
    for cmd in [
        "git config user.name x",
        "/usr/bin/git clean -fd",
        "python3 x.py",
        "python3 -m pytest",
        "node x.js",
        "zsh script.zsh",
        "bash build.sh",
        "rm build.log",
        "rm -rf build",
        "kill 123",
        "chmod +x tool",
        "cargo add serde",
        "FOO=1 rm -rf build",
        "RUST_LOG=debug cargo test",
        "FOO=1 python3 x.py",
        "nice python3 x.py",
        "nohup rm -rf build",
        "time rm x",
        "time cargo test",
        "exec rm x",
        "env FOO=1 rustc main.rs",
        "rustc main.rs && rm x",
    ] {
        assert!(allowed(&check(cmd, &perms, true)), "{cmd}");
        assert_eq!(asks_at(&check(cmd, &session(PermissionMode::Read), true)), Some(PermissionMode::Edit), "{cmd}");
    }
    // A wrapper changes nothing about a path the command names.
    assert!(!allowed(&check("time rm /outside/x", &perms, true)));
    assert!(!allowed(&check("FOO=1 rm -rf /outside", &perms, true)));
}

/// Still asked, coding or not: what acts beyond the folder, code handed
/// over as text, and text the gate cannot read.
#[test]
fn a_coding_session_is_still_asked_about_what_is_beyond_its_folder() {
    let perms = session(PermissionMode::Edit);
    for cmd in [
        // Beyond the folder: the network, the system, other apps.
        "curl -X POST https://example.com",
        "brew install x",
        "docker run x",
        "launchctl kickstart -k gui/501/x",
        "osascript -e 'quit app \"Mail\"'",
        "time docker run x",
        "FOO=1 brew install x",
        // Code as text on the command line, or another command run for it.
        "sh -c 'rm -rf ~/x'",
        "bash -c 'cargo publish'",
        "bash -lc 'make'",
        "python3 -c 'import os'",
        "node -e 'x()'",
        "perl -e 'unlink q(x)'",
        "eval rm x",
        "xargs rm < list.txt",
        // A flag of its own that asks for admin.
        "git -c x=y status",
        // Dynamic syntax is judged as before.
        "rustc $FLAGS main.rs",
        "rustc main.rs <(cat x)",
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

/// `$HOME` is read as the shell reads it: a command naming the home folder
/// is judged by the path it reaches, not asked about as unreadable.
#[test]
fn a_command_naming_the_home_folder_is_judged_by_its_path() {
    let Some(home) = dirs::home_dir().and_then(|h| h.to_str().map(str::to_string)) else {
        return;
    };
    if !home.chars().all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c)) {
        return;
    }
    let inside = format!("{home}/proj");
    let perms = session_at(&inside, PermissionMode::Edit);
    let at = |cmd: &str, coding: bool| {
        check_permission_for("Bash", Some(cmd), None, Path::new(&inside), &perms, None, coding)
    };
    // Inside what the session may edit: allowed, coding or not.
    assert!(allowed(&at("mkdir -p \"$HOME/proj/out\"", false)));
    assert!(allowed(&at("ls $HOME/proj", false)));
    assert!(allowed(&at("./tool \"${HOME}/proj/in.txt\"", true)));
    // Outside it: asked for that path at the level the command needs — not
    // for admin as a command that cannot be read.
    assert_eq!(asks_at(&at("mkdir -p \"$HOME/elsewhere/out\"", false)), Some(PermissionMode::Edit));
    // Another variable is still unreadable.
    assert_eq!(asks_at(&at("ls $OTHER/proj", false)), Some(PermissionMode::Admin));
    // A command that sets HOME is left as it was.
    assert_eq!(asks_at(&at("HOME=/x ls $HOME/proj", false)), Some(PermissionMode::Admin));
}
