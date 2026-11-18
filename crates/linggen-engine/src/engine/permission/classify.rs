//! Bash command classification: the tier a command needs.
//!
//! Best effort, not a shell parser or filesystem sandbox. Words are judged
//! as the program receives them (quotes and backslashes removed). Unknown
//! programs, flags and dynamic shell syntax go up a tier, never down.

use super::model::{
    command_run, command_tokens, file_name, is_odd_space, BashClass, Joint, Token,
};
use super::scripts::{awk_class, sed_class};
use super::shell_text::strip_heredoc_bodies;
use BashClass::{Admin, Read, Write};

use super::model::BashReason;

/// The tier a command needs outside a coding session (the tables' answer;
/// the tests read it).
#[cfg(test)]
pub fn classify_bash_command(cmd: &str) -> BashClass {
    classify_bash(cmd, false).0
}

/// The tier a command needs, in a coding session or not: in one, a segment
/// is judged by [`coding_class`] first. Also whether that lowered any
/// segment (such a run is still judged where it runs).
pub(super) fn classify_bash(cmd: &str, coding: bool) -> (BashClass, bool, Option<BashReason>) {
    let cmd = &strip_heredoc_bodies(cmd);
    if cmd.trim().is_empty() {
        return (Admin, false, None);
    }
    if let Some(reason) = dynamic_syntax(cmd) {
        return (Admin, false, Some(reason));
    }
    let mut highest = Read;
    let mut reason = None;
    let mut lowered = false;
    let mut words = Vec::new();
    for token in command_tokens(cmd)
        .into_iter()
        .chain(std::iter::once(Token::Op(Joint::Seq)))
    {
        match token {
            Token::Word(w) => words.push(w),
            Token::Op(_) if !words.is_empty() => {
                let (class, by_code, why) = classify_segment(&words, coding);
                if class > highest || (class == highest && reason.is_none()) {
                    reason = why;
                }
                highest = highest.max(class);
                lowered |= by_code;
                words.clear();
            }
            Token::Op(_) => {}
        }
    }
    (highest, lowered, reason)
}

/// Backticks and `$` outside single quotes change the command or its
/// targets at run time; an unclosed quote or trailing backslash is unclear.
/// So is what the words here would be read differently from the shell:
/// a heredoc whose body was not dropped as data (the shell does not split
/// a body into words; an apostrophe in it is no quote), a comment holding
/// a quote character or a backslash (neither means anything to the shell
/// there), and whitespace the shell does not split words on.
fn dynamic_syntax(cmd: &str) -> Option<BashReason> {
    let mut quote = None;
    let mut escaped = false;
    let mut word_start = true;
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        if escaped {
            escaped = false;
            word_start = false;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if (c == '`' || c == '$') && quote != Some('\'') {
            return Some(BashReason::Expansion);
        }
        if quote.is_none() {
            if is_odd_space(c) {
                return Some(BashReason::UnclearSyntax);
            }
            if c == '#' && word_start && comment_is_unclear(&mut chars) {
                return Some(BashReason::UnclearSyntax);
            }
            if c == '<' && chars.next_if_eq(&'<').is_some() && chars.next_if_eq(&'<').is_none() {
                return Some(BashReason::UnclearSyntax);
            }
        }
        if c == '\'' || c == '"' {
            if quote == Some(c) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(c);
            }
        }
        word_start = quote.is_none() && (c.is_whitespace() || ";|&()<>".contains(c));
    }
    (quote.is_some() || escaped).then_some(BashReason::UnclearSyntax)
}

/// Reads a comment to the end of its line; whether it holds a quote
/// character or a backslash.
fn comment_is_unclear(chars: &mut std::iter::Peekable<std::str::Chars>) -> bool {
    let mut found = false;
    while let Some(c) = chars.next_if(|c| *c != '\n') {
        found |= matches!(c, '\'' | '"' | '\\');
    }
    found
}

/// A word as the program receives it: quotes and backslash escapes removed.
pub(super) fn dequote(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut quote = None;
    let mut chars = word.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('"'), '\\') => match chars.peek() {
                Some(&n) if matches!(n, '$' | '`' | '"' | '\\' | '\n') => {
                    out.push(n);
                    chars.next();
                }
                _ => out.push('\\'),
            },
            (Some(_), _) => out.push(c),
            (None, '\\') => match chars.next() {
                Some('\n') | None => {}
                Some(n) => out.push(n),
            },
            (None, '\'' | '"') => quote = Some(c),
            (None, _) => out.push(c),
        }
    }
    out
}

