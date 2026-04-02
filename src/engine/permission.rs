use crate::engine::render::normalize_tool_path_arg;
use crate::engine::tools::{AskUserOption, AskUserQuestion};
use globset::Glob;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Permission action returned after prompting the user
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionAction {
    AllowOnce,
    AllowSession,
    Deny,
    /// User denied with a custom message to relay to the model.
    DenyWithMessage(String),
}

// ---------------------------------------------------------------------------
// Persisted format for project-level permissions
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// PermissionStore (legacy stub — kept for AgentEngine field compatibility)
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct PermissionStore;

impl PermissionStore {
    /// Legacy load — returns empty stub. Permissions now in SessionPermissions.
    pub fn load(_project_dir: &Path) -> Self {
        Self
    }
}

// ---------------------------------------------------------------------------
// Bash command pattern helpers
// ---------------------------------------------------------------------------

/// Detects shell operators that chain multiple commands.
/// Compound commands always require explicit approval (no pattern derivation).
pub fn is_compound_command(cmd: &str) -> bool {
    // Check for common shell chaining/substitution operators.
    // We scan character-by-character to avoid false positives inside quotes,
    // but for simplicity we use a heuristic approach on the raw string.
    let bytes = cmd.as_bytes();
    let len = bytes.len();
    for i in 0..len {
        match bytes[i] {
            b';' => return true,
            b'|' => return true, // covers `|` and `||`
            b'&' if i + 1 < len && bytes[i + 1] == b'&' => return true,
            b'`' => return true,
            b'$' if i + 1 < len && bytes[i + 1] == b'(' => return true,
            _ => {}
        }
    }
    false
}

/// Extracts a glob pattern from a simple (non-compound) command.
///
/// - Returns `None` for compound commands (always ask).
/// - `"pwd"` → `"pwd"` (single word = exact match)
/// - `"git status"` → `"git *"` (two words = first token + wildcard)
/// - `"npm run build"` → `"npm run *"` (3+ words = first two tokens + wildcard)
/// - `"ls -la"` → `"ls *"` (second token starts with `-` = first token + wildcard)
pub fn derive_command_pattern(cmd: &str) -> Option<String> {
    // For compound commands, use "first_program *" from the first segment.
    if is_compound_command(cmd) {
        let first_cmd = extract_first_command(cmd);
        let first_token = first_cmd.split_whitespace().next()?;
        return Some(format!("{} *", first_token));
    }
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    match tokens.len() {
        0 => None,
        1 => Some(tokens[0].to_string()),
        2 => {
            // If second token is a flag, use just "program *"
            // Otherwise "program subcommand" → "program *"
            Some(format!("{} *", tokens[0]))
        }
        _ => {
            // 3+ tokens: if second token is a flag or a file path, use "program *"
            // Only treat it as a subcommand if it looks like one (no slashes, no dots prefix, no flags)
            if tokens[1].starts_with('-') || is_path_like(tokens[1]) {
                Some(format!("{} *", tokens[0]))
            } else {
                Some(format!("{} {} *", tokens[0], tokens[1]))
            }
        }
    }
}

/// Returns true if a token looks like a file path rather than a subcommand.
/// Paths typically contain `/`, start with `.` or `~`, or start with `/`.
fn is_path_like(token: &str) -> bool {
    token.contains('/')
        || token.starts_with('.')
        || token.starts_with('~')
}

/// Extract the first simple command from a compound command string.
/// Splits on `|`, `;`, `&&`, `||` and returns the trimmed first segment.
fn extract_first_command(cmd: &str) -> String {
    let bytes = cmd.as_bytes();
    let len = bytes.len();
    for i in 0..len {
        match bytes[i] {
            b';' | b'|' | b'`' => return cmd[..i].trim().to_string(),
            b'&' if i + 1 < len && bytes[i + 1] == b'&' => return cmd[..i].trim().to_string(),
            b'$' if i + 1 < len && bytes[i + 1] == b'(' => return cmd[..i].trim().to_string(),
            _ => {}
        }
    }
    cmd.trim().to_string()
}

/// Matches a command against a stored glob pattern.
/// Falls back to exact string comparison if glob compilation fails.
pub fn command_matches_pattern(cmd: &str, pattern: &str) -> bool {
    match Glob::new(pattern) {
        Ok(glob) => glob.compile_matcher().is_match(cmd),
        Err(_) => cmd == pattern,
    }
}

// ---------------------------------------------------------------------------
// File-scoped permission helpers (Write / Edit)
// ---------------------------------------------------------------------------

/// Extracts a directory-level glob pattern from a relative file path.
///
/// - `"src/components/App.tsx"` → `"src/components/*"`
/// - `"README.md"` (root file)  → `"*"`
/// - `"deep/nested/dir/file.rs"` → `"deep/nested/dir/*"`
pub fn derive_file_pattern(rel_path: &str) -> String {
    // Strip leading slashes to avoid absolute-path patterns in stored rules
    let rel_path = rel_path.trim_start_matches('/');
    if rel_path.is_empty() {
        return "*".to_string();
    }
    match rel_path.rfind('/') {
        Some(idx) => format!("{}/*", &rel_path[..idx]),
        None => "*".to_string(), // root-level file
    }
}

// Old file permission prompt builders removed — replaced by ExceedsCeiling prompt.

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns true for tools that modify the filesystem or execute commands.
pub fn is_destructive_tool(tool: &str) -> bool {
    matches!(tool, "Write" | "Edit" | "Bash")
}

/// Returns true for tools that make network requests (WebFetch, WebSearch).
pub fn is_web_tool(tool: &str) -> bool {
    matches!(tool, "WebFetch" | "WebSearch")
}

