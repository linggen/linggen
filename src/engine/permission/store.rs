use super::model::{expand_tilde, PathMode, PermissionMode};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use tracing::warn;

fn default_true() -> bool {
    true
}

/// Per-session permission state, persisted to `permission.json`.
///
/// `path_modes[]` is the entire grant table — only explicit user approvals
/// (mode upgrade prompts), mission frontmatter, and skill frontmatter write
/// to it. `interactive` is metadata: false for mission and proxy-consumer
/// sessions so the engine pauses or fails instead of trying to prompt.
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionPermissions {
    #[serde(default)]
    pub path_modes: Vec<PathMode>,
    /// True for normal user sessions (prompt on permission-needed). False for
    /// mission and proxy-consumer sessions (pause/fail; never prompt).
    #[serde(default = "default_true")]
    pub interactive: bool,
    /// Origins (`scheme://host`) the user trusted for browser control this
    /// session — mutating `Browser_*` actions on these run without a prompt
    /// (the hard floor still confirms). See `doc/browser-control-spec.md`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub browser_origins: Vec<String>,
}

impl Default for SessionPermissions {
    fn default() -> Self {
        Self {
            path_modes: Vec::new(),
            interactive: true,
            browser_origins: Vec::new(),
        }
    }
}

impl SessionPermissions {
    /// Load from `{session_dir}/permission.json`. Returns default if missing.
    /// Tolerates legacy fields (`policy`, `locked`, `allows`, `denied_sigs`)
    /// — they're ignored, which is the migration.
    pub fn load(session_dir: &Path) -> Self {
        let file = session_dir.join("permission.json");
        if !file.exists() {
            return Self::default();
        }
        match fs::read_to_string(&file) {
            Ok(content) => match serde_json::from_str::<Self>(&content) {
                Ok(p) => p,
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
    /// Prunes child entries that are now redundant or that conflict with a downgrade.
    pub fn set_path_mode(&mut self, path: &str, mode: PermissionMode) {
        let expanded = expand_tilde(path);

        self.path_modes.retain(|pm| {
            if pm.path == path {
                return true;
            }
            let pm_expanded = expand_tilde(&pm.path);
            let is_child = pm_expanded.starts_with(&expanded)
                && (pm_expanded.len() == expanded.len()
                    || pm_expanded.as_bytes().get(expanded.len()) == Some(&b'/'));
            !is_child
        });

        if let Some(existing) = self.path_modes.iter_mut().find(|pm| pm.path == path) {
            existing.mode = mode;
        } else {
            self.path_modes.push(PathMode {
                path: path.to_string(),
                mode,
            });
        }
    }

    /// True when the origin is trusted for mutating browser actions.
    pub fn browser_origin_trusted(&self, origin: &str) -> bool {
        self.browser_origins.iter().any(|o| o == origin)
    }

    /// Trust an origin for browser control for the rest of the session.
    pub fn grant_browser_origin(&mut self, origin: &str) {
        if !self.browser_origin_trusted(origin) {
            self.browser_origins.push(origin.to_string());
        }
    }
}