/// `<(…)` and `>(…)` run a command of their own, unseen here.
fn has_process_substitution(words: &[String]) -> bool {
    words.iter().any(|word| {
        let mut quote = None;
        let mut escaped = false;
        let mut prev = None;
        for c in word.chars() {
            if escaped {
                escaped = false;
            } else if c == '\\' && quote != Some('\'') {
                escaped = true;
            } else if c == '\'' || c == '"' {
                quote = match quote {
                    Some(q) if q == c => None,
                    None => Some(c),
                    q => q,
                };
            } else if c == '(' && quote.is_none() && matches!(prev, Some('<' | '>')) {
                return true;
            }
            prev = Some(c);
        }
        false
    })
}

fn classify_segment(raw: &[String], coding: bool) -> (BashClass, bool, Option<BashReason>) {
    if has_process_substitution(raw) {
        return (Admin, false, Some(BashReason::ProcessSubstitution));
    }
    let (words, redirect) = split_redirects(raw);
    let base = classify_words(&words);
    let (class, by_code, reason) = if coding && base == Admin {
        match coding_class(&words) {
            Ok(Some(class)) => (class, true, None),
            Ok(None) => (base, false, None),
            Err(reason) => (base, false, Some(reason)),
        }
    } else {
        (base, false, None)
    };
    // A literal line selection is an inspection, including in a pipeline.
    // Leave other sed forms and redirects to the existing tables.
    let class = if class == Write && reads_sed_lines(raw) {
        Read
    } else {
        class
    };
    (if redirect { class.max(Write) } else { class }, by_code, reason)
}

