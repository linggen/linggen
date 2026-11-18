//! Ordinary shapes the gate reads as the shell does (`permission-spec.md`
//! § Bash path gating): `git grep`'s pattern is text, and `$?` is a number.
//! Each step has a table
//! of what it newly allows and a table, at least as long, of what must still
//! be judged as before.

use super::model::command_targets;
use super::*;
use std::path::Path;

/// A project folder outside the OS temp roots (those are never gated).
const WD: &str = "/readable-shapes-test/wd";

fn targets(cmd: &str) -> Vec<String> {
    command_targets(cmd, Path::new("/w"))
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// Edit on the project folder and nothing else.
fn folder_session() -> SessionPermissions {
    let mut perms = SessionPermissions::default();
    perms.set_path_mode(WD, PermissionMode::Edit);
    perms
}

fn check(cmd: &str, coding: bool) -> PermissionCheckResult {
    check_permission_for(
        "Bash",
        Some(cmd),
        None,
        Path::new(WD),
        &folder_session(),
        None,
        coding,
        None,
    )
}

fn allowed(result: &PermissionCheckResult) -> bool {
    matches!(result, PermissionCheckResult::Allowed)
}

// ---------------------------------------------------------------------------
// `git grep`: the pattern is text
// ---------------------------------------------------------------------------

/// Each command and the only targets it has: its pattern is not one.
const GIT_GREP_TEXT: &[(&str, &[&str])] = &[
    ("git grep -n '/api/config' -- ui/src", &[]),
    ("git grep '/api/config'", &[]),
    ("git grep -e '/a/' -e '/b/'", &[]),
    ("git grep -A 3 '/sessions/' -- src", &[]),
    ("git grep -wo '/x/' HEAD -- src", &[]),
    ("git grep --max-depth 2 '/x/'", &[]),
    ("git grep -- '/x/' src", &[]),
    ("git grep -i -C2 '/x/'", &[]),
    ("git grep -m 1 -n \"/api/\" -- '*.rs'", &[]),
    ("git -C /w/repo grep -n '/api/x'", &["/w/repo"]),
    ("git grep --threads 4 '/x/' ../other", &["/w/../other"]),
];

/// Each keeps `/etc/x` as a target: it is a path or a pathspec, or a flag
/// form not understood keeps every word as before.
const GIT_GREP_STILL_JUDGED: &[&str] = &[
    "git grep foo /etc/x",
    "git grep foo -- /etc/x",
    "git grep -e foo /etc/x",
    "git grep -f /etc/x",
    "git grep -fp /etc/x",
    "git grep -e/a /etc/x",
    "git grep -efoo /etc/x",
    "git grep /etc/x -e .",
    "git grep /etc/x -f p",
    "git grep -A /etc/x foo",
    "git grep -O foo /etc/x",
    "git grep -Oe /etc/x",
    "git grep --open-files-in-pager foo /etc/x",
    "git grep --color /etc/x",
    "git grep --thr 4 /etc/x foo",
    "git grep --no-max-count /etc/x",
    "git grep -- -- /etc/x",
    "git grep -e a --and -e b /etc/x",
    "git grep '(' /etc/x",
    "git grep foo HEAD /etc/x",
    "git -C /tmp grep foo /etc/x",
    "git -C /w grep -C 3 foo /etc/x",
    "git --no-pager grep /etc/x",
    "git -c core.pager=less grep /etc/x",
    "git log /etc/x",
];

#[test]
fn git_grep_patterns_are_text_not_targets() {
    for (cmd, want) in GIT_GREP_TEXT {
        assert_eq!(targets(cmd), *want, "{cmd}");
    }
}

#[test]
fn git_grep_paths_and_unread_flags_are_still_judged() {
    assert!(GIT_GREP_STILL_JUDGED.len() >= GIT_GREP_TEXT.len());
    let missing: Vec<String> = GIT_GREP_STILL_JUDGED
        .iter()
        .filter(|c| !targets(c).iter().any(|t| t == "/etc/x"))
        .map(|c| format!("{c:?}: {:?}", targets(c)))
        .collect();
    assert!(missing.is_empty(), "{missing:#?}");
}

#[test]
fn a_git_grep_in_the_folder_runs_unasked() {
    for coding in [false, true] {
        let cmd = "git grep -n '/api/config' -- ui/src";
        assert!(allowed(&check(cmd, coding)), "{cmd} coding={coding}");
        let cmd = "git grep -n '/api/config' /etc/x";
        assert!(!allowed(&check(cmd, coding)), "{cmd} coding={coding}");
    }
}

// ---------------------------------------------------------------------------
// `$?`: a number
// ---------------------------------------------------------------------------

/// Run unasked in a coding session: `$?` is read as a number.
const STATUS_READ: &[&str] = &[
    "scripts/scratch-test.sh --lib some_test; echo EXIT=$?",
    "echo $?",
    "echo \"$?\"",
    "echo ${?}",
    "cargo test; echo \"exit: $?\"",
    "cargo build 2>&1 | tail -3; echo \"status $? of 1\"",
    "test $? -eq 0 && echo ok",
    "false || echo \"failed with $?\"",
    "echo rc=$? \"${?}\"",
];

/// Left as they are (a `$` in them keeps them unreadable): `$?` glued to
/// other text, in another form, where a program is named, or another
/// variable beside it.
const STATUS_UNREAD: &[&str] = &[
    "echo $?x",
    "echo $??",
    "echo ${?:-1}",
    "echo ${?}x",
    "echo \"$?\"x",
    "echo $?$?",
    "echo -$?",
    "echo --rc=$?",
    "ls /$?",
    "cat /etc/$?",
    "ls ~$?",
    "ls ./$?",
    "echo $?>f",
    "echo $?/etc",
    "echo ($?)",
    "$?",
    "X=1 $?",
    "env $? ls",
    "echo $#",
    "echo $$",
    "echo $!",
    "echo $0",
    "echo $@",
];

#[test]
fn the_exit_status_is_read_as_a_number() {
    for cmd in STATUS_READ {
        assert!(allowed(&check(cmd, true)), "{cmd}: {:?}", check(cmd, true));
    }
}

#[test]
fn the_exit_status_glued_or_beside_another_variable_is_still_unread() {
    assert!(STATUS_UNREAD.len() >= STATUS_READ.len());
    for cmd in STATUS_UNREAD {
        assert_eq!(super::known_vars::with_status(cmd), None, "{cmd}");
        assert!(!allowed(&check(cmd, true)), "{cmd}");
    }
    // Another variable beside it keeps the command unreadable.
    assert!(!allowed(&check("echo $? $x", true)));
    // Single quotes and a backslash keep the dollar a dollar.
    assert_eq!(super::known_vars::with_status("echo '$?'"), None);
    assert_eq!(super::known_vars::with_status("echo \\$?"), None);
}