/// Build a human-readable summary of what the tool is about to do.
pub fn permission_target_summary(tool: &str, args: &serde_json::Value, cwd: &Path) -> String {
    match tool {
        "Write" | "Edit" => normalize_tool_path_arg(cwd, args)
            .unwrap_or_else(|| "<unknown file>".to_string()),
        "Bash" => args
            .get("cmd")
            .or_else(|| args.get("command"))
            .and_then(|v| v.as_str())
            .map(|cmd| {
                if cmd.len() > 80 {
                    format!("{}...", &cmd[..77])
                } else {
                    cmd.to_string()
                }
            })
            .unwrap_or_else(|| "<unknown command>".to_string()),
        "Patch" => {
            // Try to extract the first file from a unified diff header.
            args.get("diff")
                .or_else(|| args.get("patch"))
                .and_then(|v| v.as_str())
                .and_then(|diff| {
                    diff.lines().find_map(|line| {
                        line.strip_prefix("+++ b/")
                            .or_else(|| line.strip_prefix("+++ "))
                            .map(|s| s.to_string())
                    })
                })
                .unwrap_or_else(|| "<patch>".to_string())
        }
        "WebFetch" => args
            .get("url")
            .and_then(|v| v.as_str())
            .map(|url| {
                if url.len() > 120 {
                    format!("{}...", &url[..117])
                } else {
                    url.to_string()
                }
            })
            .unwrap_or_else(|| "<unknown URL>".to_string()),
        "WebSearch" => args
            .get("query")
            .and_then(|v| v.as_str())
            .map(|q| {
                if q.len() > 120 {
                    format!("{}...", &q[..117])
                } else {
                    q.to_string()
                }
            })
            .unwrap_or_else(|| "<unknown query>".to_string()),
        _ => tool.to_string(),
    }
}

// Old prompt builders (build_permission_question, build_web_permission_question,
// build_bash_permission_question, parse_*) removed — replaced by new check flow
// prompt builders below (build_exceeds_ceiling_question, etc.).

// ===========================================================================
// New permission model (permission-spec.md)
// ===========================================================================

/// Session permission mode — defines the ceiling of what the agent can do.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// Filesystem zone — determines whether mode switching is allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathZone {
    /// User's home directory — mode switching allowed.
    Home,
    /// Temporary directories — mode switching allowed.
    Temp,
    /// System directories — per-action approval only, no mode switch.
    System,
}

/// Bash command classification for permission tier mapping.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum BashClass {
    Read,
    Write,
    Admin,
}

/// Per-session permission state, persisted to permission.json.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SessionPermissions {
    #[serde(default)]
    pub path_modes: Vec<PathMode>,
    #[serde(default)]
    pub locked: bool,
    /// Ask-rule overrides approved by the user this session.
    #[serde(default)]
    pub allows: HashSet<String>,
    /// Tool call signatures the user denied (auto-blocked on retry).
    #[serde(default)]
    pub denied_sigs: HashSet<String>,
}

impl SessionPermissions {
    /// Load from `{session_dir}/permission.json`. Returns default if missing.
    pub fn load(session_dir: &Path) -> Self {
        let file = session_dir.join("permission.json");
        if !file.exists() {
            return Self::default();
        }
        match fs::read_to_string(&file) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(p) => {
                    debug!("Loaded session permissions from {}", file.display());
                    p
                }
                Err(e) => {
                    warn!("Failed to parse session permission.json: {}", e);
                    Self::default()
                }
            },
            Err(e) => {
                warn!("Failed to read session permission.json: {}", e);
                Self::default()
            }
        }
    }

    /// Save to `{session_dir}/permission.json`.
    pub fn save(&self, session_dir: &Path) {
        let file = session_dir.join("permission.json");
        if let Some(parent) = file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = fs::write(&file, json) {
                    warn!("Failed to write session permission.json: {}", e);
                }
            }
            Err(e) => warn!("Failed to serialize session permissions: {}", e),
        }
    }

    /// Add or update a path-mode grant. If a grant for the exact path exists, update it.
    pub fn set_path_mode(&mut self, path: &str, mode: PermissionMode) {
        if let Some(existing) = self.path_modes.iter_mut().find(|pm| pm.path == path) {
            existing.mode = mode;
        } else {
            self.path_modes.push(PathMode {
                path: path.to_string(),
                mode,
            });
        }
    }
}

/// Determine the filesystem zone for a path.
pub fn path_zone(path: &Path) -> PathZone {
    // Normalize to absolute for comparison.
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_default()
            .join(path)
    };

    // Temp zone — include /private/tmp (macOS symlinks /tmp → /private/tmp)
    if path.starts_with("/tmp") || path.starts_with("/private/tmp")
        || path.starts_with("/var/tmp") || path.starts_with("/private/var/tmp")
    {
        return PathZone::Temp;
    }
    #[cfg(windows)]
    {
        if let Ok(temp) = std::env::var("TEMP") {
            if path.starts_with(&temp) {
                return PathZone::Temp;
            }
        }
    }

    // Home zone — but sensitive home paths are treated as System
    if let Some(home) = dirs::home_dir() {
        if path.starts_with(&home) {
            if is_sensitive_home_path_abs(&path, &home) {
                return PathZone::System;
            }
            return PathZone::Home;
        }
    }

    // Everything else is System
    PathZone::System
}

/// Check if a path under home is sensitive (credentials, config).
fn is_sensitive_home_path_abs(path: &Path, home: &Path) -> bool {
    let sensitive = [".ssh", ".gnupg", ".aws", ".azure", ".gcloud"];
    for dir in &sensitive {
        if path.starts_with(home.join(dir)) {
            return true;
        }
    }
    // .git/ and .linggen/ internals (at any nesting level)
    for component in path.components() {
        let s = component.as_os_str().to_string_lossy();
        if s == ".git" || s == ".linggen" {
            return true;
        }
    }
    false
}

