//! Permission model: types, path matching, tier mapping,
//! and the `check_permission` decision function.
//!
//! All pure logic — no I/O, no UI prompts. The store layer lives in
//! [`super::store`]; prompt construction in [`super::prompt`].

use super::classify::{classify_bash, dequote, literal_shell_commands};
use super::pattern_text::text_words;
use super::shell_text::strip_heredoc_bodies;
use super::store::SessionPermissions;
use crate::util::expand_tilde;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Permission action returned after prompting the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionAction {
    AllowOnce,
    AllowSession,
    AllowDirectory {
        all: bool,
        mode: PermissionMode,
        /// Directory the user picked. `"*"` when `all` is set.
        path: String,
    },
    Deny,
    /// User denied with a custom message to relay to the model.
    DenyWithMessage(String),
}

/// Session permission mode — defines the ceiling of what the agent can do.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
#[cfg_attr(any(test, feature = "test-support"), derive(ts_rs::TS), ts(export))]
pub enum PermissionMode {
    Chat,
    #[default]
    Read,
    Edit,
    Admin,
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PermissionMode::Chat => write!(f, "chat"),
            PermissionMode::Read => write!(f, "read"),
            PermissionMode::Edit => write!(f, "edit"),
            PermissionMode::Admin => write!(f, "admin"),
        }
    }
}

/// A path-scoped mode grant. The mode covers the path and all its children.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathMode {
    pub path: String,
    pub mode: PermissionMode,
}

/// Bash command classification for permission tier mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BashClass {
    Read,
    Write,
    Admin,
}

#[derive(Debug)]
pub enum PermissionCheckResult {
    Allowed,
    Blocked(String),
    NeedsPrompt(PromptKind),
}

/// What kind of prompt to show. With the simplified model there is only one
/// kind — every permission-needed event is a request to upgrade the mode on
/// the target path (or a parent path for files outside cwd).
#[derive(Debug)]
pub enum PromptKind {
    ExceedsCeiling {
        target_mode: PermissionMode,
        /// The file or directory the action touches, for the card.
        path: String,
        /// Grant choices, broadest first: the repo root down to the file's directory.
        directories: Vec<String>,
        tool_summary: String,
        /// The gate's explanation for a coding-session Bash request.
        reason: Option<String>,
        /// The command's effect leaves this machine (`outward`): one answer
        /// covers this one command — allow once or deny, no grant offered.
        outward: bool,
    },
}

/// Facts supplied by the branch that made the Bash decision.
#[derive(Clone)]
pub(super) enum BashReason {
    Expansion,
    UnclearSyntax,
    ProcessSubstitution,
    BeyondFolder(String),
    GivenText(String),
    Target,
    Level,
    Outward,
    Floor,
}