/// A literal `sed -n` with numeric print selections and plain file operands.
pub(super) fn reads_sed_lines(raw: &[String]) -> bool {
    let [program, flag, script, files @ ..] = raw else {
        return false;
    };
    if program != "sed" || flag != "-n" {
        return false;
    }
    let script = script
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .or_else(|| script.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
        .unwrap_or(script);
    script.split(';').all(numeric_print_selection)
        && files.iter().all(|word| {
            !dequote(word).starts_with('-')
                && !word.contains(['<', '>', '*', '?', '[', '{', '}', '(', ')'])
        })
}

fn numeric_print_selection(selection: &str) -> bool {
    let Some(address) = selection.strip_suffix('p') else {
        return false;
    };
    let number = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    address
        .split_once(',')
        .map_or_else(|| number(address), |(a, b)| number(a) && number(b))
}

/// How a coding session judges a segment: what the tables would ask admin
/// for is a write where the session may edit. Such a session already runs
/// any code its folder holds (`cargo run`, a test), so the program is not
/// what the gate holds back there — the paths a command names are (the
/// target check), and what leaves the machine is (`outward`).
///
/// The program is the one really run: past `NAME=value` words and the
/// programs that only run what follows (`time cargo test`,
/// `RUST_LOG=debug cargo test`), as the deny floor finds it. `None` leaves
/// the segment to the tables: they already allow it, the program acts
/// beyond the folder ([`BEYOND_THE_FOLDER`]), it is handed code as text
/// ([`runs_given_text`]), or a flag of its own asks for admin.
fn coding_class(words: &[String]) -> Result<Option<BashClass>, BashReason> {
    let all: Vec<&str> = words.iter().map(String::as_str).collect();
    let run = command_run(&all);
    let Some(first) = run.first() else {
        // Only the shell's own words (`fi`, `done`, a loop's list): no
        // program runs. Anything else with none stays with the tables.
        return Ok(super::model::only_shell_words(&all).then_some(Read));
    };
    let program = file_name(first);
    let program = program.as_str();
    let args: Vec<String> = run[1..].iter().map(|w| w.to_string()).collect();
    if BEYOND_THE_FOLDER.contains(&program) {
        return Err(BashReason::BeyondFolder(program.to_string()));
    }
    if runs_given_text(program, &args) {
        return Err(BashReason::GivenText(program.to_string()));
    }
    // The command past its wrappers, judged by the tables under its plain
    // name: `time cargo test` is `cargo test`.
    let mut plain = vec![program.to_string()];
    plain.extend(args.iter().cloned());
    match classify_words(&plain) {
        Admin => Ok(Some(Write.max(arg_lift(program, &args)))),
        class => Ok(Some(class)),
    }
}

/// Programs a coding session is still asked about: what they do is beyond
/// the session's folder and is not a path the target check can see —
/// another user's rights, sending to the network, the system's packages,
/// containers and services, other apps. Like the outward table, a tripwire
/// for the plain forms, not a sandbox.
const BEYOND_THE_FOLDER: &[&str] = &[
    "sudo", "su", "doas",
    "curl", "wget", "nc", "ncat", "netcat", "telnet", "ftp",
    "brew", "port", "apt", "apt-get", "dnf", "yum", "pacman",
    "docker", "podman",
    "launchctl", "systemctl", "defaults", "osascript", "diskutil", "shutdown", "reboot", "halt",
];

/// Shells whose flags can take a separate command line.
const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh", "fish", "csh", "tcsh"];

/// Literal command operands of shells remain subject to the floor and
/// outward table, even though the outer shell still needs admin.
pub(super) fn literal_shell_commands(cmd: &str) -> Vec<String> {
    let stripped = strip_heredoc_bodies(cmd);
    let tokens = command_tokens(&stripped);
    tokens
        .split(|token| matches!(token, Token::Op(_)))
        .filter_map(literal_shell_command)
        .map(str::to_string)
        .collect()
}

fn literal_shell_command(segment: &[Token]) -> Option<&str> {
    let raw: Vec<_> = segment.iter().filter_map(|token| match token {
        Token::Word(word) => Some(word.as_str()),
        _ => None,
    }).collect();
    let plain: Vec<_> = raw.iter().map(|word| dequote(word)).collect();
    let words: Vec<_> = plain.iter().map(String::as_str).collect();
    let run = command_run(&words);
    let [program, option, _, ..] = run else { return None };
    if !SHELLS.contains(&file_name(program).as_str()) || !matches!(*option, "-c" | "-lc" | "-cl") {
        return None;
    }
    let operand = raw[words.len() - run.len() + 2];
    let text = operand.strip_prefix('\'')?.strip_suffix('\'')?;
    (!text.contains('\'')).then_some(text)
}

/// Shell command text and indirect runners stay with the tables: the gate
/// judges the outer command line, not a second shell command line inside it.
/// Other interpreters' inline code is judged like their script files.
fn runs_given_text(program: &str, args: &[String]) -> bool {
    if matches!(program, "eval" | "source" | "." | "xargs") {
        return true;
    }
    // `git -c name=value …` sets configuration for one run, and some of it
    // is a command git runs (`core.sshCommand`, an alias).
    if program == "git" {
        return args.iter().take_while(|a| a.starts_with('-')).any(|a| a == "-c");
    }
    if !SHELLS.contains(&program) {
        return false;
    }
    args.iter()
        .take_while(|a| a.starts_with('-'))
        .any(|a| match a.strip_prefix("--") {
            Some(long) => matches!(long, "eval" | "print" | "command" | "exec"),
            None => a.chars().any(|c| "ceErp".contains(c)),
        })
}

/// The command's own words (dequoted), and whether output is redirected to
/// a file. `/dev/null` and fd duplication (`2>&1`) write nothing.
fn split_redirects(raw: &[String]) -> (Vec<String>, bool) {
    let harmless = |t: &str| matches!(t, "/dev/null" | "&1" | "&2");
    let mut words = Vec::new();
    let mut redirect = false;
    let mut awaiting_target = false;
    for word in raw {
        if awaiting_target {
            redirect |= !harmless(&dequote(word));
            awaiting_target = false;
            continue;
        }
        let Some(i) = unquoted_redirect_at(word) else {
            words.push(dequote(word));
            continue;
        };
        let before = &word[..i];
        if !before.is_empty() && !before.chars().all(|c| c.is_ascii_digit() || c == '&') {
            words.push(dequote(before));
        }
        let target = dequote(word[i..].trim_start_matches(['>', '|']));
        if target.is_empty() {
            awaiting_target = true;
        } else {
            redirect |= !harmless(&target);
        }
    }
    (words, redirect || awaiting_target)
}

fn unquoted_redirect_at(word: &str) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    for (i, c) in word.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' && quote != Some('\'') {
            escaped = true;
        } else if c == '\'' || c == '"' {
            if quote == Some(c) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(c);
            }
        } else if c == '>' && quote.is_none() {
            return Some(i);
        }
    }
    None
}

