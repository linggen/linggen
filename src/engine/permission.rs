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
    AllowProject,
    Deny,
    /// User denied with a custom message to relay to the model.
    DenyWithMessage(String),
    /// Deny for this project (persisted deny rule).
    DenyProject,
}

// ---------------------------------------------------------------------------
// Persisted format for project-level permissions
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Default)]
struct PersistedPermissions {
    #[serde(default)]
    tool_allows: HashSet<String>,
    #[serde(default)]
    tool_denies: HashSet<String>,
}

// ---------------------------------------------------------------------------
// PermissionStore
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct PermissionStore {
    session_allows: HashSet<String>,
    project_allows: HashSet<String>,
    project_denies: HashSet<String>,
    project_file: Option<PathBuf>,
}

impl PermissionStore {
    /// Load project-scoped permissions from disk.
    /// `project_dir` is `{workspace}/.linggen/` (same pattern as Claude Code's `.claude/`).
    pub fn load(project_dir: &Path) -> Self {
        let file = project_dir.join("permissions.json");
        let (project_allows, project_denies) = if file.exists() {
            match fs::read_to_string(&file) {
                Ok(content) => match serde_json::from_str::<PersistedPermissions>(&content) {
                    Ok(p) => (p.tool_allows, p.tool_denies),
                    Err(e) => {
                        warn!("Failed to parse permissions.json: {}", e);
                        (HashSet::new(), HashSet::new())
                    }
                },
                Err(e) => {
                    warn!("Failed to read permissions.json: {}", e);
                    (HashSet::new(), HashSet::new())
                }
            }
        } else {
            (HashSet::new(), HashSet::new())
        };
        if !project_allows.is_empty() || !project_denies.is_empty() {
            info!(
                "Loaded project permissions from {}: {} allows, {} denies — {:?}",
                file.display(),
                project_allows.len(),
                project_denies.len(),
                project_allows,
            );
        }
        Self {
            session_allows: HashSet::new(),
            project_allows,
            project_denies,
            project_file: Some(file),
        }
    }

    /// Create an empty store (no persistence).
    #[allow(dead_code)]
    pub fn empty() -> Self {
        Self {
            session_allows: HashSet::new(),
            project_allows: HashSet::new(),
            project_denies: HashSet::new(),
            project_file: None,
        }
    }

    /// Check whether the tool is allowed (session OR project scope).
    ///
    /// For Bash commands, pass the command string to enable pattern-based matching.
    /// A blanket `"Bash"` entry still grants access to all commands (backward compat).
    /// Pattern entries like `"Bash:npm run *"` only match commands that fit the glob.
    pub fn check(&self, tool: &str, command: Option<&str>) -> bool {
        // 0. Deny rules take precedence over allows.
        if self.is_denied(tool, command) {
            return false;
        }
        // 1. Blanket tool-level allow (backward compat)
        if self.session_allows.contains(tool) || self.project_allows.contains(tool) {
            return true;
        }
        // 2. Pattern-based matching (Bash commands, file paths)
        if let Some(cmd) = command {
            let prefix = format!("{}:", tool);
            for entry in self.session_allows.iter().chain(self.project_allows.iter()) {
                if let Some(pattern) = entry.strip_prefix(&prefix) {
                    if command_matches_pattern(cmd, pattern) {
                        return true;
                    }
                }
            }
        }
        debug!(
            "Permission check: tool={} command={:?} → NOT allowed (session={:?}, project={:?})",
            tool, command, self.session_allows, self.project_allows,
        );
        false
    }

