//! Permission model: types, bash classification, path matching, tier mapping,
//! and the `check_permission` decision function.
//!
//! All pure logic — no I/O, no UI prompts. The store layer lives in
//! [`super::store`]; prompt construction in [`super::prompt`].

use super::store::SessionPermissions;
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
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
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
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
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
    },
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
    false
}

fn segment_is_hardcoded_deny(seg: &str) -> bool {
    let tokens: Vec<&str> = seg.split_whitespace().collect();
    if tokens.is_empty() {
        return false;
    }
    let program = tokens[0];

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
// Bash command classifier
// ---------------------------------------------------------------------------

/// Best-effort risk classification, not a shell parser or filesystem sandbox.
/// Unknown dynamic shell syntax is deliberately treated as admin.
pub fn classify_bash_command(cmd: &str) -> BashClass {
    let mut quote = None;
    let mut escaped = false;
    for c in cmd.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if c == '`' && quote != Some('\'') {
            return BashClass::Admin;
        }
        if c == '$' && quote != Some('\'') {
            // Variables and substitutions can change both the command and its targets.
            return BashClass::Admin;
        }
        if c == '\'' || c == '"' {
            if quote == Some(c) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(c);
            }
        }
    }
    if quote.is_some() || escaped {
        return BashClass::Admin;
    }
    let mut highest = BashClass::Read;
    let mut words = Vec::new();
    for token in command_tokens(cmd)
        .into_iter()
        .chain(std::iter::once(Token::Op(Joint::Seq)))
    {
        match token {
            Token::Word(w) => words.push(w),
            Token::Op(_) if !words.is_empty() => {
                highest = highest.max(classify_segment(&words));
                words.clear();
            }
            Token::Op(_) => {}
        }
    }
    if cmd.trim().is_empty() {
        BashClass::Admin
    } else {
        highest
    }
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

/// `env [-flags] [NAME=value…] command…` runs that command: it is what is
/// classified. `env` alone only prints.
fn env_command(args: &[String]) -> Option<&[String]> {
    let start = args
        .iter()
        .position(|a| !a.starts_with('-') && !a.contains('='))?;
    Some(&args[start..])
}

fn classify_segment(words: &[String]) -> BashClass {
    if has_process_substitution(words) {
        return BashClass::Admin;
    }
    let mut clean = Vec::new();
    let mut redirect = false;
    let mut awaiting_target = false;
    for word in words {
        let mut quote = None;
        let mut escaped = false;
        let mut redirect_at = None;
        for (i, c) in word.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' && quote != Some('\'') {
                escaped = true;
                continue;
            }
            if c == '\'' || c == '"' {
                if quote == Some(c) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(c);
                }
            } else if c == '>' && quote.is_none() {
                redirect_at = Some(i);
                break;
            }
        }
        if awaiting_target {
            if word != "/dev/null" && word != "&1" && word != "&2" {
                redirect = true;
            }
            awaiting_target = false;
        } else if let Some(i) = redirect_at {
            let before = &word[..i];
            if !before.is_empty() && !before.chars().all(|c| c.is_ascii_digit() || c == '&') {
                clean.push(before.to_string());
            }
            let target = word[i..].trim_start_matches(['>', '|']);
            if target.is_empty() {
                awaiting_target = true;
            } else if target != "/dev/null" && target != "&1" && target != "&2" {
                redirect = true;
            }
        } else {
            clean.push(word.clone());
        }
    }
    if clean.is_empty() {
        return BashClass::Admin;
    }
    let program = clean[0].as_str();
    if program == "env" {
        if let Some(command) = env_command(&clean[1..]) {
            let class = classify_segment(command);
            return if redirect || awaiting_target {
                class.max(BashClass::Write)
            } else {
                class
            };
        }
    }
    let sub = clean.get(1).map(String::as_str).unwrap_or("");
    let args: Vec<&str> = clean.iter().skip(2).map(String::as_str).collect();
    let class = match (program, sub) {
        ("find", _)
            if clean.iter().any(|a| {
                matches!(
                    a.as_str(),
                    "-delete"
                        | "-exec"
                        | "-execdir"
                        | "-ok"
                        | "-okdir"
                        | "-fprint"
                        | "-fprintf"
                        | "-fls"
                )
            }) =>
        {
            BashClass::Admin
        }
        ("git", "branch" | "tag" | "remote" | "stash") => {
            let read = match sub {
                "branch" | "tag" => {
                    args.is_empty()
                        || args.iter().all(|a| {
                            matches!(
                                *a,
                                "-a" | "-r"
                                    | "-v"
                                    | "-vv"
                                    | "-l"
                                    | "--list"
                                    | "--all"
                                    | "--verbose"
                            )
                        })
                }
                "remote" => {
                    args.is_empty()
                        || args == ["-v"]
                        || matches!(args.first(), Some(&"show" | &"get-url"))
                }
                "stash" => matches!(args.first(), Some(&"list" | &"show")),
                _ => false,
            };
            if read {
                BashClass::Read
            } else {
                BashClass::Write
            }
        }
        ("docker" | "podman", "system") => {
            if matches!(args.first(), Some(&"df" | &"events" | &"info")) {
                BashClass::Read
            } else {
                BashClass::Admin
            }
        }
        _ => classify_single_command(program, sub),
    };
    if redirect || awaiting_target {
        class.max(BashClass::Write)
    } else {
        class
    }
}