fn classify_words(words: &[String]) -> BashClass {
    let Some(program) = words.first().map(String::as_str) else {
        return Admin;
    };
    let args = &words[1..];
    if program == "env" {
        return classify_env(args);
    }
    // `cd <dir>` only moves the shell; the commands after it are judged
    // at <dir> by the target check (model.rs follows `cd`).
    if program == "cd" {
        return Read;
    }
    // `command -v <name>` only asks where a program is: nothing is run.
    if program == "command" && matches!(args.first().map(String::as_str), Some("-v" | "-V")) {
        return Read;
    }
    // `git -C <dir> <sub> …` is `<sub>` run in <dir>: judge the rest.
    // <dir> stays an argument, so the target check sees it.
    if program == "git" && args.first().map(String::as_str) == Some("-C") && args.len() > 2 {
        let mut rest = vec![words[0].clone()];
        rest.extend_from_slice(&args[2..]);
        return classify_words(&rest);
    }
    let sub = args.first().map(String::as_str).unwrap_or("");
    let base = match (program, sub) {
        ("git", "branch" | "tag" | "remote" | "stash") => git_ref_class(sub, &args[1..]),
        ("docker" | "podman", "system") => {
            if matches!(
                args.get(1).map(String::as_str),
                Some("df" | "events" | "info")
            ) {
                Read
            } else {
                Admin
            }
        }
        _ => classify_program(program, args),
    };
    base.max(arg_lift(program, args))
}

fn git_ref_class(sub: &str, args: &[String]) -> BashClass {
    let first = args.first().map(String::as_str);
    let read = match sub {
        "branch" | "tag" => args.iter().all(|a| {
            matches!(
                a.as_str(),
                "-a" | "-r" | "-v" | "-vv" | "-l" | "--list" | "--all" | "--verbose"
            )
        }),
        "remote" => {
            args.is_empty()
                || (args.len() == 1 && first == Some("-v"))
                || matches!(first, Some("show" | "get-url"))
        }
        _ => matches!(first, Some("list" | "show")),
    };
    if read {
        Read
    } else {
        Write
    }
}

// ---------------------------------------------------------------------------
// env
// ---------------------------------------------------------------------------

enum EnvRun<'a> {
    /// `env` alone only prints.
    Prints,
    /// `env [flags] [NAME=value…] command…` runs that command.
    Runs(&'a [String]),
    /// A flag that changes how or where the command runs (`-S`, `-C`, `-P`, unknown).
    Opaque,
}

fn classify_env(args: &[String]) -> BashClass {
    match env_command(args) {
        EnvRun::Prints => Read,
        EnvRun::Runs(command) => classify_words(command),
        EnvRun::Opaque => Admin,
    }
}

fn env_command(args: &[String]) -> EnvRun<'_> {
    const PLAIN: &[&str] = &[
        "-",
        "-i",
        "--ignore-environment",
        "-0",
        "--null",
        "-v",
        "--debug",
        "--",
    ];
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        let a = arg.as_str();
        if PLAIN.contains(&a) || a.starts_with("--unset=") || (a.starts_with("-u") && a.len() > 2) {
            i += 1;
        } else if a == "-u" || a == "--unset" {
            i += 2;
        } else if a.starts_with('-') {
            return EnvRun::Opaque;
        } else if a.contains('=') {
            i += 1;
        } else {
            return EnvRun::Runs(&args[i..]);
        }
    }
    EnvRun::Prints
}

// ---------------------------------------------------------------------------
// Arguments that lift a program above its base tier
// ---------------------------------------------------------------------------

/// Arguments that make a program write or execute. `subs` empty = any
/// subcommand. A two-character `-x` also matches inside a short-flag
/// cluster (`-sSo`); a `--long` flag also matches `--long=v` and the
/// abbreviations GNU tools accept; anything else matches exactly.
struct Lift {
    programs: &'static [&'static str],
    subs: &'static [&'static str],
    args: &'static [&'static str],
    class: BashClass,
}