    /// Check whether a tool + arg is project-denied.
    pub fn is_denied(&self, tool: &str, arg: Option<&str>) -> bool {
        // Blanket deny: "Bash", "Write", etc.
        if self.project_denies.contains(tool) {
            return true;
        }
        // Pattern-based deny: "Bash:npm run *", "Edit:src/secret/*"
        if let Some(cmd) = arg {
            let prefix = format!("{}:", tool);
            for entry in self.project_denies.iter() {
                if let Some(pattern) = entry.strip_prefix(&prefix) {
                    if command_matches_pattern(cmd, pattern) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Allow a tool for the remainder of this session/task.
    pub fn allow_for_session(&mut self, tool: &str) {
        self.session_allows.insert(tool.to_string());
    }

    /// Allow a tool for this project (persisted to disk).
    pub fn allow_for_project(&mut self, tool: &str) {
        self.project_allows.insert(tool.to_string());
        self.persist();
    }

    /// Deny a tool/pattern for this project (persisted to disk).
    pub fn deny_for_project(&mut self, key: &str) {
        self.project_denies.insert(key.to_string());
        self.persist();
    }

    /// Clear session-scoped permissions (called on session reset).
    #[allow(dead_code)]
    pub fn clear_session(&mut self) {
        self.session_allows.clear();
    }

    fn persist(&self) {
        let Some(file) = &self.project_file else {
            return;
        };
        if let Some(parent) = file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let data = PersistedPermissions {
            tool_allows: self.project_allows.clone(),
            tool_denies: self.project_denies.clone(),
        };
        match serde_json::to_string_pretty(&data) {
            Ok(json) => {
                if let Err(e) = fs::write(file, json) {
                    warn!("Failed to write permissions.json: {}", e);
                }
            }
            Err(e) => warn!("Failed to serialize permissions: {}", e),
        }
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

/// Build the AskUser question for a file-scoped Write/Edit permission prompt.
/// Edit/Write permissions are session-scoped only — no project-level persistence
/// so users re-approve each session.
pub fn build_file_permission_question(
    tool: &str,
    file_path: &str,
    pattern: &str,
) -> AskUserQuestion {
    let options = vec![
        AskUserOption {
            label: "Allow once".to_string(),
            description: Some("Proceed this one time only".to_string()),
            preview: None,
        },
        AskUserOption {
            label: format!("Allow {}({}) for this session", tool, pattern),
            description: Some("Session-scoped; resets on new session".to_string()),
            preview: None,
        },
        AskUserOption {
            label: format!("Allow all {} for this session", tool),
            description: Some(format!("Allow every {} without asking again this session", tool)),
            preview: None,
        },
        AskUserOption {
            label: "Deny".to_string(),
            description: Some(format!("Deny this {} call", tool)),
            preview: None,
        },
    ];

    AskUserQuestion {
        question: format!("{} {}", tool, file_path),
        header: "Permission".to_string(),
        options,
        multi_select: false,
    }
}

/// Parse the selected option from a file-scoped Write/Edit permission prompt.
/// Returns `(action, permission_key)` where `permission_key` is the string to store
/// (e.g., `"Edit:src/components/*"` for pattern-scoped, or `"Edit"` for blanket session allow).
pub fn parse_file_permission_answer(
    selected: &str,
    tool: &str,
    pattern: &str,
) -> (PermissionAction, Option<String>) {
    if selected == "Allow once" {
        return (PermissionAction::AllowOnce, None);
    }
    let key = format!("{}:{}", tool, pattern);
    if selected == format!("Allow {}({}) for this session", tool, pattern) {
        return (PermissionAction::AllowSession, Some(key));
    }
    // Blanket "allow all Edit/Write for this session" — key is just the tool name.
    if selected == format!("Allow all {} for this session", tool) {
        return (PermissionAction::AllowSession, Some(tool.to_string()));
    }
    // "Deny", "Cancel" (backward compat), or anything unexpected
    (PermissionAction::Deny, None)
}

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
pub fn permission_target_summary(tool: &str, args: &serde_json::Value, ws_root: &Path) -> String {
    match tool {
        "Write" | "Edit" => normalize_tool_path_arg(ws_root, args)
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

/// Build the AskUser question for a permission prompt.
pub fn build_permission_question(tool: &str, target_summary: &str) -> AskUserQuestion {
    AskUserQuestion {
        question: format!("{} {}", tool, target_summary),
        header: "Permission".to_string(),
        options: vec![
            AskUserOption {
                label: "Allow once".to_string(),
                description: Some("Proceed this one time only".to_string()),
                preview: None,
            },
            AskUserOption {
                label: format!("Allow all {} for this session", tool),
                description: Some("Session-scoped; resets on new session".to_string()),
                preview: None,
            },
            AskUserOption {
                label: format!("Allow all {} for this project", tool),
                description: Some("Persisted; won't ask again for this project".to_string()),
                preview: None,
            },
            AskUserOption {
                label: "Deny".to_string(),
                description: Some("Deny this tool call".to_string()),
                preview: None,
            },
            AskUserOption {
                label: format!("Deny all {} for this project", tool),
                description: Some("Persisted deny rule; auto-blocked without prompt".to_string()),
                preview: None,
            },
        ],
        multi_select: false,
    }
}

/// Build the AskUser question for a web tool permission prompt.
pub fn build_web_permission_question(tool: &str, target_summary: &str) -> AskUserQuestion {
    AskUserQuestion {
        question: format!("{} {}", tool, target_summary),
        header: "Permission".to_string(),
        options: vec![
            AskUserOption {
                label: "Allow this request".to_string(),
                description: Some("Proceed this one time only".to_string()),
                preview: None,
            },
            AskUserOption {
                label: format!("Allow all {} for this session", tool),
                description: Some("Session-scoped; resets on new session".to_string()),
                preview: None,
            },
            AskUserOption {
                label: format!("Allow all {} for this project", tool),
                description: Some("Persisted; won't ask again for this project".to_string()),
                preview: None,
            },
            AskUserOption {
                label: "Deny".to_string(),
                description: Some("Deny this web request".to_string()),
                preview: None,
            },
            AskUserOption {
                label: format!("Deny all {} for this project", tool),
                description: Some("Persisted deny rule; auto-blocked without prompt".to_string()),
                preview: None,
            },
        ],
        multi_select: false,
    }
}

/// Parse the selected option from a web permission prompt.
pub fn parse_web_permission_answer(selected: &str, tool: &str) -> PermissionAction {
    if selected == "Allow this request" {
        PermissionAction::AllowOnce
    } else if selected == format!("Allow all {} for this session", tool) {
        PermissionAction::AllowSession
    } else if selected == format!("Allow all {} for this project", tool) {
        PermissionAction::AllowProject
    } else if selected == format!("Deny all {} for this project", tool) {
        PermissionAction::DenyProject
    } else {
        PermissionAction::Deny
    }
}

/// Build the AskUser question for a Bash permission prompt with command-level granularity.
///
/// If a pattern was derived (simple command), offers pattern-scoped options.
/// If no pattern (compound command), only offers blanket allow or cancel.
pub fn build_bash_permission_question(command: &str, pattern: Option<&str>) -> AskUserQuestion {
    let mut options = vec![AskUserOption {
        label: "Allow once".to_string(),
        description: Some("Proceed this one time only".to_string()),
        preview: None,
    }];

    if let Some(pat) = pattern {
        // Pattern-scoped option — always available for simple commands
        options.push(AskUserOption {
            label: format!("Allow Bash({}) for this session", pat),
            description: Some("Session-scoped; resets on new session".to_string()),
            preview: None,
        });
    }
    // No blanket "allow all Bash" — Bash permissions must always be scoped to a pattern.
    options.push(AskUserOption {
        label: "Deny".to_string(),
        description: Some("Deny this command".to_string()),
        preview: None,
    });

    AskUserQuestion {
        question: format!("Bash {}", command),
        header: "Permission".to_string(),
        options,
        multi_select: false,
    }
}

/// Parse the selected option from a Bash permission prompt.
/// Returns `(action, permission_key)` where `permission_key` is the string to store
/// (e.g., `"Bash:npm run *"` for pattern-scoped). No blanket "allow all Bash" option.
pub fn parse_bash_permission_answer(
    selected: &str,
    _tool: &str,
    pattern: Option<&str>,
) -> (PermissionAction, Option<String>) {
    if selected == "Allow once" {
        return (PermissionAction::AllowOnce, None);
    }
    // Pattern-scoped option (only when a pattern was derived)
    if let Some(pat) = pattern {
        let key = format!("Bash:{}", pat);
        if selected == format!("Allow Bash({}) for this session", pat) {
            return (PermissionAction::AllowSession, Some(key));
        }
    }
    // "Deny", "Cancel" (backward compat), or anything unexpected
    (PermissionAction::Deny, None)
}

/// Parse the selected option label back into a PermissionAction.
pub fn parse_permission_answer(selected: &str, tool: &str) -> PermissionAction {
    if selected == "Allow once" {
        PermissionAction::AllowOnce
    } else if selected == format!("Allow all {} for this session", tool) {
        PermissionAction::AllowSession
    } else if selected == format!("Allow all {} for this project", tool) {
        PermissionAction::AllowProject
    } else if selected == format!("Deny all {} for this project", tool) {
        PermissionAction::DenyProject
    } else {
        // "Cancel" or anything unexpected
        PermissionAction::Deny
    }
}

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

    // Temp zone
    if path.starts_with("/tmp") || path.starts_with("/var/tmp") {
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

    // Check for output redirection → at least write
    if cmd.contains(" > ") || cmd.contains(" >> ") {
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
        "uniq", "tr", "cut", "paste", "diff", "comm", "tee",
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
        "ln", "install", "rsync",
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
    session_perms: &SessionPermissions,
    deny_rules: &[String],
    ask_rules: &[String],
) -> PermissionCheckResult {
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
        // Check if user already overrode this ask rule for the session
        let override_key = if let Some(a) = rule_arg {
            format!("{}:{}", tool, a)
        } else {
            tool.to_string()
        };
        if !session_perms.allows.contains(&override_key) {
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
        .or_else(|| Some(std::env::current_dir().unwrap_or_default()));
    let target_path = target_path.unwrap();
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

    #[test]
    fn test_is_destructive() {
        assert!(is_destructive_tool("Write"));
        assert!(is_destructive_tool("Edit"));
        assert!(is_destructive_tool("Bash"));
        // Patch is not a destructive user-facing tool.
        assert!(!is_destructive_tool("Patch"));
        assert!(!is_destructive_tool("Read"));
        assert!(!is_destructive_tool("Glob"));
        assert!(!is_destructive_tool("Grep"));
    }

    #[test]
    fn test_permission_store_session() {
        let mut store = PermissionStore::empty();
        assert!(!store.check("Write", None));
        store.allow_for_session("Write");
        assert!(store.check("Write", None));
        assert!(!store.check("Bash", None));
        store.clear_session();
        assert!(!store.check("Write", None));
    }

    #[test]
    fn test_permission_store_load_persist() {
        let tmp = std::env::temp_dir().join("linggen_perm_test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let mut store = PermissionStore::load(&tmp);
        assert!(!store.check("Edit", None));
        store.allow_for_project("Edit");
        assert!(store.check("Edit", None));

        // Reload from disk
        let store2 = PermissionStore::load(&tmp);
        assert!(store2.check("Edit", None));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_parse_permission_answer() {
        assert_eq!(parse_permission_answer("Allow once", "Write"), PermissionAction::AllowOnce);
        assert_eq!(
            parse_permission_answer("Allow all Write for this session", "Write"),
            PermissionAction::AllowSession
        );
        assert_eq!(
            parse_permission_answer("Allow all Bash for this project", "Bash"),
            PermissionAction::AllowProject
        );
        // Both "Deny" and "Cancel" (backward compat) resolve to Deny
        assert_eq!(parse_permission_answer("Deny", "Write"), PermissionAction::Deny);
        assert_eq!(parse_permission_answer("Cancel", "Write"), PermissionAction::Deny);
        assert_eq!(parse_permission_answer("anything else", "Write"), PermissionAction::Deny);
    }

    #[test]
    fn test_build_permission_question() {
        let q = build_permission_question("Write", "src/main.rs");
        assert_eq!(q.question, "Write src/main.rs");
        assert_eq!(q.header, "Permission");
        assert_eq!(q.options.len(), 5);
        assert_eq!(q.options[0].label, "Allow once");
        assert!(q.options[1].label.contains("Write"));
        assert_eq!(q.options[4].label, "Deny all Write for this project");
    }

    #[test]
    fn test_permission_target_summary_bash() {
        let args = serde_json::json!({ "cmd": "cargo build" });
        let summary = permission_target_summary("Bash", &args, Path::new("/tmp"));
        assert_eq!(summary, "cargo build");
    }

    #[test]
    fn test_permission_target_summary_write() {
        let args = serde_json::json!({ "path": "src/main.rs", "content": "fn main() {}" });
        let summary = permission_target_summary("Write", &args, Path::new("/tmp"));
        assert_eq!(summary, "src/main.rs");
    }

    #[test]
    fn test_is_web_tool() {
        assert!(is_web_tool("WebFetch"));
        assert!(is_web_tool("WebSearch"));
        assert!(!is_web_tool("Read"));
        assert!(!is_web_tool("Bash"));
        assert!(!is_web_tool("Write"));
    }

    #[test]
    fn test_permission_target_summary_webfetch() {
        let args = serde_json::json!({ "url": "https://example.com/docs" });
        let summary = permission_target_summary("WebFetch", &args, Path::new("/tmp"));
        assert_eq!(summary, "https://example.com/docs");
    }

    #[test]
    fn test_permission_target_summary_websearch() {
        let args = serde_json::json!({ "query": "rust async patterns" });
        let summary = permission_target_summary("WebSearch", &args, Path::new("/tmp"));
        assert_eq!(summary, "rust async patterns");
    }

    #[test]
    fn test_build_web_permission_question() {
        let q = build_web_permission_question("WebFetch", "https://example.com");
        assert_eq!(q.question, "WebFetch https://example.com");
        assert_eq!(q.options.len(), 5);
        assert_eq!(q.options[0].label, "Allow this request");
        assert!(q.options[1].label.contains("WebFetch"));
        assert_eq!(q.options[2].label, "Allow all WebFetch for this project");
        assert_eq!(q.options[3].label, "Deny");
        assert_eq!(q.options[4].label, "Deny all WebFetch for this project");
    }

    #[test]
    fn test_parse_web_permission_answer() {
        assert_eq!(
            parse_web_permission_answer("Allow this request", "WebFetch"),
            PermissionAction::AllowOnce
        );
        assert_eq!(
            parse_web_permission_answer("Allow all WebFetch for this session", "WebFetch"),
            PermissionAction::AllowSession
        );
        assert_eq!(
            parse_web_permission_answer("Allow all WebFetch for this project", "WebFetch"),
            PermissionAction::AllowProject
        );
        // Both "Deny" and "Cancel" (backward compat) resolve to Deny
        assert_eq!(
            parse_web_permission_answer("Deny", "WebFetch"),
            PermissionAction::Deny
        );
        assert_eq!(
            parse_web_permission_answer("Cancel", "WebFetch"),
            PermissionAction::Deny
        );
    }

    // --- Bash command pattern tests ---

    #[test]
    fn test_is_compound_command() {
        // Compound commands
        assert!(is_compound_command("ls; rm -rf /"));
        assert!(is_compound_command("echo foo && echo bar"));
        assert!(is_compound_command("cat file || true"));
        assert!(is_compound_command("echo `whoami`"));
        assert!(is_compound_command("echo $(whoami)"));
        assert!(is_compound_command("ls | grep foo"));

        // Simple commands
        assert!(!is_compound_command("npm run build"));
        assert!(!is_compound_command("cargo test"));
        assert!(!is_compound_command("git status"));
        assert!(!is_compound_command("ls -la"));
        assert!(!is_compound_command("pwd"));
    }

    #[test]
    fn test_derive_command_pattern() {
        // Single word → exact match
        assert_eq!(derive_command_pattern("pwd"), Some("pwd".to_string()));

        // Two words → "program *"
        assert_eq!(
            derive_command_pattern("git status"),
            Some("git *".to_string())
        );
        assert_eq!(
            derive_command_pattern("ls -la"),
            Some("ls *".to_string())
        );

        // 3+ words → "program subcommand *"
        assert_eq!(
            derive_command_pattern("npm run build"),
            Some("npm run *".to_string())
        );
        assert_eq!(
            derive_command_pattern("cargo test --release"),
            Some("cargo test *".to_string())
        );

        // 3+ words with flag as second token → "program *"
        assert_eq!(
            derive_command_pattern("ls -la /tmp"),
            Some("ls *".to_string())
        );

        // 3+ words where second token is a path → "program *"
        assert_eq!(
            derive_command_pattern("rm /tmp/foo /tmp/bar"),
            Some("rm *".to_string())
        );
        assert_eq!(
            derive_command_pattern("rm ./file1 ./file2"),
            Some("rm *".to_string())
        );
        assert_eq!(
            derive_command_pattern("cp ~/src/file ~/dst/file"),
            Some("cp *".to_string())
        );

        // Compound commands → "first_program *" from first segment
        assert_eq!(derive_command_pattern("ls && cat foo"), Some("ls *".to_string()));
        assert_eq!(derive_command_pattern("echo $(pwd)"), Some("echo *".to_string()));
        assert_eq!(
            derive_command_pattern("find ~/.linggen -type f | head -20"),
            Some("find *".to_string())
        );
        assert_eq!(
            derive_command_pattern("npm run build && npm run test"),
            Some("npm *".to_string())
        );

        // Empty → None
        assert_eq!(derive_command_pattern(""), None);
    }

    #[test]
    fn test_command_matches_pattern() {
        // Glob matching
        assert!(command_matches_pattern("npm run build", "npm run *"));
        assert!(command_matches_pattern("npm run test", "npm run *"));
        assert!(!command_matches_pattern("cargo build", "npm run *"));

        // "git *" matches any git command
        assert!(command_matches_pattern("git status", "git *"));
        assert!(command_matches_pattern("git push origin main", "git *"));
        assert!(!command_matches_pattern("npm install", "git *"));

        // Exact match
        assert!(command_matches_pattern("pwd", "pwd"));
        assert!(!command_matches_pattern("ls", "pwd"));
    }

    #[test]
    fn test_check_with_command_pattern() {
        let mut store = PermissionStore::empty();

        // No permissions → denied
        assert!(!store.check("Bash", Some("npm run build")));

        // Add pattern-scoped permission
        store.allow_for_session("Bash:npm run *");
        assert!(store.check("Bash", Some("npm run build")));
        assert!(store.check("Bash", Some("npm run test")));
        assert!(!store.check("Bash", Some("cargo build")));

        // Multiple patterns
        store.allow_for_session("Bash:cargo *");
        assert!(store.check("Bash", Some("cargo build")));
        assert!(store.check("Bash", Some("cargo test --release")));
    }

    #[test]
    fn test_backward_compat_blanket_allow() {
        let mut store = PermissionStore::empty();

        // Old-style blanket "Bash" entry allows everything
        store.allow_for_session("Bash");
        assert!(store.check("Bash", Some("npm run build")));
        assert!(store.check("Bash", Some("rm -rf /")));
        assert!(store.check("Bash", None));
    }

    #[test]
    fn test_non_bash_tools_unaffected() {
        let mut store = PermissionStore::empty();

        // Write/Edit still use simple matching (no command parameter)
        store.allow_for_session("Write");
        assert!(store.check("Write", None));
        assert!(!store.check("Edit", None));
    }

    #[test]
    fn test_build_bash_permission_question_with_pattern() {
        let q = build_bash_permission_question("npm run build", Some("npm run *"));
        assert_eq!(q.question, "Bash npm run build");
        // With pattern: Allow once, pattern session, Deny
        assert_eq!(q.options.len(), 3);
        assert_eq!(q.options[0].label, "Allow once");
        assert_eq!(q.options[1].label, "Allow Bash(npm run *) for this session");
        assert_eq!(q.options[2].label, "Deny");
    }

    #[test]
    fn test_build_bash_permission_question_no_pattern() {
        let q = build_bash_permission_question("ls && cat foo", None);
        assert_eq!(q.question, "Bash ls && cat foo");
        // Without pattern (compound command): Allow once, Deny (no pattern to offer)
        assert_eq!(q.options.len(), 2);
        assert_eq!(q.options[0].label, "Allow once");
        assert_eq!(q.options[1].label, "Deny");
    }

    #[test]
    fn test_parse_bash_permission_answer_all_paths() {
        let pat = Some("npm run *");

        // Allow once
        let (action, key) = parse_bash_permission_answer("Allow once", "Bash", pat);
        assert_eq!(action, PermissionAction::AllowOnce);
        assert!(key.is_none());

        // Pattern-scoped session
        let (action, key) =
            parse_bash_permission_answer("Allow Bash(npm run *) for this session", "Bash", pat);
        assert_eq!(action, PermissionAction::AllowSession);
        assert_eq!(key.as_deref(), Some("Bash:npm run *"));

        // Deny (and backward-compat Cancel)
        let (action, key) = parse_bash_permission_answer("Deny", "Bash", pat);
        assert_eq!(action, PermissionAction::Deny);
        assert!(key.is_none());
        let (action, key) = parse_bash_permission_answer("Cancel", "Bash", pat);
        assert_eq!(action, PermissionAction::Deny);
        assert!(key.is_none());

        // No pattern → only allow once or deny
        let (action, key) = parse_bash_permission_answer("Allow once", "Bash", None);
        assert_eq!(action, PermissionAction::AllowOnce);
        assert!(key.is_none());
    }

    #[test]
    fn test_check_with_project_pattern() {
        let tmp = std::env::temp_dir().join("linggen_perm_pattern_test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let mut store = PermissionStore::load(&tmp);
        store.allow_for_project("Bash:git *");

        // Pattern matches
        assert!(store.check("Bash", Some("git status")));
        assert!(store.check("Bash", Some("git push origin main")));
        assert!(!store.check("Bash", Some("npm install")));

        // Reload from disk and verify persistence
        let store2 = PermissionStore::load(&tmp);
        assert!(store2.check("Bash", Some("git status")));
        assert!(!store2.check("Bash", Some("npm install")));

        let _ = fs::remove_dir_all(&tmp);
    }

    // --- File-scoped permission tests ---

    #[test]
    fn test_derive_file_pattern() {
        assert_eq!(derive_file_pattern("src/components/App.tsx"), "src/components/*");
        assert_eq!(derive_file_pattern("src/main.rs"), "src/*");
        assert_eq!(derive_file_pattern("README.md"), "*");
        assert_eq!(derive_file_pattern("deep/nested/dir/file.rs"), "deep/nested/dir/*");
        // Absolute paths should be sanitized
        assert_eq!(derive_file_pattern("/etc/passwd"), "etc/*");
        assert_eq!(derive_file_pattern("/usr/local/bin/tool"), "usr/local/bin/*");
        // Edge cases
        assert_eq!(derive_file_pattern(""), "*");
        assert_eq!(derive_file_pattern("/"), "*");
    }

    #[test]
    fn test_build_file_permission_question() {
        let q = build_file_permission_question("Edit", "src/components/App.tsx", "src/components/*");
        assert_eq!(q.question, "Edit src/components/App.tsx");
        assert_eq!(q.options.len(), 4);
        assert_eq!(q.options[0].label, "Allow once");
        assert_eq!(q.options[1].label, "Allow Edit(src/components/*) for this session");
        assert_eq!(q.options[2].label, "Allow all Edit for this session");
        assert_eq!(q.options[3].label, "Deny");
    }

    #[test]
    fn test_parse_file_permission_answer_all_paths() {
        let pat = "src/components/*";

        // Allow once
        let (action, key) = parse_file_permission_answer("Allow once", "Edit", pat);
        assert_eq!(action, PermissionAction::AllowOnce);
        assert!(key.is_none());

        // Pattern-scoped session
        let (action, key) = parse_file_permission_answer(
            "Allow Edit(src/components/*) for this session", "Edit", pat,
        );
        assert_eq!(action, PermissionAction::AllowSession);
        assert_eq!(key.as_deref(), Some("Edit:src/components/*"));

        // Blanket session allow
        let (action, key) = parse_file_permission_answer(
            "Allow all Edit for this session", "Edit", pat,
        );
        assert_eq!(action, PermissionAction::AllowSession);
        assert_eq!(key.as_deref(), Some("Edit"));

        // Deny
        let (action, key) = parse_file_permission_answer("Deny", "Edit", pat);
        assert_eq!(action, PermissionAction::Deny);
        assert!(key.is_none());
    }

    #[test]
    fn test_check_with_file_pattern() {
        let mut store = PermissionStore::empty();

        // No permissions → denied
        assert!(!store.check("Edit", Some("src/components/App.tsx")));

        // Add pattern-scoped permission
        store.allow_for_session("Edit:src/components/*");
        assert!(store.check("Edit", Some("src/components/App.tsx")));
        assert!(store.check("Edit", Some("src/components/Header.tsx")));
        assert!(!store.check("Edit", Some("src/main.rs")));

        // Root glob pattern — "*" matches single path segment (root-level files)
        store.allow_for_session("Write:*");
        assert!(store.check("Write", Some("README.md")));
        // Note: globset's "*" matches any single path segment, but our command_matches_pattern
        // uses is_match which treats the input as a path, so "*" matches "src/main.rs" too.
        // This is acceptable for root-level file patterns — users granting Write(*) accept all files.
        assert!(store.check("Write", Some("src/main.rs")));
    }

    #[test]
    fn test_backward_compat_blanket_write() {
        let mut store = PermissionStore::empty();

        // Old-style blanket "Write" entry still allows everything
        store.allow_for_session("Write");
        assert!(store.check("Write", Some("src/main.rs")));
        assert!(store.check("Write", Some("README.md")));
        assert!(store.check("Write", None));
    }

    // --- Deny rules tests ---

    #[test]
    fn test_deny_takes_precedence_over_allow() {
        let mut store = PermissionStore::empty();

        // Allow everything, then deny a pattern
        store.allow_for_session("Bash");
        store.deny_for_project("Bash:rm *");

        // Blanket allow works for non-denied commands
        assert!(store.check("Bash", Some("git status")));
        // But denied pattern is blocked
        assert!(!store.check("Bash", Some("rm -rf /")));
    }

    #[test]
    fn test_deny_blanket_tool() {
        let mut store = PermissionStore::empty();

        store.allow_for_session("WebFetch");
        assert!(store.check("WebFetch", None));

        store.deny_for_project("WebFetch");
        assert!(!store.check("WebFetch", None));
    }

    #[test]
    fn test_deny_file_pattern() {
        let mut store = PermissionStore::empty();

        store.allow_for_session("Edit:src/*");
        assert!(store.check("Edit", Some("src/main.rs")));

        store.deny_for_project("Edit:src/*");
        assert!(!store.check("Edit", Some("src/main.rs")));
    }

    #[test]
    fn test_deny_persistence() {
        let tmp = std::env::temp_dir().join("linggen_deny_persist_test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let mut store = PermissionStore::load(&tmp);
        store.deny_for_project("Bash:rm *");

        // Reload from disk
        let store2 = PermissionStore::load(&tmp);
        assert!(store2.is_denied("Bash", Some("rm -rf /")));
        assert!(!store2.is_denied("Bash", Some("git status")));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_backward_compat_no_denies_in_json() {
        let tmp = std::env::temp_dir().join("linggen_compat_deny_test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // Write a permissions.json without tool_denies field (old format)
        let json = r#"{ "tool_allows": ["Bash"] }"#;
        fs::write(tmp.join("permissions.json"), json).unwrap();

        let store = PermissionStore::load(&tmp);
        assert!(store.check("Bash", Some("any command")));
        assert!(!store.is_denied("Bash", Some("any command")));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_parse_bash_deny() {
        let pat = Some("npm run *");

        // Project deny options removed — "Deny" is the only deny action now
        let (action, key) = parse_bash_permission_answer("Deny", "Bash", pat);
        assert_eq!(action, PermissionAction::Deny);
        assert!(key.is_none());

        // Unknown labels also map to Deny
        let (action, key) = parse_bash_permission_answer(
            "Deny Bash(npm run *) for this project", "Bash", pat,
        );
        assert_eq!(action, PermissionAction::Deny);
        assert!(key.is_none());
    }

    #[test]
    fn test_parse_web_deny_project() {
        assert_eq!(
            parse_web_permission_answer("Deny all WebFetch for this project", "WebFetch"),
            PermissionAction::DenyProject,
        );
    }

    #[test]
    fn test_parse_permission_answer_deny_project() {
        assert_eq!(
            parse_permission_answer("Deny all Patch for this project", "Patch"),
            PermissionAction::DenyProject,
        );
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
        // Output redirection promotes read to write
        assert_eq!(classify_bash_command("echo hello > out.txt"), BashClass::Write);
        assert_eq!(classify_bash_command("ls > files.txt"), BashClass::Write);
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

        let result = check_permission("Bash", Some("sudo apt install foo"), None, &sp, &deny, &ask);
        assert!(matches!(result, PermissionCheckResult::Blocked(_)));

        // Non-matching command should not be blocked by deny
        let result = check_permission("Bash", Some("ls -la"), None, &sp, &deny, &ask);
        assert!(!matches!(result, PermissionCheckResult::Blocked(_)));
    }

    #[test]
    fn test_check_permission_ask_rule() {
        let sp = SessionPermissions::default();
        let deny: Vec<String> = vec![];
        let ask = vec!["Bash(git push *)".to_string()];

        let result = check_permission("Bash", Some("git push origin main"), None, &sp, &deny, &ask);
        assert!(matches!(result, PermissionCheckResult::NeedsPrompt(PromptKind::AskRuleOverride { .. })));

        // After user allows for session, should not prompt again
        let mut sp2 = SessionPermissions::default();
        sp2.allows.insert("Bash:git push origin main".to_string());
        let result = check_permission("Bash", Some("git push origin main"), None, &sp2, &deny, &ask);
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
                &sp, &deny, &ask,
            );
            assert!(matches!(result, PermissionCheckResult::Allowed));

            // Write within edit ceiling → allowed
            let result = check_permission(
                "Write", None, Some(cwd.join("src/main.rs").to_str().unwrap()),
                &sp, &deny, &ask,
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
                &sp, &deny, &ask,
            );
            assert!(matches!(result, PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })));
        }
    }

    #[test]
    fn test_check_permission_system_zone_write() {
        let sp = SessionPermissions::default();
        let deny: Vec<String> = vec![];
        let ask: Vec<String> = vec![];

        // Write to /etc → system zone prompt
        let result = check_permission("Write", None, Some("/etc/hosts"), &sp, &deny, &ask);
        assert!(matches!(result, PermissionCheckResult::NeedsPrompt(PromptKind::SystemZoneWrite { .. })));
    }

    #[test]
    fn test_check_permission_locked_blocks() {
        let mut sp = SessionPermissions::default();
        sp.locked = true;

        let deny: Vec<String> = vec![];
        let ask: Vec<String> = vec![];

        // Write to system zone while locked → blocked
        let result = check_permission("Write", None, Some("/etc/hosts"), &sp, &deny, &ask);
        assert!(matches!(result, PermissionCheckResult::Blocked(_)));
    }

    #[test]
    fn test_check_permission_system_zone_read_allowed() {
        if let Some(home) = dirs::home_dir() {
            let mut sp = SessionPermissions::default();
            // Grant read on /etc
            sp.set_path_mode("/etc", PermissionMode::Read);

            let deny: Vec<String> = vec![];
            let ask: Vec<String> = vec![];

            // Read in system zone with grant → allowed
            let result = check_permission("Read", None, Some("/etc/hosts"), &sp, &deny, &ask);
            assert!(matches!(result, PermissionCheckResult::Allowed));

            // Write in system zone → still per-action
            let result = check_permission("Write", None, Some("/etc/hosts"), &sp, &deny, &ask);
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