fn classify_single_command(program: &str, subcommand: &str) -> BashClass {
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
        "env",
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

    const GIT_READ: &[&str] = &[
        "status",
        "log",
        "diff",
        "show",
        "branch",
        "tag",
        "remote",
        "rev-parse",
        "blame",
        "stash",
        "describe",
        "shortlog",
        "ls-files",
        "ls-tree",
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
    const DOCKER_READ: &[&str] = &[
        "ps", "images", "logs", "inspect", "version", "info", "system", "history", "port", "top",
        "stats", "diff", "events",
    ];

    if ADMIN_PROGRAMS.contains(&program) {
        return BashClass::Admin;
    }
    if READ_PROGRAMS.contains(&program) {
        return BashClass::Read;
    }
    if WRITE_PROGRAMS.contains(&program) {
        return BashClass::Write;
    }

    match program {
        "git" => {
            if GIT_READ.contains(&subcommand) {
                BashClass::Read
            } else if GIT_WRITE.contains(&subcommand) {
                BashClass::Write
            } else {
                BashClass::Admin
            }
        }
        "cargo" => {
            if CARGO_READ.contains(&subcommand) {
                BashClass::Read
            } else if CARGO_WRITE.contains(&subcommand) {
                BashClass::Write
            } else {
                BashClass::Admin
            }
        }
        "npm" | "npx" | "yarn" | "pnpm" => {
            if NPM_READ.contains(&subcommand) {
                BashClass::Read
            } else if NPM_WRITE.contains(&subcommand) {
                BashClass::Write
            } else {
                BashClass::Admin
            }
        }
        "pip" | "pip3" => {
            if PIP_READ.contains(&subcommand) {
                BashClass::Read
            } else if PIP_WRITE.contains(&subcommand) {
                BashClass::Write
            } else {
                BashClass::Admin
            }
        }
        "go" => {
            if GO_READ.contains(&subcommand) {
                BashClass::Read
            } else if GO_WRITE.contains(&subcommand) {
                BashClass::Write
            } else {
                BashClass::Admin
            }
        }
        "python" | "python3" | "node" => {
            if subcommand == "--version" || subcommand == "--help" || subcommand == "-V" {
                BashClass::Read
            } else {
                BashClass::Admin
            }
        }
        "curl" => {
            if subcommand == "-I" || subcommand == "--head" {
                BashClass::Read
            } else {
                BashClass::Admin
            }
        }
        "wget" => {
            if subcommand == "--spider" {
                BashClass::Read
            } else {
                BashClass::Admin
            }
        }
        "make" | "cmake" | "ninja" | "mvn" | "gradle" | "pytest" | "jest" | "vitest" => {
            BashClass::Write
        }
        "brew" => {
            if BREW_READ.contains(&subcommand) {
                BashClass::Read
            } else {
                BashClass::Admin
            }
        }
        "docker" | "podman" => {
            if DOCKER_READ.contains(&subcommand) {
                BashClass::Read
            } else {
                BashClass::Admin
            }
        }
        _ => BashClass::Admin,
    }
}

// ---------------------------------------------------------------------------
// Path utilities + effective-mode lookup
// ---------------------------------------------------------------------------

/// Pure tilde expansion (no canonicalization). Used by the store layer to
/// compare grant paths during pruning.
pub(super) fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    } else if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().to_string();
        }
    }
    path.to_string()
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
fn normalize_path(path: &str) -> String {
    let expanded = expand_tilde(path);
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
    temp_roots().iter().any(|root| {
        t.starts_with(root.as_str())
            && (t.len() == root.len() || t.as_bytes().get(root.len()) == Some(&b'/'))
    })
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
    // 0a. Hardcoded deny floor (admin mode does not bypass).
    if tool == "Bash" {
        if let Some(cmd) = bash_command {
            if is_hardcoded_deny(cmd) {
                return PermissionCheckResult::Blocked(
                    "Command blocked by safety floor (sudo, rm -rf /, mkfs, etc.)".to_string(),
                );
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
    if crate::mcp_client::is_mcp_tool(tool) && !crate::mcp_client::registry().is_gated(tool) {
        return PermissionCheckResult::Allowed;
    }

    // Chat mode is the lowest tier — any concrete tool's action_tier
    // (Read/Edit/Admin) exceeds it, so step 4 below produces an
    // ExceedsCeiling prompt offering to switch to the needed mode. We
    // intentionally do NOT short-circuit a Chat grant to a hard block:
    // when the user explicitly picks chat and then asks the agent to run
    // something, they want to be asked to upgrade, not silently denied.

    // 1. Classify action tier. Skill-declared override wins over the built-in table.
    let action_tier = if let Some(override_tier) = action_tier_override {
        override_tier
    } else if tool == "Bash" {
        match bash_command {
            Some(cmd) => match classify_bash_command(cmd) {
                BashClass::Read => PermissionMode::Read,
                BashClass::Write => PermissionMode::Edit,
                BashClass::Admin => PermissionMode::Admin,
            },
            None => PermissionMode::Admin,
        }
    } else {
        tool_action_tier(tool)
    };

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
            let targets = command_targets(cmd, session_cwd);
            if !targets.is_empty() {
                for arg_path in &targets {
                    // OS temp dir is always-available scratch — don't gate it.
                    if is_under_temp(arg_path) {
                        continue;
                    }
                    let mode = effective_mode_for_path(&session_perms.path_modes, arg_path);
                    if mode.is_none_or(|m| action_tier > m) {
                        return needs_prompt(
                            arg_path,
                            session_cwd,
                            action_tier,
                            format!("{} {}", tool, cmd),
                        );
                    }
                }
                return PermissionCheckResult::Allowed;
            }
            // No path args: fall through to the cwd check below — `ls`,
            // `cargo build`, etc. operate in cwd, so cwd's tier is the gate.
        }
    }

    // 3. Resolve target path for non-Bash tools (or Bash without path args).
    // Tools without an explicit file_path use cwd.
    let target_path = file_path
        .map(|fp| {
            if fp == "~" {
                dirs::home_dir().unwrap_or_else(|| PathBuf::from(fp))
            } else if let Some(rest) = fp.strip_prefix("~/") {
                dirs::home_dir()
                    .map(|h| h.join(rest))
                    .unwrap_or_else(|| PathBuf::from(fp))
            } else {
                let p = PathBuf::from(fp);
                if p.is_absolute() {
                    p
                } else {
                    session_cwd.join(p)
                }
            }
        })
        .unwrap_or_else(|| session_cwd.to_path_buf());

    // 3b. OS temp dir is always-available scratch (deny floor still applies).
    if is_under_temp(&target_path) {
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
    )
}

fn needs_prompt(
    target: &Path,
    session_cwd: &Path,
    mode: PermissionMode,
    summary: String,
) -> PermissionCheckResult {
    let target = PathBuf::from(normalize_path(&target.to_string_lossy()));
    let directories = grant_directories(&target, session_cwd)
        .iter()
        .map(|dir| display_path(dir))
        .collect();
    PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling {
        target_mode: mode,
        path: display_path(&target),
        directories,
        tool_summary: summary,
    })
}

