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
    Deny,
    /// User denied with a custom message to relay to the model.
    DenyWithMessage(String),
}

/// Session permission mode — defines the ceiling of what the agent can do.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Chat,
    Read,
    Edit,
    Admin,
}

impl Default for PermissionMode {
    fn default() -> Self {
        PermissionMode::Read
    }
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
        path: String,
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
            *t == "-rf" || *t == "-fr" || *t == "-Rf" || *t == "-fR" || *t == "--recursive"
                || *t == "-r" || *t == "-R"
        });
        let has_force = tokens[1..].iter().any(|t| {
            *t == "-f" || *t == "-rf" || *t == "-fr" || *t == "-Rf" || *t == "-fR" || *t == "--force"
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
            b'|' => Some(if i + 1 < len && bytes[i + 1] == b'|' { 2 } else { 1 }),
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

fn is_compound_command(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    let len = bytes.len();
    for i in 0..len {
        match bytes[i] {
            b';' | b'|' => return true,
            b'&' if i + 1 < len && bytes[i + 1] == b'&' => return true,
            b'`' => return true,
            b'$' if i + 1 < len && bytes[i + 1] == b'(' => return true,
            _ => {}
        }
    }
    false
}

fn has_output_redirect(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'>' {
            if i > 0 && bytes[i - 1] == b'-' {
                continue;
            }
            if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                continue;
            }
            let after = cmd[i..].trim_start_matches('>').trim();
            if after.starts_with("/dev/null") {
                continue;
            }
            return true;
        }
    }
    false
}

/// Classify a bash command into read/write/admin tier.
pub fn classify_bash_command(cmd: &str) -> BashClass {
    if is_compound_command(cmd) {
        return classify_compound_command(cmd);
    }

    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    if tokens.is_empty() {
        return BashClass::Admin;
    }

    let program = tokens[0];
    let subcommand = tokens.get(1).copied().unwrap_or("");

    if has_output_redirect(cmd) {
        let base = classify_single_command(program, subcommand);
        return if base == BashClass::Admin {
            BashClass::Admin
        } else {
            BashClass::Write
        };
    }

    classify_single_command(program, subcommand)
}

