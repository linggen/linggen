//! Ordinary shapes the gate reads as the shell does (`permission-spec.md`
//! § Bash path gating): `git grep`'s pattern is text, `$?` is a number, and
//! a `for` loop over words written in the command is its body once per word.
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

// ---------------------------------------------------------------------------
// `for NAME in w1 w2 …; do BODY; done`: the body once for each word
// ---------------------------------------------------------------------------

/// Run unasked in a coding session: each pass is an ordinary command.
const LOOPS_READ: &[&str] = &[
    "for term in A B C; do echo \"$term\"; git grep -wo \"$term\" | sort | uniq -c; done",
    "for f in a.rs b.rs; do wc -l $f; done",
    "for name in a b; do cat \"${name}.txt\"; done",
    "for d in src ui\ndo\n  ls $d\ndone",
    "for n in 1 2 3; do echo $n; done; echo end",
    "ls; for t in a b; do git grep -n \"$t\" -- src; done",
    "for p in foo bar; do grep -c \"$p\" README.md; echo $?; done",
    "for t in t1 t2; do scripts/scratch-test.sh --lib $t; done",
    "for x in a; do echo '$x'; done",
    "for a in 1; do echo $a; done; for b in 2; do echo $b; done",
    "for v in -n -c; do grep $v foo README.md; done",
];

/// Blocked as the same commands written out are.
const LOOPS_BLOCKED: &[&str] = &[
    "for p in / ~; do rm -rf $p; done",
    "for p in ~; do rm -rf $p; done",
    "for c in sudo; do $c ls; done",
    "for o in -rf; do rm $o /; done",
];

/// Asked once, as an outward command written out is.
const LOOPS_OUTWARD: &[&str] = &[
    "for c in publish; do cargo $c; done",
    "for r in origin; do git push $r main; done",
];

/// Still asked: the loop is not of the shape read (and stays unreadable),
/// or a pass names a path beyond the session's folder.
const LOOPS_STILL_ASKED: &[&str] = &[
    "for x in $(ls); do cat $x; done",
    "for x in *.rs; do cat $x; done",
    "for x in a; do cat $y; done",
    "for x in a; do x=/etc; cat $x; done",
    "for x in a; do read x; cat $x; done",
    "for x in a; do export x; echo $x; done",
    "for x in a; do for y in b; do echo $x$y; done; done",
    "while true; do echo $x; done",
    "until false; do echo $x; done",
    "for x; do echo $x; done",
    "for x in \"a b\"; do rm $x; done",
    "for x in a\\ b; do rm $x; done",
    "for x in {a,b}; do echo $x; done",
    "for x in a; do echo ${x:-/etc}; done",
    "for PATH in /tmp; do ls $PATH; done",
    "for x in /etc; do cat $x/passwd; done",
    "for x in ../..; do cat $x/etc/passwd; done",
    "for x in a; do echo $x; done | sh",
    "for x in a; do echo $x; done > /etc/x",
    "cd /tmp && for x in a; do cat $x; done",
    "if true; then for x in a; do echo $x; done; fi",
    "for x in a; do echo $x # ; done",
    "for x in a; do $x=1 ls; done",
    "for x in a=b; do $x ls; done",
    "for x in a; do echo `$x`; done",
    "for x in a; do sh -c \"rm $x\"; done",
    "for x in ~alex; do cat $x/.ssh/id_rsa; done",
    "for p in ~; do cat \"$p\"/.ssh/id_rsa; done",
    "for x in a; do echo $x; done; echo $x",
    "for x in a; do echo $x &\ndone",
    "for x in a; do cat <<EOF\n$x\nEOF\ndone",
    "IFS=/; for x in a/b; do ls $x; done",
];

#[test]
fn a_loop_over_written_words_is_its_body_once_per_word() {
    for cmd in LOOPS_READ {
        assert!(allowed(&check(cmd, true)), "{cmd}: {:?}", check(cmd, true));
    }
}

#[test]
fn each_pass_is_judged_as_the_command_written_out() {
    for cmd in LOOPS_BLOCKED {
        for coding in [false, true] {
            let r = check(cmd, coding);
            assert!(
                matches!(r, PermissionCheckResult::Blocked(_)),
                "{cmd}: {r:?}"
            );
        }
    }
    for cmd in LOOPS_OUTWARD {
        let r = check(cmd, true);
        let outward = matches!(
            r,
            PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { outward: true, .. })
        );
        assert!(outward, "{cmd}: {r:?}");
    }
}

#[test]
fn a_loop_of_any_other_shape_is_still_asked() {
    let still = LOOPS_BLOCKED.len() + LOOPS_OUTWARD.len() + LOOPS_STILL_ASKED.len();
    assert!(still >= LOOPS_READ.len());
    for cmd in LOOPS_STILL_ASKED {
        assert!(!allowed(&check(cmd, true)), "{cmd}");
    }
    let many: Vec<String> = (0..=super::for_loop::MAX_VALUES)
        .map(|i| i.to_string())
        .collect();
    let cmd = format!("for n in {}; do echo $n; done", many.join(" "));
    assert!(!allowed(&check(&cmd, true)), "{} values", many.len());
}

#[test]
fn a_loop_is_written_out_as_the_shell_runs_it() {
    let cases = [
        (
            "for t in A B; do echo \"$t\"; done",
            "echo \"A\"\necho \"B\"",
        ),
        (
            "ls; for n in 1 2; do echo $n; done; echo end",
            "ls; echo 1\necho 2; echo end",
        ),
        (
            "for x in a; do echo '$x' \\$x ${x}; done",
            "echo '$x' \\$x a",
        ),
        ("for p in / ~; do rm -rf $p; done", "rm -rf /\nrm -rf ~"),
    ];
    for (cmd, want) in cases {
        assert_eq!(
            super::for_loop::unrolled(cmd).as_deref(),
            Some(want),
            "{cmd}"
        );
    }
    // Piped on, the loop's output is one stream: not written out.
    assert_eq!(
        super::for_loop::unrolled("for x in a; do cat $x; done | sh"),
        None
    );
}