// ---------------------------------------------------------------------------
// Helpers reused by check_permission and prompt construction
// ---------------------------------------------------------------------------

/// Extract absolute (`/foo/bar`), tilde-prefixed (`~`, `~/foo`), and
/// upward-relative (`../foo`, `a/../../b`) tokens from a bash command. Used to
/// gate Bash by the paths it actually touches, not just the session cwd's tier.
/// Relative tokens that climb out of cwd via a `..` component are caught so a
/// session granted only on cwd can't reach `cat ../../.ssh/id_rsa`; tokens that
/// stay within cwd are omitted (already covered by the cwd-tier check).
/// Best-effort: doesn't handle quoted paths with spaces, embedded
/// `--flag=/path` forms, shell-variable expansion (`$HOME`), or command
/// substitution (`$(…)`).
pub(super) fn extract_command_paths(cmd: &str) -> Vec<String> {
    shell_words(cmd)
        .iter()
        .filter_map(|w| path_token(w))
        .collect()
}

/// The path a single shell word names, when it is one the gate cares about:
/// absolute, tilde-prefixed or upward-relative. A fully-quoted word is a
/// single string argument — unwrapped so a quoted path ARG (`cat "/etc/x"`)
/// still counts, while a quoted string that merely CONTAINS path-like words
/// (a `ling-mem search "... /usr/local/bin ..."` query, a jq program) stays
/// one word and is not mistaken for a path the command touches.
fn path_token(word: &str) -> Option<String> {
    let t = word.trim();
    let quoted = t.len() >= 2
        && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')));
    let t = if quoted { &t[1..t.len() - 1] } else { t };
    if !(t.starts_with('/') || t.starts_with("~/") || t == "~" || is_upward_relative(t)) {
        return None;
    }
    // Strip trailing punctuation from compound shell syntax.
    let t = t.trim_end_matches([';', '&', '|']);
    // Reject all-slash tokens (`/`, `//`, `///`) — e.g. an unquoted jq
    // `//` operator — which would otherwise demand admin on `//`.
    (!t.trim_matches('/').is_empty()).then(|| t.to_string())
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
pub(super) fn command_targets(cmd: &str, session_cwd: &Path) -> Vec<PathBuf> {
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
    for token in command_tokens(cmd) {
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
        if cd_pending && !is_cd_flag(w) {
            cd_pending = false;
            if let Some(token) = path_token(w) {
                out.push(resolve_bash_arg(&token, &cwd));
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
            let t = w.trim_matches(|c| c == '"' || c == '\'');
            (t.starts_with("./")
                || t == "."
                || (file_operands
                    && !t.starts_with('-')
                    && !t.contains(['$', '`', '*', '?', '[', '>', '<'])))
            .then(|| t.to_string())
        }) {
            out.push(resolve_bash_arg(&token, &cwd));
        }
        if at_command {
            file_operands = matches!(
                w,
                "cp" | "mv" | "ln" | "install" | "rsync" | "cat" | "touch" | "mkdir" | "rm"
            );
        }
        cd_pending = cd_pending && is_cd_flag(w) || (at_command && w == "cd");
        at_command = false;
    }
    out
}

fn is_cd_flag(w: &str) -> bool {
    matches!(w, "-P" | "-L" | "-e" | "--")
}

/// How a control operator joins one command to the next.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Joint {
    /// `;` or a newline: the next command runs in the same shell regardless.
    Seq,
    /// `&&`: the next command runs only when this one succeeded.
    And,
    /// `|`, `|&`, `||` or `&`: a cd before it does not move the shell.
    Break,
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Op(Joint),
}