const LIFTS: &[Lift] = &[
    Lift {
        programs: &["find"],
        subs: &[],
        args: &["-delete", "-exec", "-execdir", "-ok", "-okdir"],
        class: Admin,
    },
    Lift {
        programs: &["find"],
        subs: &[],
        args: &["-fprint", "-fprint0", "-fprintf", "-fls"],
        class: Write,
    },
    Lift {
        programs: &["fd"],
        subs: &[],
        args: &["-x", "-X", "--exec", "--exec-batch"],
        class: Admin,
    },
    Lift {
        programs: &["rg"],
        subs: &[],
        args: &["--pre"],
        class: Admin,
    },
    Lift {
        programs: &["bat", "ack", "ag"],
        subs: &[],
        args: &["--pager"],
        class: Admin,
    },
    Lift {
        programs: &["git"],
        subs: &["diff", "log", "show"],
        args: &["--output"],
        class: Write,
    },
    Lift {
        // Opens the matches in a program of the caller's choosing.
        programs: &["git"],
        subs: &["grep"],
        args: &["-O", "--open-files-in-pager"],
        class: Admin,
    },
    Lift {
        programs: &["sort"],
        subs: &[],
        args: &["-o", "--output"],
        class: Write,
    },
    Lift {
        programs: &["sort"],
        subs: &[],
        args: &["--compress-program"],
        class: Admin,
    },
    Lift {
        programs: &["tree"],
        subs: &[],
        args: &["-o"],
        class: Write,
    },
    Lift {
        programs: &["yq"],
        subs: &[],
        args: &["-i", "--inplace"],
        class: Write,
    },
    Lift {
        programs: &["go"],
        subs: &["env"],
        args: &["-w", "-u"],
        class: Write,
    },
    Lift {
        programs: &["go"],
        subs: &["vet"],
        args: &["-vettool", "--vettool"],
        class: Admin,
    },
    Lift {
        programs: &["npm", "yarn", "pnpm"],
        subs: &["audit"],
        args: &["fix", "--fix"],
        class: Write,
    },
    Lift {
        programs: &["cargo"],
        subs: &["clippy"],
        args: &["--fix"],
        class: Write,
    },
];

/// Programs whose arguments carry a script or file operands judged on their own.
const ARG_CHECKS: &[(&str, fn(&[String]) -> BashClass)] =
    &[("sed", sed_class), ("awk", awk_class), ("uniq", uniq_class)];

fn arg_lift(program: &str, args: &[String]) -> BashClass {
    let sub = args.first().map(String::as_str).unwrap_or("");
    let lifted = LIFTS
        .iter()
        .filter(|l| l.programs.contains(&program) && (l.subs.is_empty() || l.subs.contains(&sub)))
        .filter(|l| {
            args.iter()
                .any(|a| l.args.iter().any(|f| arg_matches(a, f)))
        })
        .map(|l| l.class);
    let checked = ARG_CHECKS
        .iter()
        .filter(|(p, _)| *p == program)
        .map(|(_, check)| check(args));
    lifted.chain(checked).max().unwrap_or(Read)
}

fn arg_matches(arg: &str, flag: &str) -> bool {
    if flag.starts_with("--") {
        let name = arg.split('=').next().unwrap_or(arg);
        return name.len() > 2 && name.starts_with("--") && flag.starts_with(name);
    }
    if let [b'-', c] = flag.as_bytes() {
        return arg.starts_with('-') && !arg.starts_with("--") && arg[1..].contains(*c as char);
    }
    arg == flag || arg.split('=').next() == Some(flag)
}

/// `uniq [options] [input [output]]` — a second operand is a file it writes.
fn uniq_class(args: &[String]) -> BashClass {
    let mut operands = 0;
    let mut i = 0;
    while let Some(a) = args.get(i) {
        let takes_value = matches!(
            a.as_str(),
            "-f" | "-s" | "-w" | "--skip-fields" | "--skip-chars" | "--check-chars"
        );
        if takes_value {
            i += 2;
            continue;
        }
        if a == "-" || !a.starts_with('-') {
            operands += 1;
        }
        i += 1;
    }
    if operands >= 2 {
        Write
    } else {
        Read
    }
}