impl BashReason {
    fn sentence(&self, mode: PermissionMode, path: &str) -> String {
        match self {
            Self::Expansion => "Its words are decided at run time (`$` or backticks): write the values out or use a script in your folder.".into(),
            Self::UnclearSyntax => "Its shell syntax is unclear to the gate: use a simpler command or a script in your folder.".into(),
            Self::ProcessSubstitution => "It uses process substitution: pass a file in your folder instead.".into(),
            Self::BeyondFolder(program) => format!("{program} acts beyond your folder: this form needs {mode}."),
            Self::GivenText(program) => format!("{program} runs commands supplied as text: this form needs {mode}."),
            Self::Target => {
                let verb = if mode == PermissionMode::Read { "reads" } else { "acts on" };
                format!("It {verb} {path}, outside your folder: use an equivalent path already granted at {mode}.")
            }
            Self::Level => format!("This command needs {mode} at {path}; the session grant does not cover it."),
            Self::Outward => "It leaves this machine: this is always asked.".into(),
            Self::Floor => "This command is never run here: the safety floor blocks it.".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Hardcoded deny floor — engine-baked, not user-configurable.
//
// Curated list of patterns that are always blocked regardless of mode. Admin
// mode does NOT bypass this. See `doc/permission-spec.md` §"Hardcoded deny
// floor".
// ---------------------------------------------------------------------------

/// True if the given bash command matches the hardcoded deny floor.
pub fn is_hardcoded_deny(cmd: &str) -> bool {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Forkbomb pattern :(){:|:&};: — also tolerates whitespace variants
    // (`:() { :|:& };:`).
    let dewhitespaced: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    if dewhitespaced.contains(":(){:|:&}") {
        return true;
    }

    for segment in split_command_segments(trimmed) {
        let seg = segment.trim();
        if seg.is_empty() {
            continue;
        }
        if segment_is_hardcoded_deny(seg) {
            return true;
        }
    }
    // And each command as the shell splits them: by line and by `&` too,
    // with a quoted span kept whole.
    let stripped = super::shell_text::strip_heredoc_bodies(trimmed);
    command_segments(before_a_heredoc_body(&stripped))
        .iter()
        .any(|words| words_are_hardcoded_deny(words))
}

/// `cmd` up to the end of its first line that opens a heredoc: the lines
/// after it are a body some program reads (a script for `python3`, a
/// message for `git commit`), not commands — a line of text is never
/// blocked for starting with a word of the floor.
fn before_a_heredoc_body(cmd: &str) -> &str {
    let Some(at) = cmd.find("<<") else {
        return cmd;
    };
    match cmd[at..].find('\n') {
        Some(end) => &cmd[..at + end],
        None => cmd,
    }
}

/// The words of each command in `cmd`, as [`command_tokens`] splits them
/// and as the program receives them (quotes removed).
fn command_segments(cmd: &str) -> Vec<Vec<String>> {
    let mut segments = vec![Vec::new()];
    for token in command_tokens(cmd) {
        match token {
            Token::Word(w) => segments.last_mut().expect("one segment").push(dequote(&w)),
            Token::Op(_) => segments.push(Vec::new()),
        }
    }
    segments.retain(|words| !words.is_empty());
    segments
}

/// Programs that only run the command that follows them.
pub(super) const RUNS_WHAT_FOLLOWS: &[&str] =
    &["env", "command", "exec", "nohup", "nice", "time", "builtin"];

/// `NAME=value`, as a shell reads a word before the command.
pub(super) fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Shell words that open, join or close a compound command: what follows
/// one is a command like any other (`if sudo ls`, `then sudo ls`,
/// `{ sudo ls`, `! sudo ls`).
const SHELL_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "while", "until", "do", "done", "esac", "!", "{", "}",
    "coproc",
];

/// Heads of a compound command whose own words name no program: a loop's
/// list, a `case` word. Their bodies are other segments.
const LISTS_ONLY: &[&str] = &["for", "select"];

/// The command a segment really runs, for the floor: past leading
/// `NAME=value` words, past the shell's own words around a compound command
/// (`if`, `then`, `{`, a subshell's `(`, a `case` arm's label, a function's
/// name) and past the programs that only run what follows them (with their
/// options), its program named by its file name whatever path or letter
/// case it was given by. `/usr/bin/sudo ls`, `FOO=1 sudo ls`,
/// `env -i sudo ls`, `nice -n 19 sudo ls`, `then sudo ls` and `(sudo ls`
/// are all `sudo ls`. Empty when the segment names no program.
pub(super) fn command_run<'a>(tokens: &'a [&'a str]) -> &'a [&'a str] {
    run_of(tokens, true)
}

/// [`command_run`], reading past the shell's own words or not. Not, for
/// words that were split without regard to quotes: there a `then` may be a
/// word of a sentence.
fn run_of<'a>(tokens: &'a [&'a str], shell_words: bool) -> &'a [&'a str] {
    let mut rest = tokens;
    loop {
        while rest.first().is_some_and(|w| is_assignment(w)) {
            rest = &rest[1..];
        }
        if let Some(past) = past_shell_words(rest).filter(|_| shell_words) {
            rest = past;
            continue;
        }
        let Some(first) = rest.first() else {
            return rest;
        };
        let wrapper = file_name(first);
        if !RUNS_WHAT_FOLLOWS.contains(&wrapper.as_str()) {
            return rest;
        }
        rest = &rest[1..];
        // `command -v x` only asks where x is: nothing is run.
        if wrapper == "command" && matches!(rest.first(), Some(&"-v" | &"-V")) {
            return &[];
        }
        // The wrapper's own options: flags, a flag's name or number
        // (`env -u HOME`, `nice -n 19`), and `env`'s assignments.
        let mut takes_value = false;
        while let Some(word) = rest.first() {
            let flag = word.starts_with('-');
            let value = takes_value && !flag;
            if !(flag || value || is_assignment(word)) {
                break;
            }
            takes_value = flag && matches!(*word, "-u" | "-n" | "-C" | "-P" | "-S" | "-a");
            rest = &rest[1..];
        }
    }
}

/// `rest` past the shell's own words it starts with, when it starts with
/// any: a keyword, a bare `(` or `)`, a function's name (`f()`, `f ()`,
/// `function f`), a `case` head up to its first arm, an arm's label
/// (`x)` before a command). A list head (`for x in …`) leaves nothing.
fn past_shell_words<'a>(rest: &'a [&'a str]) -> Option<&'a [&'a str]> {
    let first = *rest.first()?;
    // A shell word is the word itself: `./for` and `bin/if` are programs.
    let word = first.trim_start_matches('(').trim_end_matches(')');
    if LISTS_ONLY.contains(&word) {
        return Some(past_the_do(rest));
    }
    match word {
        "function" => return Some(rest.get(2..).unwrap_or(&[])),
        "case" => return Some(past_first_label(&rest[1..])),
        // `coproc NAME { cmd`: the name is not the program.
        "coproc" if rest.get(2).is_some_and(|w| w.starts_with(['{', '('])) => {
            return Some(&rest[2..]);
        }
        _ => {}
    }
    let only_parens = first.chars().all(|c| matches!(c, '(' | ')'));
    if only_parens || SHELL_KEYWORDS.contains(&word) || first.ends_with("()") {
        return Some(&rest[1..]);
    }
    let next = rest.get(1).copied();
    if next == Some("()") {
        return Some(&rest[2..]);
    }
    // `x) cmd`: an arm's label. `(cmd) >out` is a subshell and its redirect.
    let redirect = |w: &str| {
        w.trim_start_matches(|c: char| c.is_ascii_digit() || c == '&')
            .starts_with(['>', '<'])
    };
    let label = first.ends_with(')') && next.is_some_and(|w| !redirect(w));
    label.then(|| &rest[1..])
}

/// A loop head's words past its `do`, when the `do` stands right after the
/// loop's name (`for x do cmd` loops over the shell's own arguments, with
/// its body on the same line); none when the head lists its words — a `do`
/// among them is one of the words.
fn past_the_do<'a>(head: &'a [&'a str]) -> &'a [&'a str] {
    match head.get(2) {
        Some(&"do") => &head[3..],
        _ => &[],
    }
}

/// Whether a segment that names no program is made of the shell's own
/// words (past its `NAME=value` words): `fi`, `done`, `for x in a b`.
pub(super) fn only_shell_words(tokens: &[&str]) -> bool {
    let at = tokens.iter().position(|w| !is_assignment(w));
    at.is_some_and(|at| past_shell_words(&tokens[at..]).is_some()) && command_run(tokens).is_empty()
}

/// The words after a `case` head's first arm label; none when it has no arm
/// on this segment.
fn past_first_label<'a>(words: &'a [&'a str]) -> &'a [&'a str] {
    match words.iter().position(|w| w.contains(')')) {
        // `x)cmd`: the label is glued to its command, which `file_name` reads.
        Some(at) if !words[at].ends_with(')') => &words[at..],
        Some(at) => &words[at + 1..],
        None => &[],
    }
}

/// A program's own name, whatever path, letter case or subshell
/// parentheses it was written with (`/usr/bin/SUDO`, `(sudo`, `sudo)` and
/// a `case` arm's `x)sudo` are all `sudo`): a Mac's disk does not tell
/// `SUDO` from `sudo`.
pub(super) fn file_name(word: &str) -> String {
    let word = word.trim_end_matches(')');
    let word = past_a_glued_label(word);
    let word = word.rsplit('/').next().unwrap_or(word);
    word.trim_start_matches('(').to_ascii_lowercase()
}

/// `word` past a `case` arm's label glued to it (`x)cmd`, `a|b)cmd`). A
/// `)` after a `/` is part of a file's name, not a label's end.
fn past_a_glued_label(word: &str) -> &str {
    match word.rsplit_once(')') {
        Some((label, command)) if !label.contains('/') => command,
        _ => word,
    }
}

/// A piece of the command cut at `;`, `&&`, `||`, `|` without regard to
/// quotes: its program is read past wrappers only.
fn segment_is_hardcoded_deny(seg: &str) -> bool {
    let words: Vec<String> = seg.split_whitespace().map(dequote).collect();
    run_is_hardcoded_deny(&words, false)
}

/// A command as the shell splits it: its program is read past the shell's
/// own words too.
fn words_are_hardcoded_deny(words: &[String]) -> bool {
    run_is_hardcoded_deny(words, true)
}

fn run_is_hardcoded_deny(words: &[String], shell_words: bool) -> bool {
    let all: Vec<&str> = words.iter().map(String::as_str).collect();
    let tokens = run_of(&all, shell_words);
    if tokens.is_empty() {
        return false;
    }
    let program = file_name(tokens[0]);

    // sudo / sudoedit — privilege escalation, never allowed.
    if program == "sudo" || program == "sudoedit" {
        return true;
    }

    // mkfs.* — filesystem creation on a device.
    if program == "mkfs" || program.starts_with("mkfs.") {
        return true;
    }

    // dd of=/dev/{disk,sd*,nvme*,hd*,mmcblk*} — direct disk overwrite.
    if program == "dd" {
        for arg in &tokens[1..] {
            if let Some(target) = arg.strip_prefix("of=") {
                if dd_target_is_blockdev(target) {
                    return true;
                }
            }
        }
    }

    // rm -rf / and rm -rf /* — whole-disk wipe.
    if program == "rm" {
        let has_recursive = tokens[1..].iter().any(|t| {
            *t == "-rf"
                || *t == "-fr"
                || *t == "-Rf"
                || *t == "-fR"
                || *t == "--recursive"
                || *t == "-r"
                || *t == "-R"
        });
        let has_force = tokens[1..].iter().any(|t| {
            *t == "-f"
                || *t == "-rf"
                || *t == "-fr"
                || *t == "-Rf"
                || *t == "-fR"
                || *t == "--force"
        });
        if has_recursive && has_force {
            for arg in &tokens[1..] {
                if rm_target_is_root(arg) {
                    return true;
                }
            }
        }
    }

    // chown -R … / and chmod -R … / — root-tree ownership/mode bombs.
    if program == "chown" || program == "chmod" {
        let has_recursive = tokens[1..]
            .iter()
            .any(|t| *t == "-R" || *t == "--recursive");
        if has_recursive {
            if let Some(last) = tokens.last() {
                if rm_target_is_root(last) {
                    return true;
                }
            }
        }
    }

    false
}

fn dd_target_is_blockdev(target: &str) -> bool {
    if let Some(rest) = target.strip_prefix("/dev/") {
        return rest.starts_with("disk")
            || rest.starts_with("sd")
            || rest.starts_with("nvme")
            || rest.starts_with("hd")
            || rest.starts_with("mmcblk");
    }
    false
}

fn rm_target_is_root(arg: &str) -> bool {
    matches!(arg, "/" | "/*" | "/.*" | "~" | "~/" | "~/*" | "/.")
}

/// Split a compound command into segments by `;`, `&&`, `||`, `|`.
fn split_command_segments(cmd: &str) -> Vec<String> {
    let bytes = cmd.as_bytes();
    let len = bytes.len();
    let mut segments = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < len {
        let b = bytes[i];
        let split = match b {
            b';' => Some(1usize),
            b'|' => Some(if i + 1 < len && bytes[i + 1] == b'|' {
                2
            } else {
                1
            }),
            b'&' if i + 1 < len && bytes[i + 1] == b'&' => Some(2),
            _ => None,
        };
        if let Some(advance) = split {
            segments.push(cmd[start..i].to_string());
            i += advance;
            start = i;
        } else {
            i += 1;
        }
    }
    if start < len {
        segments.push(cmd[start..].to_string());
    }
    segments
}

// ---------------------------------------------------------------------------
// Path utilities + effective-mode lookup
// ---------------------------------------------------------------------------

#[cfg(test)]
#[test]
fn permission_tilde_expansion() {
    // Targets and grants are expanded before matching or pruning.
    crate::util::tilde_cases::check(|path, home| {
        expand_tilde(Path::new(path), home).to_string_lossy().into_owned()
    });
}

/// Tilde-expand and resolve symlinks. On macOS, `/tmp` and `/private/tmp` are
/// the same directory but differ as strings — the engine's `cwd()` returns
/// the canonical `/private/tmp`, while `session.yaml.cwd` keeps the typed
/// `/tmp`. Without canonicalizing both sides at lookup, a grant on one form
/// fails to cover a target on the other.
///
/// Falls back to the tilde-expanded path when canonicalize fails (e.g. the
/// leaf doesn't exist yet — common for Write to a new file). In that case
/// we walk up to the nearest existing ancestor, canonicalize it, and append
/// the missing tail so `/tmp/new-file.txt` still normalizes to
/// `/private/tmp/new-file.txt`.
pub(super) fn normalize_path(path: &str) -> String {
    let expanded = expand_tilde(Path::new(path), dirs::home_dir().as_deref());
    let expanded = expanded.to_string_lossy().into_owned();
    let mut p = std::path::PathBuf::from(&expanded);
    if let Ok(c) = p.canonicalize() {
        return c.to_string_lossy().to_string();
    }
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    while let Some(parent) = p.parent().map(|x| x.to_path_buf()) {
        // file_name() drops `..`. Keep it until the nearest real ancestor
        // has been canonicalized, then collapse the missing suffix. A new
        // path such as /project/missing/../../outside must not inherit the
        // /project grant merely because `missing` does not exist yet.
        if let Some(component) = p.components().next_back() {
            suffix.push(component.as_os_str().to_os_string());
        }
        if parent.as_os_str().is_empty() {
            break;
        }
        if let Ok(c) = parent.canonicalize() {
            let mut out = c;
            for n in suffix.iter().rev() {
                if n == ".." {
                    out.pop();
                } else if n != "." {
                    out.push(n);
                }
            }
            return out.to_string_lossy().to_string();
        }
        if parent == p {
            break;
        }
        p = parent;
    }
    expanded
}

/// Find the effective permission mode for a target path by checking path_modes.
/// Returns the mode from the most specific (longest) matching path.
/// Returns `None` if no grant covers the target — caller treats as `chat`.
pub fn effective_mode_for_path(path_modes: &[PathMode], target: &Path) -> Option<PermissionMode> {
    let target_str = normalize_path(&target.to_string_lossy());
    let mut best: Option<(&PathMode, usize)> = None;

    for pm in path_modes {
        // The session-wide wildcard is the least-specific grant; a concrete
        // path can still override it, including with a lower mode.
        if pm.path == "*" {
            if best.is_none() {
                best = Some((pm, 0));
            }
            continue;
        }
        let grant_path = normalize_path(&pm.path);
        if target_str.starts_with(&grant_path)
            && (target_str.len() == grant_path.len()
                || grant_path == "/"
                || target_str.as_bytes().get(grant_path.len()) == Some(&b'/'))
        {
            let specificity = grant_path.len();
            if best.is_none() || specificity > best.unwrap().1 {
                best = Some((pm, specificity));
            }
        }
    }
    best.map(|(pm, _)| pm.mode)
}

/// Canonical OS temp roots — scratch space always usable by any skill or
/// agent without a permission prompt. Covers `/tmp` and `/private/tmp`
/// (the same dir on macOS) plus `$TMPDIR` (`std::env::temp_dir()`, e.g.
/// `/var/folders/.../T` on macOS). Computed once. The hardcoded deny floor
/// (sudo, rm -rf /, mkfs, …) still applies to any command touching temp.
fn temp_roots() -> &'static [String] {
    static ROOTS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    ROOTS.get_or_init(|| {
        let mut v: Vec<String> = ["/tmp", "/private/tmp"]
            .iter()
            .map(|p| normalize_path(p))
            .collect();
        v.push(normalize_path(&std::env::temp_dir().to_string_lossy()));
        v.retain(|s| !s.is_empty());
        v.sort();
        v.dedup();
        v
    })
}

/// True when `target` resolves inside an OS temp root. Such paths are
/// exempt from permission gating — treated as always-granted scratch.
pub(super) fn is_under_temp(target: &Path) -> bool {
    let t = normalize_path(&target.to_string_lossy());
    temp_roots().iter().any(|root| is_within(&t, root))
}

/// Under a turn's ceiling, Linggen's own folder is the owner's alone: its
/// settings, its credentials, every session's record. Nothing in it is
/// reached by a turn another device sent — whatever the session's grants
/// or a skill's cover — but the session's own folder, read.
fn owners_alone(
    perms: &SessionPermissions,
    (tier, ceiling): (PermissionMode, Option<PermissionMode>),
    target: &Path,
) -> bool {
    if ceiling.is_none() || reads_own_dir(perms, tier, target) {
        return false;
    }
    let home = normalize_path(&crate::paths::linggen_home().to_string_lossy());
    is_within(&normalize_path(&target.to_string_lossy()), &home)
}

/// A read inside the session's own folder (its tasks' logs): allowed
/// without a grant. Never a write, and never another session's folder —
/// both sides are resolved, so `<own>/../<other>` is the other one.
fn reads_own_dir(perms: &SessionPermissions, tier: PermissionMode, target: &Path) -> bool {
    let Some(own) = perms
        .own_dir
        .as_deref()
        .filter(|own| tier <= PermissionMode::Read && names_one_session(own))
    else {
        return false;
    };
    let own = normalize_path(&own.to_string_lossy());
    is_within(&normalize_path(&target.to_string_lossy()), &own)
}

/// The folder ends in a real session id: never the sessions folder itself
/// (an empty id) or a parent of it (`.`, `..`).
fn names_one_session(own: &Path) -> bool {
    let text = own.to_string_lossy();
    let last = text.rsplit(['/', '\\']).next().unwrap_or("");
    !matches!(last, "" | "." | "..")
        && !own
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// `path` is `root` or inside it (both normalized strings).
fn is_within(path: &str, root: &str) -> bool {
    path.starts_with(root)
        && (path.len() == root.len() || path.as_bytes().get(root.len()) == Some(&b'/'))
}

// ---------------------------------------------------------------------------
// Tool tier mapping
// ---------------------------------------------------------------------------

/// Map a tool name to its permission mode requirement.
///
/// Lookup order:
/// 1. Built-in tools — `engine::tools::builtin_tier(name)`. Owns Read,
///    Write, Edit, Bash, Glob, Grep, Task, Skill,
///    RunApp, lock_paths, unlock_paths, WebSearch, WebFetch, AskUser.
/// 2. Plan-mode tools (`EnterPlanMode`, `ExitPlanMode`, `UpdatePlan`) —
///    routed through actions.rs, not Tools::execute, but still need a
///    permission tier for the gate.
/// 3. Unknown — `PermissionMode::Admin` (fail closed).
pub fn tool_action_tier(tool: &str) -> PermissionMode {
    if let Some(tier) = crate::engine::tools::builtin_tier(tool) {
        return tier;
    }
    match tool {
        "EnterPlanMode" | "ExitPlanMode" | "UpdatePlan" => PermissionMode::Read,
        _ => PermissionMode::Admin,
    }
}

/// Parse the `tier:` string from a skill tool's manifest into a `PermissionMode`.
pub fn parse_skill_tier(tier: &str) -> Option<PermissionMode> {
    match tier {
        "read" => Some(PermissionMode::Read),
        "edit" | "write" => Some(PermissionMode::Edit),
        "admin" => Some(PermissionMode::Admin),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Permission check
// ---------------------------------------------------------------------------

/// The main permission check.
///
/// The caller (tool_exec) decides what to do with `NeedsPrompt`: prompt the
/// user if `session_perms.interactive`, otherwise treat as permission-needed
/// (pause/fail) for missions and consumer sessions.
pub fn check_permission(
    tool: &str,
    bash_command: Option<&str>,
    file_path: Option<&str>,
    session_cwd: &Path,
    session_perms: &SessionPermissions,
    action_tier_override: Option<PermissionMode>,
) -> PermissionCheckResult {
    check_permission_for(
        tool,
        bash_command,
        file_path,
        session_cwd,
        session_perms,
        action_tier_override,
        false,
        None,
    )
}

/// [`check_permission`] for a session that is, or is not, a coding session
/// (`work: code`): one runs its programs where it may edit, without asking
/// ([`super::classify`]'s `coding_class` says which); and for a turn under a
/// `ceiling` (`SessionSpec::ceiling`, the level of the device that sent it):
/// an action that needs more is refused, never asked for, whatever the
/// session's grants.
#[allow(clippy::too_many_arguments)]
pub fn check_permission_for(
    tool: &str,
    bash_command: Option<&str>,
    file_path: Option<&str>,
    session_cwd: &Path,
    session_perms: &SessionPermissions,
    action_tier_override: Option<PermissionMode>,
    coding: bool,
    ceiling: Option<PermissionMode>,
) -> PermissionCheckResult {
    // `$HOME` and `$?` are read as the shell reads them, so a command naming
    // the home folder is judged by the path it reaches (`known_vars`).
    let read_as = bash_command
        .filter(|_| tool == "Bash")
        .and_then(super::known_vars::read_as_the_shell);
    let bash_command = read_as.as_deref().or(bash_command);
    let inner = bash_command
        .filter(|_| coding && tool == "Bash")
        .map(literal_shell_commands)
        .unwrap_or_default();
    if inner.iter().any(|text| is_hardcoded_deny(text)) {
        return PermissionCheckResult::Blocked(BashReason::Floor.sentence(PermissionMode::Admin, ""));
    }
    let result = check_grants(
        tool,
        bash_command,
        file_path,
        session_cwd,
        session_perms,
        (action_tier_override, ceiling),
        coding,
    );
    let result = match bash_command.filter(|cmd| {
        tool == "Bash" && (super::outward::is_outward(cmd)
            || inner.iter().any(|text| super::outward::is_outward(text)))
    }) {
        Some(cmd) => ask_outward(result, cmd, session_cwd, coding),
        None => result,
    };
    match (&result, ceiling) {
        (PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { target_mode, .. }), Some(c))
            if *target_mode > c =>
        {
            beyond_ceiling(*target_mode, c)
        }
        _ => result,
    }
}

/// An action above a turn's ceiling: refused, with what it needs and the
/// level the turn's device stands at.
fn beyond_ceiling(needs: PermissionMode, ceiling: PermissionMode) -> PermissionCheckResult {
    PermissionCheckResult::Blocked(format!(
        "Beyond this turn's level: the action needs {needs}, and the device that sent the turn stands at {ceiling}."
    ))
}

/// The check against the session's grants: the deny floor, then the
/// action's level on every path it touches.
fn check_grants(
    tool: &str,
    bash_command: Option<&str>,
    file_path: Option<&str>,
    session_cwd: &Path,
    session_perms: &SessionPermissions,
    (action_tier_override, ceiling): (Option<PermissionMode>, Option<PermissionMode>),
    coding: bool,
) -> PermissionCheckResult {
    // 0a. Hardcoded deny floor (admin mode does not bypass).
    if tool == "Bash" {
        if let Some(cmd) = bash_command {
            if is_hardcoded_deny(cmd) {
                let reason = if coding {
                    BashReason::Floor.sentence(PermissionMode::Admin, "")
                } else {
                    "Command blocked by safety floor (sudo, rm -rf /, mkfs, etc.)".into()
                };
                return PermissionCheckResult::Blocked(reason);
            }
        }
    }

    // 0b. Skill is a navigation primitive — always allowed regardless of mode.
    // Skills that need elevated permissions request them at activation time
    // through their own SKILL.md `permission:` block. Without this bypass, a
    // chat-mode session can never reach a skill via natural language because
    // the model can't call `Skill` to dispatch to it.
    if tool == "Skill" {
        return PermissionCheckResult::Allowed;
    }

    // 0b'. Under a turn's ceiling no tool runs ungated below: an MCP tool
    // (the memory server's among them — its store is the owner's) is judged
    // by its tier like any other, and refused above the ceiling.
    let ungated_mcp = crate::mcp_client::is_mcp_tool(tool) && !crate::mcp_client::registry().is_gated(tool);

    // 0c. An MCP server whose entry declares `gated = false`. The user, who
    // is the host, said they trust it — the direct analogue of a Claude Code
    // allow rule covering a whole server, and the only direction trust flows.
    // A server can never clear its own gate: `readOnlyHint` is an advisory
    // hint from the thing being gated.
    //
    // The built-in memory server ships this way, which is why nothing here
    // knows ling-mem's name. It also has to be mode-independent: a chat-mode
    // session that cannot remember defeats the feature, and auto-recall
    // already runs before the model with no tool call to gate — so gating
    // the write half while the read half is free would be incoherent.
    //
    // `enabled = false` on the entry is how memory is turned off. This
    // decides whether tools *prompt*, never whether they exist.
    if ungated_mcp && ceiling.is_none() {
        return PermissionCheckResult::Allowed;
    }

    // Chat mode is the lowest tier — any concrete tool's action_tier
    // (Read/Edit/Admin) exceeds it, so step 4 below produces an
    // ExceedsCeiling prompt offering to switch to the needed mode. We
    // intentionally do NOT short-circuit a Chat grant to a hard block:
    // when the user explicitly picks chat and then asks the agent to run
    // something, they want to be asked to upgrade, not silently denied.

    // 1. Classify action tier. Skill-declared override wins over the built-in table.
    // In a coding session what the tables would ask admin for is a write
    // (`lowered`).
    let mut lowered = false;
    let mut bash_reason = None;
    let action_tier = if let Some(override_tier) = action_tier_override {
        override_tier
    } else if tool == "Bash" {
        match bash_command {
            Some(cmd) => {
                let class = classify_bash(cmd, coding);
                lowered = class.1;
                bash_reason = class.2;
                match class.0 {
                    BashClass::Read => PermissionMode::Read,
                    BashClass::Write => PermissionMode::Edit,
                    BashClass::Admin => PermissionMode::Admin,
                }
            }
            None => PermissionMode::Admin,
        }
    } else {
        tool_action_tier(tool)
    };

    // 1b. A turn under a ceiling: an action above it is refused, whatever
    // the grants say.
    if let Some(c) = ceiling.filter(|c| action_tier > *c) {
        return beyond_ceiling(action_tier, c);
    }

    // 1a. Chat-tier tools (AskUser) are at the floor. Nothing exceeds Chat,
    // and they don't touch the workspace — nothing in `session_cwd`.
    // Path-gating them produces a bogus
    // "Switch this folder to chat" ExceedsCeiling prompt when path_modes
    // is empty. Allow unconditionally.
    if action_tier == PermissionMode::Chat {
        return PermissionCheckResult::Allowed;
    }

    // 2. Bash-specific path gating: if the command contains explicit absolute,
    // tilde-prefixed, or upward-relative (`../…`) path args, each must be
    // covered at action_tier. Without this, `bash ls /B` from a session with
    // read-on-/A would pass because step 3 only consulted cwd's tier — but the
    // command's actual reach is /B, not /A. cwd is just where the shell starts;
    // we gate on what the command actually touches. Relative args are resolved
    // against the session cwd, so `cat ../../.ssh/id_rsa` is gated against the
    // file it really reaches, not cwd.
    if tool == "Bash" {
        if let Some(cmd) = bash_command {
            let targets = command_target_levels(cmd, session_cwd);
            if !targets.is_empty() {
                let mut judged = false;
                for (arg_path, kind) in &targets {
                    let tier = if *kind == TargetKind::Full {
                        action_tier
                    } else {
                        PermissionMode::Read
                    };
                    if let Some(c) = ceiling.filter(|_| owners_alone(session_perms, (tier, ceiling), arg_path)) {
                        return beyond_ceiling(PermissionMode::Admin, c);
                    }
                    // OS temp dir is always-available scratch, and a session
                    // reads its own folder — don't gate them.
                    if is_under_temp(arg_path) || reads_own_dir(session_perms, tier, arg_path) {
                        continue;
                    }
                    judged |= *kind == TargetKind::Full;
                    let mode = effective_mode_for_path(&session_perms.path_modes, arg_path);
                    if mode.is_none_or(|m| tier > m) {
                        return needs_prompt(
                            arg_path,
                            session_cwd,
                            tier,
                            format!("{} {}", tool, cmd),
                            coding.then(|| bash_grant_reason(arg_path, session_cwd, bash_reason.as_ref())),
                        );
                    }
                }
                // A scratch path is free; a command that runs code is not.
                // With no other path to judge it (`sh /tmp/x.sh`,
                // `python3 -c … > /tmp/out`), it is judged where it runs,
                // like a command with no path at all — and so is a program
                // a coding session runs: it runs where the session may edit.
                if judged || (action_tier < PermissionMode::Admin && !lowered) {
                    return PermissionCheckResult::Allowed;
                }
            }
            // No path args: fall through to the cwd check below — `ls`,
            // `cargo build`, etc. operate in cwd, so cwd's tier is the gate.
        }
    }

    // 3. Resolve target path for non-Bash tools (or Bash without path args).
    // Tools without an explicit file_path use cwd.
    let target_path = file_path
        .map(|fp| {
            if fp == "~" || fp.starts_with("~/") {
                // An unresolved home-relative path is not rebased onto the session cwd.
                expand_tilde(Path::new(fp), dirs::home_dir().as_deref())
            } else {
                let p = Path::new(fp);
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    session_cwd.join(p)
                }
            }
        })
        .unwrap_or_else(|| session_cwd.to_path_buf());

    if let Some(c) = ceiling.filter(|_| owners_alone(session_perms, (action_tier, ceiling), &target_path)) {
        return beyond_ceiling(PermissionMode::Admin, c);
    }

    // 3b. OS temp dir is always-available scratch (deny floor still applies);
    // a session reads its own folder.
    if is_under_temp(&target_path) || reads_own_dir(session_perms, action_tier, &target_path) {
        return PermissionCheckResult::Allowed;
    }

    // 4. Most-specific grant covering the target.
    if let Some(mode) = effective_mode_for_path(&session_perms.path_modes, &target_path) {
        if action_tier <= mode {
            return PermissionCheckResult::Allowed;
        }
    }

    // 5. Exceeds ceiling (or no grant) — needs upgrade prompt.
    let rule_arg = bash_command.or(file_path);
    let summary = rule_arg.unwrap_or(tool).to_string();
    needs_prompt(
        &target_path,
        session_cwd,
        action_tier,
        format!("{} {}", tool, summary),
        (coding && tool == "Bash").then(|| bash_grant_reason(&target_path, session_cwd, bash_reason.as_ref())),
    )
}

fn bash_grant_reason(target: &Path, cwd: &Path, class: Option<&BashReason>) -> BashReason {
    let target = normalize_path(&target.to_string_lossy());
    let cwd = normalize_path(&cwd.to_string_lossy());
    if !Path::new(&target).starts_with(Path::new(&cwd)) {
        BashReason::Target
    } else {
        class.cloned().unwrap_or(BashReason::Level)
    }
}

fn needs_prompt(
    target: &Path,
    session_cwd: &Path,
    mode: PermissionMode,
    summary: String,
    reason: Option<BashReason>,
) -> PermissionCheckResult {
    let target = PathBuf::from(normalize_path(&target.to_string_lossy()));
    let path = display_path(&target);
    let reason = reason.map(|why| why.sentence(mode, &path));
    let directories = grant_directories(&target, session_cwd)
        .iter()
        .map(|dir| display_path(dir))
        .collect();
    PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling {
        target_mode: mode,
        path,
        directories,
        tool_summary: summary,
        reason,
        outward: false,
    })
}

/// An outward command (`super::outward`) asks whatever the grants: what
/// the check would allow is asked at admin where it runs, and an ask the
/// check makes anyway (a path beyond the grants) becomes the same one-time
/// ask for the whole command. The deny floor still blocks first.
fn ask_outward(
    result: PermissionCheckResult,
    cmd: &str,
    session_cwd: &Path,
    coding: bool,
) -> PermissionCheckResult {
    let ask = match result {
        PermissionCheckResult::Blocked(_) => return result,
        PermissionCheckResult::Allowed => match needs_prompt(
            session_cwd,
            session_cwd,
            PermissionMode::Admin,
            format!("Bash {cmd}"),
            None,
        ) {
            PermissionCheckResult::NeedsPrompt(kind) => kind,
            other => return other,
        },
        PermissionCheckResult::NeedsPrompt(kind) => kind,
    };
    let PromptKind::ExceedsCeiling {
        target_mode,
        path,
        tool_summary,
        reason,
        ..
    } = ask;
    let reason = coding.then(|| {
        let outward = BashReason::Outward.sentence(target_mode, &path);
        match reason {
            Some(need) => format!("{outward} {need}"),
            None => outward,
        }
    });
    PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling {
        target_mode,
        path,
        directories: Vec::new(),
        tool_summary,
        reason,
        outward: true,
    })
}

// ---------------------------------------------------------------------------
// Helpers reused by check_permission and prompt construction
// ---------------------------------------------------------------------------

/// The path a single shell word names, when it is one the gate cares about:
/// absolute, tilde-prefixed or upward-relative. A fully-quoted word is a
/// single string argument — unwrapped so a quoted path ARG (`cat "/etc/x"`)
/// still counts, while a quoted string that merely CONTAINS path-like words
/// (a `ling-mem search "... /usr/local/bin ..."` query, a jq program) stays
/// one word and is not mistaken for a path the command touches.
fn path_token(word: &str) -> Option<String> {
    // The word as the program receives it: `"/etc"/passwd` and
    // `\/etc/passwd` are `/etc/passwd`, whatever part of it is quoted.
    let plain = dequote(word.trim());
    let t = plain.as_str();
    if !(t.starts_with('/') || t.starts_with("~/") || t == "~" || is_upward_relative(t)) {
        return None;
    }
    // Strip trailing punctuation from compound shell syntax.
    let t = t.trim_end_matches([';', '&', '|']);
    if is_device_stream(t) {
        return None;
    }
    // Reject all-slash tokens (`/`, `//`, `///`) — e.g. an unquoted jq
    // `//` operator — which would otherwise demand admin on `//`.
    (!t.trim_matches('/').is_empty()).then(|| t.to_string())
}

/// What a command does to a path it names: the level that path is judged at.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TargetKind {
    /// Acted on by the command: judged at the command's own level.
    Full,
    /// The program being run, named by a path: its file is read, so it is
    /// judged at read — and it is not a path the command was "judged" on (a
    /// command whose only path is its own program is still judged, at its
    /// full level, in the folder it runs in).
    Program,
    /// A source of a plain copy: read, never changed.
    Source,
}

#[cfg(test)]
pub(super) fn command_targets(cmd: &str, session_cwd: &Path) -> Vec<PathBuf> {
    command_target_levels(cmd, session_cwd)
        .into_iter()
        .map(|(p, _)| p)
        .collect()
}

/// The words that are sources of a plain copy (`cp`, `rsync`, `install`,
/// `ln -s`): every operand but the last. Only simple, literal operands and
/// known switch-only options qualify; any doubt leaves the whole segment at
/// its original level. `mv` is not here (it removes its source), nor is a
/// hard link (`ln` without `-s`): the new name is the same file, so writing
/// it would write the source.
fn copy_sources(tokens: &[Token]) -> Vec<usize> {
    let mut sources = Vec::new();
    let mut start = 0;
    while start < tokens.len() {
        let end = (start..tokens.len())
            .find(|&i| matches!(tokens[i], Token::Op(_)))
            .unwrap_or(tokens.len());
        if let Some(Token::Word(program)) = tokens.get(start) {
            let name = program.as_str();
            if matches!(name, "cp" | "rsync" | "install" | "ln") {
                let mut operands = Vec::new();
                let mut safe = true;
                let mut options = true;
                let mut symbolic = false;
                for i in start + 1..end {
                    let Token::Word(w) = &tokens[i] else {
                        unreachable!()
                    };
                    if w.contains([
                        '$', '`', '*', '?', '[', ']', '>', '<', '\\', '\'', '"', '(', ')', '{',
                        '}', '#',
                    ]) || w.chars().any(is_odd_space)
                    {
                        safe = false;
                        break;
                    }
                    if options && w == "--" {
                        options = false;
                        continue;
                    }
                    if options && w.starts_with('-') {
                        let allowed = match name {
                            "cp" => "RrPpfinv",
                            "rsync" => "arltpogDvz",
                            "install" => "psv",
                            "ln" => "sfivn",
                            _ => "",
                        };
                        if w.len() < 2
                            || w.starts_with("--")
                            || !w[1..].chars().all(|c| allowed.contains(c))
                        {
                            safe = false;
                            break;
                        }
                        symbolic |= w[1..].contains('s');
                    } else {
                        operands.push(i);
                    }
                }
                if safe && operands.len() >= 2 && (name != "ln" || symbolic) {
                    sources.extend_from_slice(&operands[..operands.len() - 1]);
                }
            }
        }
        start = end + 1;
    }
    sources
}

/// Every path a Bash command reaches, each resolved where the shell would
/// resolve it. A relative token (`../x`) after `cd <dir>` belongs to `<dir>`,
/// not to the session's folder — `cd skills && ls ../other` reaches
/// `linggen/other`, never the session folder's parent. Only a plain `cd`
/// to a directory that exists, with no variable, glob or subshell syntax
/// anywhere in the command, is followed, and only across `&&`, `;` and a
/// newline. After `|`, `||` or `&` the shell's own folder is not known to
/// have moved (a pipeline stage is a subshell, `||` runs only when the cd
/// failed, `&` backgrounds it), so the rest resolves from the session's
/// folder as before.
///
/// Text the command carries is not a target: a heredoc body, or the pattern
/// of grep, sed, awk or jq (see `shell_text`).
fn command_target_levels(cmd: &str, session_cwd: &Path) -> Vec<(PathBuf, TargetKind)> {
    let cmd = strip_heredoc_bodies(cmd);
    let cmd = cmd.as_str();
    let mut cwd = session_cwd.to_path_buf();
    let mut following = !has_subshell_syntax(cmd);
    let mut at_command = true;
    let mut cd_pending = false;
    // A cd after `&&` may never have run; past the next `;` the folder is
    // then unknown.
    let mut after_and = false;
    let mut moved_conditionally = false;
    let mut out = Vec::new();
    // Relative operands of common file operations are targets too, even when
    // another operand is an absolute or exempt scratch path.
    let mut file_operands = false;
    let tokens = command_tokens(cmd);
    let text = text_words(&tokens);
    let sources = copy_sources(&tokens);
    for (index, (token, text)) in tokens.into_iter().zip(text).enumerate() {
        let w = match token {
            Token::Op(joint) => {
                // A bare `cd` goes home — not followed.
                let unknown = cd_pending
                    || joint == Joint::Break
                    || joint == Joint::Seq && moved_conditionally;
                if unknown {
                    following = false;
                    cwd = session_cwd.to_path_buf();
                }
                after_and = joint == Joint::And || (after_and && joint != Joint::Seq);
                at_command = true;
                file_operands = false;
                cd_pending = false;
                continue;
            }
            Token::Word(w) => w,
        };
        let w = w.as_str();
        // A redirection glued to its file (`>/x`, `2>>/x`, `hi>/x`, `</x`).
        for file in redirect_files(w) {
            if let Some(token) = path_token(file) {
                out.push((resolve_bash_arg(&token, &cwd), TargetKind::Full));
            }
        }
        if text {
            // Pattern text, not a path.
        } else if cd_pending && !is_cd_flag(&dequote(w)) {
            cd_pending = false;
            if let Some(token) = path_token(w) {
                out.push((resolve_bash_arg(&token, &cwd), TargetKind::Full));
            }
            if following {
                match cd_destination(w, &cwd) {
                    Some(dir) => {
                        cwd = dir;
                        moved_conditionally |= after_and;
                    }
                    None => following = false,
                }
            }
        } else if let Some(token) = path_token(w).or_else(|| {
            let t = dequote(w);
            (t.starts_with("./")
                || t == "."
                || (file_operands
                    && !t.starts_with('-')
                    && !t.contains(['$', '`', '*', '?', '[', '>', '<'])))
            .then_some(t)
        }) {
            let kind = if at_command
                && path_token(w).is_some()
                && !w.contains(['>', '<', '$', '`', '*', '?', '[', '\\'])
            {
                TargetKind::Program
            } else if sources.contains(&index) {
                TargetKind::Source
            } else {
                TargetKind::Full
            };
            out.push((resolve_bash_arg(&token, &cwd), kind));
        }
        // Program names and cd flags as the shell passes them (`\rm`, `'cp'`).
        let word = dequote(w);
        if at_command {
            file_operands = matches!(
                word.as_str(),
                "cp" | "mv" | "ln" | "install" | "rsync" | "cat" | "touch" | "mkdir" | "rm"
            );
        }
        cd_pending = cd_pending && is_cd_flag(&word) || (at_command && word == "cd");
        at_command = false;
    }
    out
}

/// The file each redirection operator inside `word` is glued to
/// (`>/x`, `2>>/x`, `hi>/x`, `</x`, `&>/x`, `>|/x`); a `>` in quotes or
/// after a backslash is text.
fn redirect_files(word: &str) -> Vec<&str> {
    let mut files = Vec::new();
    let mut quote: Option<char> = None;
    let mut chars = word.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '\\' && quote != Some('\'') {
            chars.next();
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            continue;
        }
        if c != '>' && c != '<' {
            continue;
        }
        let mut end = i + 1;
        while let Some((j, _)) = chars.next_if(|(_, n)| "<>|&".contains(*n)) {
            end = j + 1;
        }
        files.push(&word[end..]);
    }
    files
}

/// Where output is thrown away or a stream is named: nothing is written
/// to disk.
fn is_device_stream(file: &str) -> bool {
    matches!(
        file,
        "/dev/null" | "/dev/stdin" | "/dev/stdout" | "/dev/stderr" | "/dev/tty"
    )
}

fn is_cd_flag(w: &str) -> bool {
    matches!(w, "-P" | "-L" | "-e" | "--")
}

/// How a control operator joins one command to the next.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Joint {
    /// `;` or a newline: the next command runs in the same shell regardless.
    Seq,
    /// `&&`: the next command runs only when this one succeeded.
    And,
    /// `|`, `|&`, `||` or `&`: a cd before it does not move the shell.
    Break,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Token {
    Word(String),
    Op(Joint),
}

/// Split a command into words and control operators, the way the shell
/// does: quotes keep a span whole (and are retained), and an operator is
/// its own token even when glued to a word (`x&`, `a|b`, `x;`). `>&`/`<&`
/// (`2>&1`) and `&>` are redirections, not operators.
pub(super) fn command_tokens(cmd: &str) -> Vec<Token> {
    fn flush(cur: &mut String, out: &mut Vec<Token>) {
        if !cur.is_empty() {
            out.push(Token::Word(std::mem::take(cur)));
        }
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        // A backslash outside single quotes takes the next character
        // literally — `\'` opens no quote and `\;` ends no command, as the
        // shell reads them.
        if c == '\\' && quote != Some('\'') {
            cur.push(c);
            if let Some(next) = chars.next() {
                cur.push(next);
            }
            continue;
        }
        if let Some(q) = quote {
            cur.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        let op = match c {
            '\'' | '"' => {
                quote = Some(c);
                cur.push(c);
                continue;
            }
            ';' | '\n' => Joint::Seq,
            // `>|` is a redirect that overwrites, not a pipe.
            '|' if cur.ends_with('>') => {
                cur.push(c);
                continue;
            }
            '|' => {
                chars.next_if(|&n| n == '|' || n == '&');
                Joint::Break
            }
            '&' if cur.ends_with(['>', '<']) || chars.peek() == Some(&'>') => {
                cur.push(c);
                continue;
            }
            '&' if chars.next_if_eq(&'&').is_some() => Joint::And,
            '&' => Joint::Break,
            // Only a space or a tab ends a word for the shell: a carriage
            // return or a no-break space stays in it.
            ' ' | '\t' => {
                flush(&mut cur, &mut out);
                continue;
            }
            c => {
                cur.push(c);
                continue;
            }
        };
        flush(&mut cur, &mut out);
        out.push(Token::Op(op));
    }
    flush(&mut cur, &mut out);
    out
}

/// Whitespace the shell does not split words on (a carriage return, a
/// no-break space, …): unquoted, it makes a command read here differently.
pub(super) fn is_odd_space(c: char) -> bool {
    c.is_whitespace() && !matches!(c, ' ' | '\t' | '\n')
}

/// Parentheses, braces, backticks and `$(` open a scope the shell leaves
/// again (a subshell's `cd` does not outlive it) — not worth following.
fn has_subshell_syntax(cmd: &str) -> bool {
    shell_words(cmd).iter().any(|w| {
        let t = w.trim();
        let quoted = t.starts_with('"') || t.starts_with('\'');
        !quoted && t.contains(['(', ')', '{', '}', '`'])
    })
}

/// Where `cd <word>` lands from `cwd` — the real directory, so a later `..`
/// goes where the kernel sends it. `None` for what cannot be known before
/// the command runs (a variable, a glob, `-`, a missing directory).
fn cd_destination(word: &str, cwd: &Path) -> Option<PathBuf> {
    let plain = dequote(word.trim().trim_end_matches([';', '&', '|']));
    let t = plain.as_str();
    if t.is_empty()
        || t == "-"
        || t.contains(['$', '`', '*', '?', '['])
        || t.starts_with("~") && t != "~" && !t.starts_with("~/")
    {
        return None;
    }
    let joined = if t == "~" || t.starts_with("~/") || t.starts_with('/') {
        expand_tilde(Path::new(t), dirs::home_dir().as_deref())
    } else {
        cwd.join(t)
    };
    joined.canonicalize().ok().filter(|p| p.is_dir())
}

/// Split a command line into shell-ish words, keeping single/double quoted
/// spans together as one word (the quotes are retained). Not a full shell
/// parser — just enough to tell a path ARG from a path that merely appears
/// inside a quoted string argument (search query, jq program), which must
/// NOT be gated as a path the command touches.
fn shell_words(cmd: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    for c in cmd.chars() {
        match quote {
            Some(q) => {
                cur.push(c);
                if c == q {
                    quote = None;
                }
            }
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                cur.push(c);
                has = true;
            }
            None if c.is_whitespace() => {
                if has {
                    words.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            None => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        words.push(cur);
    }
    words
}

/// Resolve a path token from a bash command to an absolute path for gating.
/// Absolute (`/x`) and tilde (`~`, `~/x`) tokens expand as-is; a relative token
/// is joined onto the session cwd so its real target — including any `..`
/// traversal, which `normalize_path` then collapses — is what gets gated,
/// rather than the process-global cwd that `canonicalize` would otherwise use.
fn resolve_bash_arg(path_arg: &str, session_cwd: &Path) -> PathBuf {
    if path_arg == "~" || path_arg.starts_with("~/") || path_arg.starts_with('/') {
        expand_tilde(Path::new(path_arg), dirs::home_dir().as_deref())
    } else {
        session_cwd.join(path_arg)
    }
}

/// True for a relative token that climbs out of cwd via a `..` path component
/// (e.g. `../secret`, `a/../../b`). Such a token can reach outside the granted
/// folder, so it must be gated against its resolved target. Relative tokens
/// without `..` stay inside cwd and are already covered by the cwd-tier check,
/// so they are not extracted — keeping subcommand words (`build`, `lint`) and
/// git ranges (`main..feature`, a single component with no `..`) ungated.
fn is_upward_relative(token: &str) -> bool {
    if token.starts_with('/') || token.starts_with('~') {
        return false;
    }
    token.split('/').any(|c| c == "..")
}

/// Directories the user can grant, broadest first.
///
/// The chain runs from the file's directory up to the git repo root. With no
/// repo root it stops before the home directory, so a path `a/b/c/file` can
/// be granted at `a` without granting every directory. `/` and system
/// directories such as `/dev` are not offered.
fn grant_directories(target: &Path, _session_cwd: &Path) -> Vec<PathBuf> {
    let target = PathBuf::from(normalize_path(&target.to_string_lossy()));
    let start = if target.is_dir() {
        target
    } else {
        target.parent().map(Path::to_path_buf).unwrap_or(target)
    };
    if ungrantable(&start) {
        return Vec::new();
    }
    let mut leaf_to_root = Vec::new();
    let mut cur = start;
    for _ in 0..12 {
        leaf_to_root.push(cur.clone());
        if cur.join(".git").exists() {
            break;
        }
        let Some(parent) = cur.parent() else { break };
        if parent.as_os_str().is_empty()
            || parent == Path::new("/")
            || ungrantable(parent)
            || is_home(parent)
        {
            break;
        }
        cur = parent.to_path_buf();
    }
    thin_directories(leaf_to_root)
}

fn is_home(path: &Path) -> bool {
    dirs::home_dir()
        .is_some_and(|home| PathBuf::from(normalize_path(&home.to_string_lossy())) == path)
}

/// A grant here is either the whole machine or a device path such as `/dev`.
/// A folder inside the home directory never is, wherever the home lives.
fn ungrantable(path: &Path) -> bool {
    let home = dirs::home_dir().map(|home| PathBuf::from(normalize_path(&home.to_string_lossy())));
    ungrantable_with_home(path, home.as_deref())
}

/// A home of `/` or a bare root such as `/var` exempts nothing.
fn ungrantable_with_home(path: &Path, home: Option<&Path>) -> bool {
    let in_home = home.is_some_and(|home| {
        home.components().count() > 2 && path != home && path.starts_with(home)
    });
    !in_home && system_path(path)
}

fn system_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    const ROOTS: &[&str] = &[
        "/dev",
        "/tmp",
        "/private",
        "/var",
        "/usr",
        "/bin",
        "/sbin",
        "/etc",
        "/opt",
        "/System",
        "/Library",
        "/Applications",
    ];
    text == "/"
        || ROOTS
            .iter()
            .any(|root| text == *root || text.starts_with(&format!("{root}/")))
}

/// Keep the repo root and the directories closest to the file. The middle of
/// a very deep path is dropped so the card stays short.
fn thin_directories(leaf_to_root: Vec<PathBuf>) -> Vec<PathBuf> {
    const MAX: usize = 6;
    if leaf_to_root.len() <= MAX {
        let mut dirs = leaf_to_root;
        dirs.reverse();
        return dirs;
    }
    let root = leaf_to_root.last().unwrap().clone();
    let mut closest: Vec<PathBuf> = leaf_to_root[..MAX - 1].to_vec();
    closest.reverse();
    let mut out = vec![root.clone()];
    out.extend(closest.into_iter().filter(|dir| *dir != root));
    out
}

pub fn display_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        let s = path.to_string_lossy();
        let h = home.to_string_lossy();
        if s.starts_with(h.as_ref()) {
            return format!("~{}", &s[h.len()..]);
        }
    }
    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod grantable_tests {
    use super::ungrantable_with_home;
    use std::path::Path;

    #[test]
    fn a_folder_in_a_home_under_a_system_root_is_grantable() {
        let home = Some(Path::new("/private/var/tmp/w/home"));
        assert!(!ungrantable_with_home(
            Path::new("/private/var/tmp/w/home/work"),
            home
        ));
        assert!(ungrantable_with_home(
            Path::new("/private/var/tmp/w/home"),
            home
        ));
        assert!(ungrantable_with_home(
            Path::new("/private/var/tmp/w/homework"),
            home
        ));
        assert!(ungrantable_with_home(Path::new("/dev/null"), home));
    }

    #[test]
    fn a_bare_root_home_exempts_nothing() {
        assert!(ungrantable_with_home(
            Path::new("/var/x"),
            Some(Path::new("/var"))
        ));
        assert!(ungrantable_with_home(
            Path::new("/dev"),
            Some(Path::new("/"))
        ));
        assert!(!ungrantable_with_home(
            Path::new("/Users/alex/repo"),
            Some(Path::new("/Users/alex"))
        ));
    }
}
