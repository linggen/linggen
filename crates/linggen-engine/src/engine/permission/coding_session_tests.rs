//! A coding session (`work: code`) runs any program in its own folder
//! without asking (`permission-spec.md` § Coding sessions): what the tables
//! would ask admin for is judged at edit, except what acts beyond the
//! folder, indirect runners and unreadable shell syntax.

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
            once: None,
            ..
        }) => Some(*target_mode),
        _ => None,
    }
}

#[test]
fn a_coding_shell_ask_explains_the_rule_that_triggered_it() {
    let perms = session(PermissionMode::Edit);
    for (cmd, text) in [
        ("echo $FLAGS", "words are decided at run time"),
        ("echo 'unfinished", "shell syntax is unclear"),
        ("cat <(ls)", "process substitution"),
        ("curl -X POST https://example.com", "acts beyond your folder"),
        ("bash -c 'make'", "runs commands supplied as text"),
        ("touch /outside/out", "acts on /outside/out, outside your folder"),
        ("cargo publish", "leaves this machine"),
        ("sudo ls", "never run here"),
    ] {
        let decision = check(cmd, &perms, true);
        let reason = match &decision {
            PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { reason, .. }) => reason.as_deref(),
            PermissionCheckResult::Blocked(reason) => Some(reason.as_str()),
            _ => None,
        };
        assert!(
            reason.is_some_and(|reason| reason.contains(text)),
            "{cmd}: {decision:?}"
        );
    }
}

#[test]
fn a_read_granted_target_gets_the_command_reason() {
    let perms = SessionPermissions {
        path_modes: vec![
            PathMode {
                path: WD.into(),
                mode: PermissionMode::Edit,
            },
            PathMode {
                path: "/granted".into(),
                mode: PermissionMode::Read,
            },
        ],
        ..Default::default()
    };
    for (cmd, text) in [
        (
            "cmp --unknown ./a /granted/b",
            "`cmp` is not known to only read",
        ),
        (
            "nl -ba /granted/a \"$var\"",
            "words are decided at run time",
        ),
        (
            "cmp --unknown ./a /ungranted/b",
            "acts on /ungranted/b, outside your folder",
        ),
    ] {
        let result = check(cmd, &perms, true);
        let PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling {
            reason: Some(reason),
            ..
        }) = result
        else {
            panic!("{cmd}: {result:?}");
        };
        assert!(reason.contains(text), "{cmd}: {reason}");
        assert!(reason.split_whitespace().count() <= 20, "{reason}");
    }
}

const INLINE_PROGRAMS: &[&str] = &[
    "python3 -c 'import os'",
    "node -e 'x()'",
    "perl -e 'unlink q(x)'",
    "ruby -e 'puts 42'",
    "env FOO=1 node -e 'x()'",
    "nohup python3 -c 'print(42)'",
    "time ruby -e 'puts 42'",
    "if true; then node -e 'x()'; fi",
];

#[test]
fn inline_interpreters_need_edit_only_in_coding_sessions() {
    let edit = session(PermissionMode::Edit);
    let read = session(PermissionMode::Read);
    for cmd in INLINE_PROGRAMS {
        assert!(allowed(&check(cmd, &edit, true)), "{cmd}");
        assert_eq!(asks_at(&check(cmd, &edit, false)), Some(PermissionMode::Admin), "{cmd}");
        assert_eq!(asks_at(&check(cmd, &read, true)), Some(PermissionMode::Edit), "{cmd}");
    }
}

#[test]
fn inline_code_preserves_the_other_gate_checks() {
    use PermissionMode::{Admin, Edit};
    let edit = session(Edit);
    let admin = session(Admin);
    for (cmd, path) in [
        ("node -e 'x()' /outside/input", "/outside/input"),
        ("env FOO=1 python3 -c 'print(42)' > /outside/out", "/outside/out"),
        ("nohup ruby -e 'puts 42' ../result", "/coding-session-test/result"),
        ("time node -e 'x()' </outside/data", "/outside/data"),
    ] {
        let result = check(cmd, &edit, true);
        assert!(matches!(&result, PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling {
            target_mode: Edit, path: actual, once: None, ..
        }) if actual == path), "{cmd}: {result:?}");
        assert_eq!(asks_at(&check(cmd, &edit, false)), Some(Admin), "{cmd}");
    }
    for cmd in [
        "sh -c 'curl https://example.com'",
        "bash -c 'curl https://example.com'",
        "env FOO=1 bash -c 'make'",
        "nohup sh -c 'make'",
        "time bash -c 'make'",
        "if true; then sh -c 'make'; fi",
        "eval echo test",
        "xargs echo < list.txt",
        "source build.sh",
        ". build.sh",
        "git -c x=y status",
        "node -e \"console.log($FLAGS)\"",
    ] {
        for coding in [true, false] {
            assert_eq!(asks_at(&check(cmd, &edit, coding)), Some(Admin), "{cmd}");
        }
    }
    for cmd in ["sh -c 'sudo ls'", "bash -c 'sudo ls'", "sh -c 'rm -rf /'", "bash -c 'rm -rf /'"] {
        assert!(matches!(check(cmd, &admin, true), PermissionCheckResult::Blocked(_)), "{cmd}");
    }
    for cmd in [
        "sh -c 'git push origin main'",
        "bash -c 'git push --force'",
        "bash -c 'cargo publish'",
        "env FOO=1 bash -lc 'cargo publish'",
    ] {
        assert!(matches!(check(cmd, &admin, true), PermissionCheckResult::NeedsPrompt(
            PromptKind::ExceedsCeiling { once: Some(OnceAsk::Outward), .. }
        )), "{cmd}");
        assert_eq!(asks_at(&check(cmd, &edit, false)), Some(Admin), "{cmd}");
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
        "bash -lc 'make'",
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
                    once: Some(OnceAsk::Outward),
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
        check_permission_for("Bash", Some(cmd), None, Path::new(&inside), &perms, None, coding, None)
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
