//! Outward commands: a command whose effect is off this machine — it
//! publishes, operates a remote machine, or rewrites a shared branch — asks
//! whatever the session's grants (`permission-spec.md` § Outward commands).
//!
//! Like the deny floor, a tripwire for the plain forms, not a sandbox: a
//! script or a build tool can still do the same thing. The program is found
//! the way the floor finds it ([`command_run`]).

use super::classify::dequote;
use super::model::{command_run, command_tokens, file_name, Token};
use super::shell_text::strip_heredoc_bodies;

/// What makes a use of a program outward.
enum Rule {
    /// Every use: the program only works on another machine.
    Always,
    /// The subcommand path (`release create`), its first word the first
    /// subcommand word.
    Sub(&'static [&'static str]),
    /// One of several subcommand paths.
    AnySub(&'static [&'static [&'static str]]),
    /// An operand on another host (`host:path`, `rsync://`).
    RemoteOperand,
    /// A push that names main/master, forces, deletes, mirrors or pushes all.
    Push,
}

/// The one table of outward commands. What belongs here: a command whose
/// effect is off this machine and cannot be undone from here. Read-only
/// uses of the cloud command lines (`kubectl get`, `aws … describe-*`) are
/// too many to tell apart: those programs ask on every use.
const OUTWARD: &[(&[&str], Rule)] = &[
    (
        &["cargo", "npm", "pnpm", "yarn", "bun"],
        Rule::Sub(&["publish"]),
    ),
    (
        &["gh"],
        Rule::AnySub(&[
            &["release", "create"],
            &["release", "edit"],
            &["release", "upload"],
            &["release", "delete"],
            &["pr", "merge"],
        ]),
    ),
    (&["flutter", "dart"], Rule::Sub(&["pub", "publish"])),
    (&["docker", "podman", "gem"], Rule::Sub(&["push"])),
    (&["twine"], Rule::Sub(&["upload"])),
    (&["pod"], Rule::Sub(&["trunk", "push"])),
    (&["ssh", "scp", "sftp"], Rule::Always),
    (&["rsync"], Rule::RemoteOperand),
    (
        &[
            "aws",
            "gcloud",
            "az",
            "kubectl",
            "helm",
            "terraform",
            "pulumi",
            "wrangler",
            "vercel",
            "netlify",
            "fly",
            "flyctl",
            "heroku",
            "firebase",
        ],
        Rule::Always,
    ),
    (&["git"], Rule::Push),
];

/// Whether any command segment of `cmd` sends something off this machine.
pub(super) fn is_outward(cmd: &str) -> bool {
    let cmd = strip_heredoc_bodies(cmd);
    let mut words: Vec<String> = Vec::new();
    for token in command_tokens(&cmd)
        .into_iter()
        .chain(std::iter::once(Token::Op(super::model::Joint::Seq)))
    {
        match token {
            Token::Word(w) => words.push(dequote(&w)),
            Token::Op(_) => {
                if segment_is_outward(&words) {
                    return true;
                }
                words.clear();
            }
        }
    }
    false
}

fn segment_is_outward(words: &[String]) -> bool {
    let all: Vec<&str> = words.iter().map(String::as_str).collect();
    // The program really run: past the shell's own words (`then`, `(`) too.
    let run = command_run(&all);
    let Some(program) = run.first().map(|w| file_name(w)) else {
        return false;
    };
    let args = &run[1..];
    OUTWARD
        .iter()
        .find(|(names, _)| names.contains(&program.as_str()))
        .is_some_and(|(_, rule)| rule_holds(rule, args))
}

fn rule_holds(rule: &Rule, args: &[&str]) -> bool {
    match rule {
        Rule::Always => true,
        Rule::Sub(path) => names_sub(args, path).is_some(),
        Rule::AnySub(paths) => paths.iter().any(|path| names_sub(args, path).is_some()),
        Rule::RemoteOperand => args.iter().any(|a| is_remote_operand(a)),
        Rule::Push => names_sub(args, &["push"]).is_some_and(|at| push_is_outward(&args[at + 1..])),
    }
}

fn is_option(word: &str) -> bool {
    word.starts_with('-') || word.starts_with('+')
}

/// Where `path` starts when the program's arguments run it: its first word
/// is the first non-option word, or the second when the first may be the
/// value of an option before it (`gh -R o/r pr merge`, `git -C d push`);
/// its later words follow in order. The index of its last word.
fn names_sub(args: &[&str], path: &[&str]) -> Option<usize> {
    let words: Vec<(usize, &str)> = args
        .iter()
        .enumerate()
        .filter(|(_, w)| !is_option(w))
        .map(|(i, w)| (i, *w))
        .collect();
    let first = words.first()?.0;
    let may_be_value = first > 0 && !args[first - 1].contains('=');
    let starts = if may_be_value { 2 } else { 1 };
    (0..starts.min(words.len())).find_map(|start| follows(&words[start..], path))
}

/// `path` as a run of `words` from the first: its first word there, the
/// rest in order after it. The index of the last.
fn follows(words: &[(usize, &str)], path: &[&str]) -> Option<usize> {
    let (head, rest) = path.split_first()?;
    let (at, word) = words.first()?;
    if word != head {
        return None;
    }
    let mut last = *at;
    let mut later = words[1..].iter();
    for want in rest {
        last = later.find(|(_, w)| w == want)?.0;
    }
    Some(last)
}

/// `host:path`, `user@host:path` or `rsync://…` — rsync's own rule: a
/// colon before any slash.
fn is_remote_operand(word: &str) -> bool {
    if word.starts_with('-') {
        return false;
    }
    if word.starts_with("rsync://") {
        return true;
    }
    match (word.find(':'), word.find('/')) {
        (Some(colon), Some(slash)) => colon < slash,
        (Some(_), None) => true,
        _ => false,
    }
}

/// The words after `push`: whether it forces, deletes, mirrors, pushes
/// every branch, or names main/master as a destination.
fn push_is_outward(args: &[&str]) -> bool {
    args.iter()
        .any(|word| push_flag_is_outward(word) || refspec_is_outward(word))
}

fn push_flag_is_outward(word: &str) -> bool {
    if let Some(long) = word.strip_prefix("--") {
        let name = long.split('=').next().unwrap_or(long);
        return matches!(
            name,
            "force"
                | "force-with-lease"
                | "force-if-includes"
                | "delete"
                | "mirror"
                | "all"
                | "prune"
        );
    }
    // A cluster of short flags: `-f`, `-d`, `-uf`.
    word.strip_prefix('-')
        .is_some_and(|short| short.chars().any(|c| c == 'f' || c == 'd'))
}

/// A refspec (or the remote's word) that forces (`+x`), deletes (`:x`) or
/// lands on main/master (`main`, `HEAD:main`, `x:refs/heads/master`).
fn refspec_is_outward(word: &str) -> bool {
    if word.starts_with('-') {
        return false;
    }
    if word.starts_with('+') || word.starts_with(':') {
        return true;
    }
    let destination = word.rsplit(':').next().unwrap_or(word);
    let branch = destination
        .strip_prefix("refs/heads/")
        .unwrap_or(destination);
    matches!(branch, "main" | "master")
}