/// `curl` is read only for a HEAD request with flags known to write nothing.
fn curl_class(args: &[String]) -> BashClass {
    const SAFE_LONG: &[&str] = &[
        "--head",
        "--silent",
        "--show-error",
        "--location",
        "--insecure",
        "--verbose",
        "--include",
        "--fail",
        "--max-time",
        "--connect-timeout",
        "--user-agent",
        "--header",
    ];
    let is_head = args.iter().any(|a| {
        a == "--head"
            || (!a.starts_with("--")
                && a.strip_prefix('-').is_some_and(|c| {
                    c.chars()
                        .take_while(|c| !"mHA".contains(*c))
                        .any(|c| c == 'I')
                }))
    });
    let safe = args.iter().filter(|a| a.starts_with('-')).all(|a| {
        short_flags_safe(a, "sSLkvIif", "mHA")
            || SAFE_LONG.contains(&a.split('=').next().unwrap_or(""))
    });
    if is_head && safe {
        Read
    } else {
        Admin
    }
}

/// `wget --spider` checks a link without saving it.
fn wget_class(args: &[String]) -> BashClass {
    const SAFE_LONG: &[&str] = &[
        "--spider",
        "--quiet",
        "--server-response",
        "--verbose",
        "--no-verbose",
        "--timeout",
        "--tries",
        "--no-check-certificate",
    ];
    let spider = args.iter().any(|a| a == "--spider");
    let safe = args.iter().filter(|a| a.starts_with('-')).all(|a| {
        a == "-nv"
            || short_flags_safe(a, "qSv", "Tt")
            || SAFE_LONG.contains(&a.split('=').next().unwrap_or(""))
    });
    if spider && safe {
        Read
    } else {
        Admin
    }
}