/// Public wrapper for sensitive path check.
pub fn is_sensitive_home_path(path: &Path) -> bool {
    if let Some(home) = dirs::home_dir() {
        if path.starts_with(&home) {
            return is_sensitive_home_path_abs(path, &home);
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Bash command classifier
// ---------------------------------------------------------------------------

/// Check if a command contains output redirection (`>` or `>>`).
/// Excludes `->` (arrow) and `>=` (comparison).
fn has_output_redirect(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'>' {
            // Exclude `->`
            if i > 0 && bytes[i - 1] == b'-' {
                continue;
            }
            // Exclude `>=`
            if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
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
        // Classify each segment, return highest
        return classify_compound_command(cmd);
    }

    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    if tokens.is_empty() {
        return BashClass::Admin; // empty = unknown = admin
    }

    let program = tokens[0];
    let subcommand = tokens.get(1).copied().unwrap_or("");

    // Check for output redirection → at least write.
    // Match `>` and `>>` with or without surrounding spaces (shell doesn't require them).
    // Exclude `->` (arrow operator) and `>=` (comparison).
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
    // Read-class programs
    const READ_PROGRAMS: &[&str] = &[
        "ls", "cat", "head", "tail", "less", "more", "wc", "file", "stat", "du", "df",
        "pwd", "env", "printenv", "echo", "printf", "which", "whereis", "type",
        "find", "grep", "rg", "ag", "ack", "fd", "tree", "bat", "jq", "yq",
        "uname", "hostname", "date", "id", "whoami", "realpath", "dirname", "basename",
        "ping", "dig", "nslookup", "host", "test", "true", "false", "seq", "sort",
        "uniq", "tr", "cut", "paste", "diff", "comm",
    ];

    // Read-class git subcommands
    const GIT_READ: &[&str] = &[
        "status", "log", "diff", "show", "branch", "tag", "remote", "rev-parse",
        "blame", "stash", "describe", "shortlog", "ls-files", "ls-tree",
    ];

    // Read-class cargo/npm/pip/go subcommands
    const CARGO_READ: &[&str] = &["check", "clippy", "doc", "metadata", "tree", "verify-project"];
    const NPM_READ: &[&str] = &["list", "ls", "outdated", "view", "info", "audit", "why", "explain"];
    const PIP_READ: &[&str] = &["list", "show", "freeze", "check"];
    const GO_READ: &[&str] = &["vet", "list", "doc", "env", "version"];

    // Admin-class programs (always dangerous)
    const ADMIN_PROGRAMS: &[&str] = &[
        "rm", "sudo", "su", "kill", "killall", "pkill",
        "chmod", "chown", "chgrp",
        "docker", "podman", "systemctl", "launchctl", "service",
        "mount", "umount", "mkfs", "fdisk", "dd",
        "apt", "apt-get", "yum", "dnf", "pacman", "brew",
        "reboot", "shutdown", "halt", "poweroff",
        "iptables", "ufw", "firewall-cmd",
        "crontab", "at",
    ];

    // Write-class programs
    const WRITE_PROGRAMS: &[&str] = &[
        "mkdir", "cp", "mv", "touch", "sed", "awk", "patch",
        "ln", "install", "rsync", "tee",
    ];

    // Write-class git subcommands
    const GIT_WRITE: &[&str] = &[
        "add", "commit", "push", "pull", "merge", "rebase", "checkout", "switch",
        "fetch", "clone", "init", "reset", "cherry-pick", "am", "apply",
    ];

    // Write-class build/package subcommands
    const CARGO_WRITE: &[&str] = &["build", "test", "run", "fmt", "install", "publish", "bench"];
    const NPM_WRITE: &[&str] = &["install", "ci", "run", "start", "test", "build", "publish", "exec"];
    const PIP_WRITE: &[&str] = &["install", "uninstall"];
    const GO_WRITE: &[&str] = &["build", "test", "run", "install", "get", "mod"];

    // Check admin first (highest priority)
    if ADMIN_PROGRAMS.contains(&program) {
        return BashClass::Admin;
    }

    // Check read programs
    if READ_PROGRAMS.contains(&program) {
        return BashClass::Read;
    }

    // Check write programs
    if WRITE_PROGRAMS.contains(&program) {
        return BashClass::Write;
    }

    // Handle multi-token commands (git, cargo, npm, pip, go, python, node)
    match program {
        "git" => {
            if GIT_READ.contains(&subcommand) {
                BashClass::Read
            } else if GIT_WRITE.contains(&subcommand) {
                BashClass::Write
            } else {
                BashClass::Admin // unknown git subcommand
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
            // --version, --help are read; everything else is admin
            if subcommand == "--version" || subcommand == "--help" || subcommand == "-V" {
                BashClass::Read
            } else {
                BashClass::Admin
            }
        }
        "curl" => {
            // curl -I (HEAD) or --head is read; otherwise admin
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
            BashClass::Write // build/test tools
        }
        _ => BashClass::Admin, // unknown → admin
    }
}

/// Classify a compound command by the highest-tier component.
fn classify_compound_command(cmd: &str) -> BashClass {
    let mut highest = BashClass::Read;
    // Split on common shell operators
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
            return BashClass::Admin; // short-circuit
        }
    }
    highest
}

// ---------------------------------------------------------------------------
// Effective mode lookup
// ---------------------------------------------------------------------------

/// Find the effective permission mode for a target path by checking path_modes.
/// Returns the mode from the most specific (longest) matching path.
/// Returns `None` if no grant covers the target path.
pub fn effective_mode_for_path(path_modes: &[PathMode], target: &Path) -> Option<PermissionMode> {
    let target_str = target.to_string_lossy();
    let mut best: Option<(&PathMode, usize)> = None;

    for pm in path_modes {
        // Expand ~ to home dir for comparison
        let grant_path = if pm.path.starts_with("~/") {
            if let Some(home) = dirs::home_dir() {
                home.join(&pm.path[2..]).to_string_lossy().to_string()
            } else {
                pm.path.clone()
            }
        } else if pm.path == "~" {
            dirs::home_dir()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|| pm.path.clone())
        } else {
            pm.path.clone()
        };

        // Check if target starts with the grant path (grant covers children)
        if target_str.starts_with(&grant_path)
            && (target_str.len() == grant_path.len()
                || target_str.as_bytes().get(grant_path.len()) == Some(&b'/'))
        {
            let specificity = grant_path.len();
            if best.is_none() || specificity > best.unwrap().1 {
                best = Some((pm, specificity));
            }
        }
    }

    best.map(|(pm, _)| pm.mode.clone())
}

// ---------------------------------------------------------------------------
// Action tier for non-Bash tools
// ---------------------------------------------------------------------------

/// Map a tool name to its permission mode requirement.
/// For Bash, use `classify_bash_command` instead.
pub fn tool_action_tier(tool: &str) -> PermissionMode {
    match tool {
        "Read" | "Glob" | "Grep" | "WebSearch" | "capture_screenshot"
        | "EnterPlanMode" | "ExitPlanMode" | "UpdatePlan" | "AskUser" => PermissionMode::Read,
        "Write" | "Edit" => PermissionMode::Edit,
        // Everything else: Bash, WebFetch, RunApp, Task, Skill, lock_paths, unlock_paths
        _ => PermissionMode::Admin,
    }
}

// ---------------------------------------------------------------------------
// New permission check flow (permission-spec.md)
// ---------------------------------------------------------------------------

/// Result of a permission check.
#[derive(Debug)]
pub enum PermissionCheckResult {
    /// Action is allowed — proceed without prompting.
    Allowed,
    /// Action is hard-blocked (deny rule, locked session, etc.).
    Blocked(String),
    /// Action needs user approval — show a prompt.
    NeedsPrompt(PromptKind),
}

/// What kind of prompt to show the user.
#[derive(Debug)]
pub enum PromptKind {
    /// Action exceeds the mode ceiling on a home/temp path.
    /// Offer: Allow once / Switch to {target_mode} mode / Deny / Other
    ExceedsCeiling {
        target_mode: PermissionMode,
        path: String,
        tool_summary: String,
    },
    /// Write/edit in system zone — per-action only, no mode switch.
    /// Offer: Allow once / Deny
    SystemZoneWrite {
        tool_summary: String,
    },
    /// Config `ask` rule forces a prompt even within ceiling.
    /// Offer: Allow once / Allow for session / Deny
    AskRuleOverride {
        rule: String,
        tool_summary: String,
    },
    /// Read outside any granted path.
    /// Offer: Allow read on {dir} / Allow once / Deny
    ReadOutsidePath {
        dir: String,
        tool_summary: String,
    },
}

/// Parse a tool rule like `"Bash(sudo *)"` into `("Bash", "sudo *")`.
pub fn parse_tool_rule(rule: &str) -> Option<(String, String)> {
    let open = rule.find('(')?;
    let close = rule.rfind(')')?;
    if close <= open {
        return None;
    }
    let tool = rule[..open].trim().to_string();
    let pattern = rule[open + 1..close].trim().to_string();
    if tool.is_empty() || pattern.is_empty() {
        return None;
    }
    Some((tool, pattern))
}

/// Check if a tool call matches any rule in a list.
/// Rules are `Tool(pattern)` format, e.g. `"Bash(sudo *)"`.
fn matches_rules(rules: &[String], tool: &str, arg: Option<&str>) -> Option<String> {
    for rule in rules {
        if let Some((rule_tool, rule_pattern)) = parse_tool_rule(rule) {
            if rule_tool == tool {
                if let Some(a) = arg {
                    if command_matches_pattern(a, &rule_pattern) {
                        return Some(rule.clone());
                    }
                } else {
                    // No arg — blanket tool match if pattern is "*"
                    if rule_pattern == "*" {
                        return Some(rule.clone());
                    }
                }
            }
        }
    }
    None
}

/// The main permission check for the new model.
///
/// Returns whether the action is allowed, blocked, or needs a prompt.
/// This does NOT handle the actual prompting — the caller does that.
pub fn check_permission(
    tool: &str,
    bash_command: Option<&str>,
    file_path: Option<&str>,
    session_cwd: &Path,
    session_perms: &SessionPermissions,
    deny_rules: &[String],
    ask_rules: &[String],
) -> PermissionCheckResult {
    // 0. Chat mode = no tools at all, hard-block everything
    let target_path_for_chat = file_path
        .map(PathBuf::from)
        .unwrap_or_else(|| session_cwd.to_path_buf());
    if let Some(PermissionMode::Chat) = effective_mode_for_path(&session_perms.path_modes, &target_path_for_chat) {
        return PermissionCheckResult::Blocked("Chat mode: no tools available".to_string());
    }

    // 1. Classify action tier
    let action_tier = if tool == "Bash" {
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

    // Determine the argument for rule matching
    let rule_arg = bash_command.or(file_path);

    // 2. Check deny rules (config)
    if let Some(rule) = matches_rules(deny_rules, tool, rule_arg) {
        return PermissionCheckResult::Blocked(format!("Denied by rule: {}", rule));
    }

    // 3. Check ask rules (config) — but skip if user already allowed for session
    if let Some(rule) = matches_rules(ask_rules, tool, rule_arg) {
        // Check if user already overrode this ask rule for the session.
        // Store the rule itself as the key so suppression covers all matching commands.
        if !session_perms.allows.contains(&rule) {
            let summary = rule_arg.unwrap_or(tool).to_string();
            return PermissionCheckResult::NeedsPrompt(PromptKind::AskRuleOverride {
                rule,
                tool_summary: format!("{} {}", tool, summary),
            });
        }
    }

    // 4. Resolve target path + zone
    let target_path = file_path
        .map(PathBuf::from)
        .unwrap_or_else(|| session_cwd.to_path_buf());
    let zone = path_zone(&target_path);

    // 5. System zone + write/edit/admin → per-action only
    if zone == PathZone::System && action_tier > PermissionMode::Read {
        if session_perms.locked {
            return PermissionCheckResult::Blocked(
                "System zone write blocked (locked session)".to_string(),
            );
        }
        let summary = rule_arg.unwrap_or(tool).to_string();
        return PermissionCheckResult::NeedsPrompt(PromptKind::SystemZoneWrite {
            tool_summary: format!("{} {}", tool, summary),
        });
    }

    // 6. Find effective mode for target path
    let effective_mode = effective_mode_for_path(&session_perms.path_modes, &target_path);

    match effective_mode {
        Some(ref mode) if action_tier <= *mode => {
            // Within ceiling → allowed
            PermissionCheckResult::Allowed
        }
        Some(_) | None => {
            // Exceeds ceiling or no grant
            if session_perms.locked {
                return PermissionCheckResult::Blocked(format!(
                    "Action requires {} mode but session is locked",
                    action_tier,
                ));
            }

            // For reads outside any granted path, offer path grant
            if action_tier == PermissionMode::Read && effective_mode.is_none() {
                let dir = target_path
                    .parent()
                    .unwrap_or(&target_path)
                    .to_string_lossy()
                    .to_string();
                let summary = rule_arg.unwrap_or(tool).to_string();
                return PermissionCheckResult::NeedsPrompt(PromptKind::ReadOutsidePath {
                    dir,
                    tool_summary: format!("{} {}", tool, summary),
                });
            }

            // For write/admin actions, offer mode upgrade
            let target_mode = action_tier.clone();
            let path_str = if let Some(home) = dirs::home_dir() {
                let ts = target_path.to_string_lossy();
                let hs = home.to_string_lossy();
                if ts.starts_with(hs.as_ref()) {
                    format!("~{}", &ts[hs.len()..])
                } else {
                    ts.to_string()
                }
            } else {
                target_path.to_string_lossy().to_string()
            };
            let summary = rule_arg.unwrap_or(tool).to_string();
            PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling {
                target_mode,
                path: path_str,
                tool_summary: format!("{} {}", tool, summary),
            })
        }
    }
}

/// Build AskUser question for an ExceedsCeiling prompt.
pub fn build_exceeds_ceiling_question(
    tool_summary: &str,
    target_mode: &PermissionMode,
    path: &str,
) -> AskUserQuestion {
    AskUserQuestion {
        question: tool_summary.to_string(),
        header: "Permission".to_string(),
        options: vec![
            AskUserOption {
                label: "Allow once".to_string(),
                description: Some("One-time approval, mode stays the same".to_string()),
                preview: None,
            },
            AskUserOption {
                label: format!("Switch to {} mode", target_mode),
                description: Some(format!("Grants {} on {} and children", target_mode, path)),
                preview: None,
            },
            AskUserOption {
                label: "Deny".to_string(),
                description: Some("Block this action".to_string()),
                preview: None,
            },
        ],
        multi_select: false,
    }
}

/// Build AskUser question for a SystemZoneWrite prompt.
pub fn build_system_zone_question(tool_summary: &str) -> AskUserQuestion {
    AskUserQuestion {
        question: tool_summary.to_string(),
        header: "Permission".to_string(),
        options: vec![
            AskUserOption {
                label: "Allow once".to_string(),
                description: Some("One-time approval for this system path".to_string()),
                preview: None,
            },
            AskUserOption {
                label: "Deny".to_string(),
                description: Some("Block this action".to_string()),
                preview: None,
            },
        ],
        multi_select: false,
    }
}

/// Build AskUser question for an AskRuleOverride prompt.
pub fn build_ask_rule_question(tool_summary: &str, rule: &str) -> AskUserQuestion {
    AskUserQuestion {
        question: tool_summary.to_string(),
        header: "Permission".to_string(),
        options: vec![
            AskUserOption {
                label: "Allow once".to_string(),
                description: Some("Proceed this one time".to_string()),
                preview: None,
            },
            AskUserOption {
                label: "Allow for this session".to_string(),
                description: Some(format!("Suppress '{}' for this session", rule)),
                preview: None,
            },
            AskUserOption {
                label: "Deny".to_string(),
                description: Some("Block this action".to_string()),
                preview: None,
            },
        ],
        multi_select: false,
    }
}

/// Build AskUser question for a ReadOutsidePath prompt.
pub fn build_read_outside_path_question(tool_summary: &str, dir: &str) -> AskUserQuestion {
    AskUserQuestion {
        question: tool_summary.to_string(),
        header: "Permission".to_string(),
        options: vec![
            AskUserOption {
                label: "Allow once".to_string(),
                description: Some("One-time read".to_string()),
                preview: None,
            },
            AskUserOption {
                label: format!("Allow read on {}", dir),
                description: Some("Grants read for this directory tree this session".to_string()),
                preview: None,
            },
            AskUserOption {
                label: "Deny".to_string(),
                description: Some("Block this read".to_string()),
                preview: None,
            },
        ],
        multi_select: false,
    }
}

/// Parse user response to an ExceedsCeiling prompt.
pub fn parse_exceeds_ceiling_answer(
    selected: &str,
    target_mode: &PermissionMode,
) -> PermissionAction {
    if selected == "Allow once" {
        PermissionAction::AllowOnce
    } else if selected == format!("Switch to {} mode", target_mode) {
        PermissionAction::AllowSession // caller interprets as mode switch
    } else {
        PermissionAction::Deny
    }
}

/// Parse user response to a ReadOutsidePath prompt.
pub fn parse_read_outside_path_answer(selected: &str, dir: &str) -> PermissionAction {
    if selected == "Allow once" {
        PermissionAction::AllowOnce
    } else if selected == format!("Allow read on {}", dir) {
        PermissionAction::AllowSession // caller interprets as read grant on dir
    } else {
        PermissionAction::Deny
    }
}

/// Parse user response to an AskRuleOverride prompt.
pub fn parse_ask_rule_answer(selected: &str) -> PermissionAction {
    if selected == "Allow once" {
        PermissionAction::AllowOnce
    } else if selected == "Allow for this session" {
        PermissionAction::AllowSession
    } else {
        PermissionAction::Deny
    }
}

/// Parse user response to a SystemZoneWrite prompt.
pub fn parse_system_zone_answer(selected: &str) -> PermissionAction {
    if selected == "Allow once" {
        PermissionAction::AllowOnce
    } else {
        PermissionAction::Deny
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Utility tests (kept)
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_destructive() {
        assert!(is_destructive_tool("Write"));
        assert!(is_destructive_tool("Edit"));
        assert!(is_destructive_tool("Bash"));
        assert!(!is_destructive_tool("Read"));
    }

    #[test]
    fn test_is_web_tool() {
        assert!(is_web_tool("WebFetch"));
        assert!(is_web_tool("WebSearch"));
        assert!(!is_web_tool("Bash"));
    }

    #[test]
    fn test_is_compound_command() {
        assert!(is_compound_command("ls; rm -rf /"));
        assert!(is_compound_command("echo foo && echo bar"));
        assert!(is_compound_command("ls | grep foo"));
        assert!(!is_compound_command("npm run build"));
    }

    #[test]
    fn test_derive_command_pattern() {
        assert_eq!(derive_command_pattern("pwd"), Some("pwd".to_string()));
        assert_eq!(derive_command_pattern("git status"), Some("git *".to_string()));
        assert_eq!(derive_command_pattern("npm run build"), Some("npm run *".to_string()));
        assert_eq!(derive_command_pattern("ls -la"), Some("ls *".to_string()));
    }

    #[test]
    fn test_command_matches_pattern() {
        assert!(command_matches_pattern("npm run build", "npm run *"));
        assert!(!command_matches_pattern("cargo build", "npm run *"));
        assert!(command_matches_pattern("git status", "git *"));
        assert!(command_matches_pattern("pwd", "pwd"));
    }

    #[test]
    fn test_derive_file_pattern() {
        assert_eq!(derive_file_pattern("src/components/App.tsx"), "src/components/*");
        assert_eq!(derive_file_pattern("README.md"), "*");
    }

    #[test]
    fn test_permission_target_summary_bash() {
        let args = serde_json::json!({ "cmd": "cargo build" });
        assert_eq!(permission_target_summary("Bash", &args, Path::new("/tmp")), "cargo build");
    }

    // -----------------------------------------------------------------------
    // New permission model tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_permission_mode_ordering() {
        assert!(PermissionMode::Chat < PermissionMode::Read);
        assert!(PermissionMode::Read < PermissionMode::Edit);
        assert!(PermissionMode::Edit < PermissionMode::Admin);
    }

    #[test]
    fn test_permission_mode_serde() {
        let json = serde_json::to_string(&PermissionMode::Edit).unwrap();
        assert_eq!(json, "\"edit\"");
        let mode: PermissionMode = serde_json::from_str("\"admin\"").unwrap();
        assert_eq!(mode, PermissionMode::Admin);
    }

    #[test]
    fn test_session_permissions_serde_roundtrip() {
        let mut sp = SessionPermissions::default();
        sp.path_modes.push(PathMode {
            path: "~/workspace/linggen".to_string(),
            mode: PermissionMode::Edit,
        });
        sp.allows.insert("Bash:git push *".to_string());
        sp.denied_sigs.insert("Bash:rm -rf dist".to_string());

        let json = serde_json::to_string_pretty(&sp).unwrap();
        let loaded: SessionPermissions = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.path_modes.len(), 1);
        assert_eq!(loaded.path_modes[0].mode, PermissionMode::Edit);
        assert!(loaded.allows.contains("Bash:git push *"));
        assert!(loaded.denied_sigs.contains("Bash:rm -rf dist"));
        assert!(!loaded.locked);
    }

    #[test]
    fn test_session_permissions_load_save() {
        let tmp = std::env::temp_dir().join("linggen_session_perm_test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let mut sp = SessionPermissions::default();
        sp.set_path_mode("~/workspace", PermissionMode::Edit);
        sp.locked = true;
        sp.save(&tmp);

        let loaded = SessionPermissions::load(&tmp);
        assert_eq!(loaded.path_modes.len(), 1);
        assert_eq!(loaded.path_modes[0].path, "~/workspace");
        assert_eq!(loaded.path_modes[0].mode, PermissionMode::Edit);
        assert!(loaded.locked);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_session_permissions_set_path_mode_updates() {
        let mut sp = SessionPermissions::default();
        sp.set_path_mode("~/workspace", PermissionMode::Read);
        assert_eq!(sp.path_modes.len(), 1);
        assert_eq!(sp.path_modes[0].mode, PermissionMode::Read);

        // Update existing path
        sp.set_path_mode("~/workspace", PermissionMode::Admin);
        assert_eq!(sp.path_modes.len(), 1); // no duplicate
        assert_eq!(sp.path_modes[0].mode, PermissionMode::Admin);

        // Add different path
        sp.set_path_mode("~/other", PermissionMode::Edit);
        assert_eq!(sp.path_modes.len(), 2);
    }

    #[test]
    fn test_path_zone_home() {
        if let Some(home) = dirs::home_dir() {
            assert_eq!(path_zone(&home.join("workspace")), PathZone::Home);
            assert_eq!(path_zone(&home.join("Documents/file.txt")), PathZone::Home);
        }
    }

    #[test]
    fn test_path_zone_temp() {
        assert_eq!(path_zone(Path::new("/tmp")), PathZone::Temp);
        assert_eq!(path_zone(Path::new("/tmp/build")), PathZone::Temp);
        assert_eq!(path_zone(Path::new("/var/tmp/scratch")), PathZone::Temp);
        // macOS: /tmp is symlinked to /private/tmp
        assert_eq!(path_zone(Path::new("/private/tmp")), PathZone::Temp);
        assert_eq!(path_zone(Path::new("/private/tmp/arcadegame")), PathZone::Temp);
    }

    #[test]
    fn test_path_zone_system() {
        assert_eq!(path_zone(Path::new("/etc/hosts")), PathZone::System);
        assert_eq!(path_zone(Path::new("/usr/bin/ls")), PathZone::System);
        assert_eq!(path_zone(Path::new("/bin/sh")), PathZone::System);
    }

    #[test]
    fn test_path_zone_sensitive_home() {
        if let Some(home) = dirs::home_dir() {
            // Sensitive home paths are classified as System
            assert_eq!(path_zone(&home.join(".ssh/id_rsa")), PathZone::System);
            assert_eq!(path_zone(&home.join(".aws/credentials")), PathZone::System);
            assert_eq!(path_zone(&home.join(".gnupg/pubring.gpg")), PathZone::System);
        }
    }

    #[test]
    fn test_is_sensitive_home_path() {
        if let Some(home) = dirs::home_dir() {
            assert!(is_sensitive_home_path(&home.join(".ssh/id_rsa")));
            assert!(is_sensitive_home_path(&home.join(".aws/config")));
            assert!(!is_sensitive_home_path(&home.join("workspace/src/main.rs")));
        }
    }

    #[test]
    fn test_classify_bash_read() {
        assert_eq!(classify_bash_command("ls"), BashClass::Read);
        assert_eq!(classify_bash_command("ls -la"), BashClass::Read);
        assert_eq!(classify_bash_command("cat foo.txt"), BashClass::Read);
        assert_eq!(classify_bash_command("pwd"), BashClass::Read);
        assert_eq!(classify_bash_command("git status"), BashClass::Read);
        assert_eq!(classify_bash_command("git log --oneline"), BashClass::Read);
        assert_eq!(classify_bash_command("git diff"), BashClass::Read);
        assert_eq!(classify_bash_command("cargo check"), BashClass::Read);
        assert_eq!(classify_bash_command("npm list"), BashClass::Read);
        assert_eq!(classify_bash_command("grep foo bar.txt"), BashClass::Read);
        assert_eq!(classify_bash_command("find . -name '*.rs'"), BashClass::Read);
        assert_eq!(classify_bash_command("python --version"), BashClass::Read);
        assert_eq!(classify_bash_command("curl -I https://example.com"), BashClass::Read);
    }

    #[test]
    fn test_classify_bash_write() {
        assert_eq!(classify_bash_command("mkdir -p src/new"), BashClass::Write);
        assert_eq!(classify_bash_command("cp foo.txt bar.txt"), BashClass::Write);
        assert_eq!(classify_bash_command("mv old.rs new.rs"), BashClass::Write);
        assert_eq!(classify_bash_command("git add ."), BashClass::Write);
        assert_eq!(classify_bash_command("git commit -m 'fix'"), BashClass::Write);
        assert_eq!(classify_bash_command("git push origin main"), BashClass::Write);
        assert_eq!(classify_bash_command("npm install"), BashClass::Write);
        assert_eq!(classify_bash_command("npm run build"), BashClass::Write);
        assert_eq!(classify_bash_command("cargo build"), BashClass::Write);
        assert_eq!(classify_bash_command("cargo test"), BashClass::Write);
        assert_eq!(classify_bash_command("make"), BashClass::Write);
    }

    #[test]
    fn test_classify_bash_admin() {
        assert_eq!(classify_bash_command("rm -rf dist"), BashClass::Admin);
        assert_eq!(classify_bash_command("sudo apt install foo"), BashClass::Admin);
        assert_eq!(classify_bash_command("chmod 755 script.sh"), BashClass::Admin);
        assert_eq!(classify_bash_command("docker run nginx"), BashClass::Admin);
        assert_eq!(classify_bash_command("kill -9 1234"), BashClass::Admin);
        assert_eq!(classify_bash_command("unknown_program --flag"), BashClass::Admin);
        assert_eq!(classify_bash_command("curl https://example.com"), BashClass::Admin);
    }

    #[test]
    fn test_classify_bash_compound() {
        // Highest tier wins
        assert_eq!(classify_bash_command("ls && rm foo"), BashClass::Admin);
        assert_eq!(classify_bash_command("ls | grep foo"), BashClass::Read);
        assert_eq!(classify_bash_command("mkdir dir && cp a b"), BashClass::Write);
        assert_eq!(classify_bash_command("git status; git add ."), BashClass::Write);
    }

    #[test]
    fn test_classify_bash_redirect() {
        // Output redirection promotes read to write — with and without spaces
        assert_eq!(classify_bash_command("echo hello > out.txt"), BashClass::Write);
        assert_eq!(classify_bash_command("ls > files.txt"), BashClass::Write);
        assert_eq!(classify_bash_command("cat /etc/passwd>leak.txt"), BashClass::Write);
        assert_eq!(classify_bash_command("echo foo>>append.txt"), BashClass::Write);
    }

    #[test]
    fn test_check_permission_chat_mode_blocks_all() {
        let mut sp = SessionPermissions::default();
        sp.set_path_mode("~/workspace", PermissionMode::Chat);

        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("workspace");
            let result = check_permission(
                "Read", None,
                Some(home.join("workspace/file.txt").to_str().unwrap()),
                &cwd, &sp, &[], &[],
            );
            assert!(matches!(result, PermissionCheckResult::Blocked(_)));
        }
    }

    #[test]
    fn test_effective_mode_for_path_basic() {
        let modes = vec![
            PathMode { path: "~/workspace/linggen".to_string(), mode: PermissionMode::Edit },
            PathMode { path: "~/workspace/other".to_string(), mode: PermissionMode::Read },
        ];

        if let Some(home) = dirs::home_dir() {
            let result = effective_mode_for_path(
                &modes,
                &home.join("workspace/linggen/src/main.rs"),
            );
            assert_eq!(result, Some(PermissionMode::Edit));

            let result = effective_mode_for_path(
                &modes,
                &home.join("workspace/other/README.md"),
            );
            assert_eq!(result, Some(PermissionMode::Read));

            // No grant for this path
            let result = effective_mode_for_path(
                &modes,
                &home.join("Documents/notes.txt"),
            );
            assert_eq!(result, None);
        }
    }

    #[test]
    fn test_effective_mode_most_specific_wins() {
        let modes = vec![
            PathMode { path: "~/workspace".to_string(), mode: PermissionMode::Read },
            PathMode { path: "~/workspace/linggen".to_string(), mode: PermissionMode::Admin },
        ];

        if let Some(home) = dirs::home_dir() {
            // Most specific path wins
            let result = effective_mode_for_path(
                &modes,
                &home.join("workspace/linggen/src/main.rs"),
            );
            assert_eq!(result, Some(PermissionMode::Admin));

            // Parent path applies to sibling
            let result = effective_mode_for_path(
                &modes,
                &home.join("workspace/other/file.txt"),
            );
            assert_eq!(result, Some(PermissionMode::Read));
        }
    }

    #[test]
    fn test_tool_action_tier() {
        assert_eq!(tool_action_tier("Read"), PermissionMode::Read);
        assert_eq!(tool_action_tier("Glob"), PermissionMode::Read);
        assert_eq!(tool_action_tier("Grep"), PermissionMode::Read);
        assert_eq!(tool_action_tier("Write"), PermissionMode::Edit);
        assert_eq!(tool_action_tier("Edit"), PermissionMode::Edit);
        assert_eq!(tool_action_tier("Bash"), PermissionMode::Admin);
        assert_eq!(tool_action_tier("WebFetch"), PermissionMode::Admin);
        assert_eq!(tool_action_tier("Task"), PermissionMode::Admin);
        assert_eq!(tool_action_tier("Skill"), PermissionMode::Admin);
    }

    // -----------------------------------------------------------------------
    // Check flow tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_tool_rule() {
        assert_eq!(
            parse_tool_rule("Bash(sudo *)"),
            Some(("Bash".to_string(), "sudo *".to_string()))
        );
        assert_eq!(
            parse_tool_rule("Edit(src/*)"),
            Some(("Edit".to_string(), "src/*".to_string()))
        );
        assert_eq!(
            parse_tool_rule("WebFetch(domain:github.com)"),
            Some(("WebFetch".to_string(), "domain:github.com".to_string()))
        );
        assert_eq!(parse_tool_rule("invalid"), None);
        assert_eq!(parse_tool_rule("()"), None);
    }

    #[test]
    fn test_check_permission_deny_rule() {
        let sp = SessionPermissions::default();
        let deny = vec!["Bash(sudo *)".to_string()];
        let ask: Vec<String> = vec![];
        let cwd = dirs::home_dir().unwrap_or_default();

        let result = check_permission("Bash", Some("sudo apt install foo"), None, &cwd, &sp, &deny, &ask);
        assert!(matches!(result, PermissionCheckResult::Blocked(_)));

        // Non-matching command should not be blocked by deny
        let result = check_permission("Bash", Some("ls -la"), None, &cwd, &sp, &deny, &ask);
        assert!(!matches!(result, PermissionCheckResult::Blocked(_)));
    }

    #[test]
    fn test_check_permission_ask_rule() {
        let sp = SessionPermissions::default();
        let deny: Vec<String> = vec![];
        let ask = vec!["Bash(git push *)".to_string()];

        let cwd = dirs::home_dir().unwrap_or_default();

        let result = check_permission("Bash", Some("git push origin main"), None, &cwd, &sp, &deny, &ask);
        assert!(matches!(result, PermissionCheckResult::NeedsPrompt(PromptKind::AskRuleOverride { .. })));

        // After user allows for session (stores the rule pattern), should not prompt again
        let mut sp2 = SessionPermissions::default();
        sp2.allows.insert("Bash(git push *)".to_string()); // stores the rule, not the exact command
        let result = check_permission("Bash", Some("git push origin main"), None, &cwd, &sp2, &deny, &ask);
        assert!(!matches!(result, PermissionCheckResult::NeedsPrompt(PromptKind::AskRuleOverride { .. })));
        // Different command matching same rule pattern — also suppressed
        let result = check_permission("Bash", Some("git push origin feature"), None, &cwd, &sp2, &deny, &ask);
        assert!(!matches!(result, PermissionCheckResult::NeedsPrompt(PromptKind::AskRuleOverride { .. })));
    }

    #[test]
    fn test_check_permission_within_ceiling() {
        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("workspace/linggen");
            let cwd_str = format!("~/{}", "workspace/linggen");
            let mut sp = SessionPermissions::default();
            sp.set_path_mode(&cwd_str, PermissionMode::Edit);

            let deny: Vec<String> = vec![];
            let ask: Vec<String> = vec![];

            // Read within edit ceiling → allowed
            let result = check_permission(
                "Read", None, Some(cwd.join("src/main.rs").to_str().unwrap()),
                &cwd, &sp, &deny, &ask,
            );
            assert!(matches!(result, PermissionCheckResult::Allowed));

            // Write within edit ceiling → allowed
            let result = check_permission(
                "Write", None, Some(cwd.join("src/main.rs").to_str().unwrap()),
                &cwd, &sp, &deny, &ask,
            );
            assert!(matches!(result, PermissionCheckResult::Allowed));
        }
    }

    #[test]
    fn test_check_permission_exceeds_ceiling() {
        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("workspace/linggen");
            let cwd_str = format!("~/{}", "workspace/linggen");
            let mut sp = SessionPermissions::default();
            sp.set_path_mode(&cwd_str, PermissionMode::Read);

            let deny: Vec<String> = vec![];
            let ask: Vec<String> = vec![];

            // Write exceeds read ceiling → prompt
            let result = check_permission(
                "Write", None, Some(cwd.join("src/main.rs").to_str().unwrap()),
                &cwd, &sp, &deny, &ask,
            );
            assert!(matches!(result, PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })));
        }
    }

    #[test]
    fn test_check_permission_system_zone_write() {
        let sp = SessionPermissions::default();
        let deny: Vec<String> = vec![];
        let ask: Vec<String> = vec![];
        let cwd = dirs::home_dir().unwrap_or_default();

        // Write to /etc → system zone prompt
        let result = check_permission("Write", None, Some("/etc/hosts"), &cwd, &sp, &deny, &ask);
        assert!(matches!(result, PermissionCheckResult::NeedsPrompt(PromptKind::SystemZoneWrite { .. })));
    }

    #[test]
    fn test_check_permission_locked_blocks() {
        let mut sp = SessionPermissions::default();
        sp.locked = true;

        let deny: Vec<String> = vec![];
        let ask: Vec<String> = vec![];
        let cwd = dirs::home_dir().unwrap_or_default();

        // Write to system zone while locked → blocked
        let result = check_permission("Write", None, Some("/etc/hosts"), &cwd, &sp, &deny, &ask);
        assert!(matches!(result, PermissionCheckResult::Blocked(_)));
    }

    #[test]
    fn test_check_permission_system_zone_read_allowed() {
        if let Some(_home) = dirs::home_dir() {
            let mut sp = SessionPermissions::default();
            // Grant read on /etc
            sp.set_path_mode("/etc", PermissionMode::Read);

            let deny: Vec<String> = vec![];
            let ask: Vec<String> = vec![];

            let cwd = dirs::home_dir().unwrap_or_default();

            // Read in system zone with grant → allowed
            let result = check_permission("Read", None, Some("/etc/hosts"), &cwd, &sp, &deny, &ask);
            assert!(matches!(result, PermissionCheckResult::Allowed));

            // Write in system zone → still per-action
            let result = check_permission("Write", None, Some("/etc/hosts"), &cwd, &sp, &deny, &ask);
            assert!(matches!(result, PermissionCheckResult::NeedsPrompt(PromptKind::SystemZoneWrite { .. })));
        }
    }

    #[test]
    fn test_parse_exceeds_ceiling_answer() {
        let mode = PermissionMode::Edit;
        assert_eq!(parse_exceeds_ceiling_answer("Allow once", &mode), PermissionAction::AllowOnce);
        assert_eq!(parse_exceeds_ceiling_answer("Switch to edit mode", &mode), PermissionAction::AllowSession);
        assert_eq!(parse_exceeds_ceiling_answer("Deny", &mode), PermissionAction::Deny);
    }

    #[test]
    fn test_parse_ask_rule_answer() {
        assert_eq!(parse_ask_rule_answer("Allow once"), PermissionAction::AllowOnce);
        assert_eq!(parse_ask_rule_answer("Allow for this session"), PermissionAction::AllowSession);
        assert_eq!(parse_ask_rule_answer("Deny"), PermissionAction::Deny);
    }

    #[test]
    fn test_parse_system_zone_answer() {
        assert_eq!(parse_system_zone_answer("Allow once"), PermissionAction::AllowOnce);
        assert_eq!(parse_system_zone_answer("Deny"), PermissionAction::Deny);
    }

    #[test]
    fn test_build_exceeds_ceiling_question() {
        let q = build_exceeds_ceiling_question("Edit src/main.rs", &PermissionMode::Edit, "~/workspace/linggen");
        assert_eq!(q.options.len(), 3);
        assert_eq!(q.options[0].label, "Allow once");
        assert_eq!(q.options[1].label, "Switch to edit mode");
        assert_eq!(q.options[2].label, "Deny");
    }

    #[test]
    fn test_build_system_zone_question() {
        let q = build_system_zone_question("Edit /etc/hosts");
        assert_eq!(q.options.len(), 2);
        assert_eq!(q.options[0].label, "Allow once");
        assert_eq!(q.options[1].label, "Deny");
    }
}