/// Split a command into words and control operators, the way the shell
/// does: quotes keep a span whole (and are retained), and an operator is
/// its own token even when glued to a word (`x&`, `a|b`, `x;`). `>&`/`<&`
/// (`2>&1`) and `&>` are redirections, not operators.
fn command_tokens(cmd: &str) -> Vec<Token> {
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
            c if c.is_whitespace() => {
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
    let t = word.trim().trim_end_matches([';', '&', '|']);
    let t = t.trim_matches(|c| c == '"' || c == '\'');
    if t.is_empty()
        || t == "-"
        || t.contains(['$', '`', '*', '?', '['])
        || t.starts_with("~") && t != "~" && !t.starts_with("~/")
    {
        return None;
    }
    let joined = if t == "~" || t.starts_with("~/") || t.starts_with('/') {
        expand_path_arg(t)
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

fn expand_path_arg(p: &str) -> PathBuf {
    if p == "~" {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(p))
    } else if let Some(rest) = p.strip_prefix("~/") {
        dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| PathBuf::from(p))
    } else {
        PathBuf::from(p)
    }
}

/// Resolve a path token from a bash command to an absolute path for gating.
/// Absolute (`/x`) and tilde (`~`, `~/x`) tokens expand as-is; a relative token
/// is joined onto the session cwd so its real target — including any `..`
/// traversal, which `normalize_path` then collapses — is what gets gated,
/// rather than the process-global cwd that `canonicalize` would otherwise use.
fn resolve_bash_arg(path_arg: &str, session_cwd: &Path) -> PathBuf {
    if path_arg == "~" || path_arg.starts_with("~/") || path_arg.starts_with('/') {
        expand_path_arg(path_arg)
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
fn ungrantable(path: &Path) -> bool {
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

fn display_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        let s = path.to_string_lossy();
        let h = home.to_string_lossy();
        if s.starts_with(h.as_ref()) {
            return format!("~{}", &s[h.len()..]);
        }
    }
    path.to_string_lossy().to_string()
}