/// A short-flag cluster made only of `plain` flags, optionally ending in
/// one of `valued` with its value glued on.
fn short_flags_safe(arg: &str, plain: &str, valued: &str) -> bool {
    let Some(cluster) = arg.strip_prefix('-') else {
        return false;
    };
    if cluster.is_empty() || cluster.starts_with('-') {
        return false;
    }
    for c in cluster.chars() {
        if valued.contains(c) {
            return true;
        }
        if !plain.contains(c) {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Program tables
// ---------------------------------------------------------------------------

fn classify_program(program: &str, args: &[String]) -> BashClass {
    known_program(program, args).unwrap_or(Admin)
}

/// The tables' class for `program`; `None` for a program they do not know.
fn known_program(program: &str, args: &[String]) -> Option<BashClass> {
    const READ_PROGRAMS: &[&str] = &[
        "ls",
        "cat",
        "head",
        "tail",
        "less",
        "more",
        "wc",
        "file",
        "stat",
        "du",
        "df",
        "pwd",
        "printenv",
        "echo",
        "printf",
        "which",
        "whereis",
        "type",
        "find",
        "grep",
        "rg",
        "ag",
        "ack",
        "fd",
        "tree",
        "bat",
        "jq",
        "yq",
        "uname",
        "hostname",
        "date",
        "id",
        "whoami",
        "realpath",
        "dirname",
        "basename",
        "ping",
        "dig",
        "nslookup",
        "host",
        "test",
        "true",
        "false",
        "seq",
        // Only waits: `sleep 30; tail -25 <task log>` watches a shell.
        "sleep",
        "sort",
        "uniq",
        "tr",
        "cut",
        "paste",
        "diff",
        "comm",
        // Read-only system diagnostics (used by apple-shifu and similar inspection
        // skills). These programs only display info — they don't mutate state.
        "uptime",
        "sw_vers",
        "vm_stat",
        "netstat",
        "ifconfig",
        "ipconfig",
        "scutil",
        "ps",
        "lsof",
        "vmmap",
        "iostat",
        // macOS system / security inspection. Bare invocations are read-only;
        // mutating forms (sysctl -w, csrutil disable, spctl --add, fdesetup
        // enable, etc.) all require sudo, which the hardcoded deny floor blocks.
        "sysctl",
        "spctl",
        "csrutil",
        "fdesetup",
        "socketfilterfw",
        "systemsetup",
    ];

    // branch/tag/remote/stash are judged by `git_ref_class`.
    const GIT_READ: &[&str] = &[
        "status",
        "log",
        "diff",
        "show",
        "rev-parse",
        "blame",
        "describe",
        "shortlog",
        "ls-files",
        "ls-tree",
        "grep",
    ];

    const CARGO_READ: &[&str] = &[
        "check",
        "clippy",
        "doc",
        "metadata",
        "tree",
        "verify-project",
    ];
    const NPM_READ: &[&str] = &[
        "list", "ls", "outdated", "view", "info", "audit", "why", "explain",
    ];
    const PIP_READ: &[&str] = &["list", "show", "freeze", "check"];
    const GO_READ: &[&str] = &["vet", "list", "doc", "env", "version"];

    const ADMIN_PROGRAMS: &[&str] = &[
        "rm",
        "sudo",
        "su",
        "kill",
        "killall",
        "pkill",
        "chmod",
        "chown",
        "chgrp",
        "systemctl",
        "launchctl",
        "service",
        "mount",
        "umount",
        "mkfs",
        "fdisk",
        "dd",
        "apt",
        "apt-get",
        "yum",
        "dnf",
        "pacman",
        "reboot",
        "shutdown",
        "halt",
        "poweroff",
        "iptables",
        "ufw",
        "firewall-cmd",
        "crontab",
        "at",
    ];

    const WRITE_PROGRAMS: &[&str] = &[
        "mkdir", "cp", "mv", "touch", "sed", "awk", "patch", "ln", "install", "rsync", "tee",
        // Runs a package, as `npm exec` does.
        "npx",
    ];

    const GIT_WRITE: &[&str] = &[
        "add",
        "commit",
        "push",
        "pull",
        "merge",
        "rebase",
        "checkout",
        "switch",
        "fetch",
        "clone",
        "init",
        "reset",
        "cherry-pick",
        "am",
        "apply",
    ];

    const CARGO_WRITE: &[&str] = &["build", "test", "run", "fmt", "install", "publish", "bench"];
    const NPM_WRITE: &[&str] = &[
        "install", "ci", "run", "start", "test", "build", "publish", "exec",
    ];
    const PIP_WRITE: &[&str] = &["install", "uninstall"];
    const GO_WRITE: &[&str] = &["build", "test", "run", "install", "get", "mod"];

    // brew and docker have read subcommands worth carving out so inspection
    // skills (apple-shifu) don't have to admin-prompt for `brew list` /
    // `docker images`.
    const BREW_READ: &[&str] = &[
        "list",
        "info",
        "search",
        "outdated",
        "deps",
        "leaves",
        "doctor",
        "config",
        "--version",
        "-v",
        "--prefix",
        "tap-info",
    ];
    // `system` is judged before this table.
    const DOCKER_READ: &[&str] = &[
        "ps", "images", "logs", "inspect", "version", "info", "history", "port", "top", "stats",
        "diff", "events",
    ];

    if ADMIN_PROGRAMS.contains(&program) {
        return Some(Admin);
    }
    if READ_PROGRAMS.contains(&program) {
        return Some(Read);
    }
    if WRITE_PROGRAMS.contains(&program) {
        return Some(Write);
    }

    let sub = args.first().map(String::as_str).unwrap_or("");
    let by_sub = |read: &[&str], write: &[&str]| {
        if read.contains(&sub) {
            Read
        } else if write.contains(&sub) {
            Write
        } else {
            Admin
        }
    };
    Some(match program {
        "git" => by_sub(GIT_READ, GIT_WRITE),
        "cargo" => by_sub(CARGO_READ, CARGO_WRITE),
        "npm" | "yarn" | "pnpm" => by_sub(NPM_READ, NPM_WRITE),
        "pip" | "pip3" => by_sub(PIP_READ, PIP_WRITE),
        "go" => by_sub(GO_READ, GO_WRITE),
        "brew" => by_sub(BREW_READ, &[]),
        "docker" | "podman" => by_sub(DOCKER_READ, &[]),
        "python" | "python3" | "node" => by_sub(&["--version", "--help", "-V"], &[]),
        "curl" => curl_class(args),
        "wget" => wget_class(args),
        "make" | "cmake" | "ninja" | "mvn" | "gradle" | "pytest" | "jest" | "vitest" => Write,
        _ => return None,
    })
}