fn classify_single_command(program: &str, subcommand: &str) -> BashClass {
    const READ_PROGRAMS: &[&str] = &[
        "ls", "cat", "head", "tail", "less", "more", "wc", "file", "stat", "du", "df",
        "pwd", "env", "printenv", "echo", "printf", "which", "whereis", "type",
        "find", "grep", "rg", "ag", "ack", "fd", "tree", "bat", "jq", "yq",
        "uname", "hostname", "date", "id", "whoami", "realpath", "dirname", "basename",
        "ping", "dig", "nslookup", "host", "test", "true", "false", "seq", "sort",
        "uniq", "tr", "cut", "paste", "diff", "comm",
        // Read-only system diagnostics (used by sys-doctor and similar inspection
        // skills). These programs only display info — they don't mutate state.
        "uptime", "sw_vers", "vm_stat", "netstat", "ifconfig", "ipconfig", "scutil",
        "ps", "lsof", "vmmap", "iostat",
        // macOS system / security inspection. Bare invocations are read-only;
        // mutating forms (sysctl -w, csrutil disable, spctl --add, fdesetup
        // enable, etc.) all require sudo, which the hardcoded deny floor blocks.
        "sysctl", "spctl", "csrutil", "fdesetup", "socketfilterfw", "systemsetup",
    ];

    const GIT_READ: &[&str] = &[
        "status", "log", "diff", "show", "branch", "tag", "remote", "rev-parse",
        "blame", "stash", "describe", "shortlog", "ls-files", "ls-tree",
    ];

    const CARGO_READ: &[&str] = &["check", "clippy", "doc", "metadata", "tree", "verify-project"];
    const NPM_READ: &[&str] = &["list", "ls", "outdated", "view", "info", "audit", "why", "explain"];
    const PIP_READ: &[&str] = &["list", "show", "freeze", "check"];
    const GO_READ: &[&str] = &["vet", "list", "doc", "env", "version"];

    const ADMIN_PROGRAMS: &[&str] = &[
        "rm", "sudo", "su", "kill", "killall", "pkill",
        "chmod", "chown", "chgrp",
        "podman", "systemctl", "launchctl", "service",
        "mount", "umount", "mkfs", "fdisk", "dd",
        "apt", "apt-get", "yum", "dnf", "pacman",
        "reboot", "shutdown", "halt", "poweroff",
        "iptables", "ufw", "firewall-cmd",
        "crontab", "at",
    ];

    const WRITE_PROGRAMS: &[&str] = &[
        "mkdir", "cp", "mv", "touch", "sed", "awk", "patch",
        "ln", "install", "rsync", "tee",
    ];

    const GIT_WRITE: &[&str] = &[
        "add", "commit", "push", "pull", "merge", "rebase", "checkout", "switch",
        "fetch", "clone", "init", "reset", "cherry-pick", "am", "apply",
    ];

    const CARGO_WRITE: &[&str] = &["build", "test", "run", "fmt", "install", "publish", "bench"];
    const NPM_WRITE: &[&str] = &["install", "ci", "run", "start", "test", "build", "publish", "exec"];
    const PIP_WRITE: &[&str] = &["install", "uninstall"];
    const GO_WRITE: &[&str] = &["build", "test", "run", "install", "get", "mod"];

    // brew and docker have read subcommands worth carving out so inspection
    // skills (sys-doctor) don't have to admin-prompt for `brew list` /
    // `docker images`.
    const BREW_READ: &[&str] = &["list", "info", "search", "outdated", "deps", "leaves",
                                  "doctor", "config", "--version", "-v", "--prefix", "tap-info"];
    const DOCKER_READ: &[&str] = &["ps", "images", "logs", "inspect", "version", "info",
                                    "system", "history", "port", "top", "stats", "diff", "events"];

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
            if GIT_READ.contains(&subcommand) { BashClass::Read }
            else if GIT_WRITE.contains(&subcommand) { BashClass::Write }
            else { BashClass::Admin }
        }
        "cargo" => {
            if CARGO_READ.contains(&subcommand) { BashClass::Read }
            else if CARGO_WRITE.contains(&subcommand) { BashClass::Write }
            else { BashClass::Admin }
        }
        "npm" | "npx" | "yarn" | "pnpm" => {
            if NPM_READ.contains(&subcommand) { BashClass::Read }
            else if NPM_WRITE.contains(&subcommand) { BashClass::Write }
            else { BashClass::Admin }
        }
        "pip" | "pip3" => {
            if PIP_READ.contains(&subcommand) { BashClass::Read }
            else if PIP_WRITE.contains(&subcommand) { BashClass::Write }
            else { BashClass::Admin }
        }
        "go" => {
            if GO_READ.contains(&subcommand) { BashClass::Read }
            else if GO_WRITE.contains(&subcommand) { BashClass::Write }
            else { BashClass::Admin }
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
            if subcommand == "--spider" { BashClass::Read } else { BashClass::Admin }
        }
        "make" | "cmake" | "ninja" | "mvn" | "gradle" | "pytest" | "jest" | "vitest" => {
            BashClass::Write
        }
        "brew" => {
            if BREW_READ.contains(&subcommand) { BashClass::Read } else { BashClass::Admin }
        }
        "docker" | "podman" => {
            if DOCKER_READ.contains(&subcommand) { BashClass::Read } else { BashClass::Admin }
        }
        _ => BashClass::Admin,
    }
}

fn classify_compound_command(cmd: &str) -> BashClass {
    let mut highest = BashClass::Read;
    let segments: Vec<&str> = cmd
        .split(|c: char| c == ';' || c == '|' || c == '&')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    for segment in segments {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let program = tokens[0];
        let sub = tokens.get(1).copied().unwrap_or("");
        let class = classify_single_command(program, sub);
        if class > highest {
            highest = class.clone();
        }
        if highest == BashClass::Admin {
            return BashClass::Admin;
        }
    }
    highest
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
        if let Some(name) = p.file_name() {
            suffix.push(name.to_os_string());
        }
        if parent.as_os_str().is_empty() {
            break;
        }
        if let Ok(c) = parent.canonicalize() {
            let mut out = c;
            for n in suffix.iter().rev() {
                out.push(n);
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
///    Write, Edit, Bash, Glob, Grep, capture_screenshot, Task, Skill,
///    RunApp, lock_paths, unlock_paths, WebSearch, WebFetch, AskUser,
///    Memory_query, Memory_write.
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

    // 1a. Chat-tier tools (Memory_query, Memory_write — see capabilities.rs)
    // are at the floor. Nothing exceeds Chat, and they don't touch the
    // workspace — they hit the user's own daemon-backed memory store, not
    // any path in `session_cwd`. Path-gating them produces a bogus
    // "Switch this folder to chat" ExceedsCeiling prompt when path_modes
    // is empty. Allow unconditionally.
    if action_tier == PermissionMode::Chat {
        return PermissionCheckResult::Allowed;
    }

    // 2. Bash-specific path gating: if the command contains explicit absolute
    // or tilde-prefixed path args, each must be covered at action_tier.
    // Without this, `bash ls /B` from a session with read-on-/A would pass
    // because step 3 only consulted cwd's tier — but the command's actual
    // reach is /B, not /A. cwd is just where the shell starts; we gate on
    // what the command actually touches.
    if tool == "Bash" {
        if let Some(cmd) = bash_command {
            let path_args = extract_command_paths(cmd);
            if !path_args.is_empty() {
                for path_arg in &path_args {
                    let arg_path = expand_path_arg(path_arg);
                    // OS temp dir is always-available scratch — don't gate it.
                    if is_under_temp(&arg_path) {
                        continue;
                    }
                    let mode = effective_mode_for_path(&session_perms.path_modes, &arg_path);
                    if mode.map_or(true, |m| action_tier > m) {
                        let grant_path = grant_path_for_prompt(tool, &arg_path, session_cwd);
                        let path_str = display_path(&grant_path);
                        return PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling {
                            target_mode: action_tier,
                            path: path_str,
                            tool_summary: format!("{} {}", tool, cmd),
                        });
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
                if p.is_absolute() { p } else { session_cwd.join(p) }
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
    let grant_path = grant_path_for_prompt(tool, &target_path, session_cwd);
    let path_str = display_path(&grant_path);
    let rule_arg = bash_command.or(file_path);
    let summary = rule_arg.unwrap_or(tool).to_string();

    PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling {
        target_mode: action_tier,
        path: path_str,
        tool_summary: format!("{} {}", tool, summary),
    })
}

// ---------------------------------------------------------------------------
// Helpers reused by check_permission and prompt construction
// ---------------------------------------------------------------------------

/// Extract absolute (`/foo/bar`) and tilde-prefixed (`~`, `~/foo`) tokens from
/// a bash command. Used to gate Bash by the paths it actually touches, not
/// just the session cwd's tier. Best-effort: doesn't handle quoted paths with
/// spaces, embedded `--flag=/path` forms, or command substitution. Catches the
/// common `cmd /path` and `cmd ~/path` forms.
pub(super) fn extract_command_paths(cmd: &str) -> Vec<String> {
    shell_words(cmd)
        .into_iter()
        // A fully-quoted word is a single string argument — unwrap it so a
        // quoted path ARG (`cat "/etc/x"`) still counts, while a quoted
        // string that merely CONTAINS path-like words (a `ling-mem search
        // "... /usr/local/bin ..."` query, a jq program) stays one word
        // and is not mistaken for a path the command touches.
        .map(|w| {
            let t = w.trim();
            let quoted = t.len() >= 2
                && ((t.starts_with('"') && t.ends_with('"'))
                    || (t.starts_with('\'') && t.ends_with('\'')));
            if quoted { t[1..t.len() - 1].to_string() } else { t.to_string() }
        })
        .filter(|t| t.starts_with('/') || t.starts_with("~/") || t == "~")
        // Strip trailing punctuation from compound shell syntax.
        .map(|t| t.trim_end_matches(';').trim_end_matches('&').trim_end_matches('|').to_string())
        .filter(|t| !t.is_empty())
        // Reject all-slash tokens (`/`, `//`, `///`) — e.g. an unquoted jq
        // `//` operator — which would otherwise demand admin on `//`.
        .filter(|t| !t.trim_matches('/').is_empty())
        .collect()
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

/// Compute the path to offer in the "Switch this folder to {mode}" option.
fn grant_path_for_prompt(tool: &str, target: &Path, session_cwd: &Path) -> PathBuf {
    if matches!(tool, "Grep" | "Glob" | "Task") {
        return target.to_path_buf();
    }
    if target.starts_with(session_cwd) {
        session_cwd.to_path_buf()
    } else if target.is_dir() {
        target.to_path_buf()
    } else {
        target.parent().map(Path::to_path_buf).unwrap_or_else(|| target.to_path_buf())
    }
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
