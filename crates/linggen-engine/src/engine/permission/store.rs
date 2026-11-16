use super::model::{expand_tilde, PathMode, PermissionMode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::warn;

fn default_true() -> bool {
    true
}

/// Per-session permission state, persisted to `permission.json`, for one
/// place of the session (`turn_place`): its own place's grants are the
/// file's `path_modes`, as they always were; another place's are under
/// `places.<key>` ([`SessionPermissions::load_at`]).
///
/// `path_modes[]` is the place's entire grant table — only explicit user
/// approvals (mode upgrade prompts), mission frontmatter, and skill
/// frontmatter write to it. `interactive` is metadata: false for mission and
/// proxy-consumer sessions so the engine pauses or fails instead of trying
/// to prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPermissions {
    #[serde(default)]
    pub path_modes: Vec<PathMode>,
    /// True for normal user sessions (prompt on permission-needed). False for
    /// mission and proxy-consumer sessions (pause/fail; never prompt).
    #[serde(default = "default_true")]
    pub interactive: bool,
    /// A non-interactive session whose caller answers permission requests
    /// (an MCP `agent_run` delegate): an action beyond the grants waits for
    /// the caller's decision instead of being refused at once.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ask_caller: bool,
    /// The session's own folder (`sessions/<id>/`): readable without a
    /// grant — its background tasks' logs live there. Never saved; set by
    /// [`SessionPermissions::load`] from where the file lives.
    #[serde(skip)]
    pub own_dir: Option<PathBuf>,
    /// The place these grants are, when it is not the session's own: its key
    /// (`TurnPlace::key`). Never saved as a field; it names the slot of the
    /// file the grants are read from and saved to.
    #[serde(skip)]
    pub place: Option<String>,
}

impl Default for SessionPermissions {
    fn default() -> Self {
        Self {
            path_modes: Vec::new(),
            interactive: true,
            ask_caller: false,
            own_dir: None,
            place: None,
        }
    }
}

/// `permission.json` as it is on disk: the session's own place's state at
/// the top, as before places, and every other place's grants by its key.
#[derive(Debug, Default, Serialize, Deserialize)]
struct PermissionFile {
    #[serde(flatten)]
    own: SessionPermissions,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    places: BTreeMap<String, Vec<PathMode>>,
}

impl PermissionFile {
    /// `place`'s view of the file: its grants, with the session's flags.
    fn view(mut self, place: Option<&str>) -> SessionPermissions {
        let Some(key) = place else {
            return self.own;
        };
        SessionPermissions {
            path_modes: self.places.remove(key).unwrap_or_default(),
            place: Some(key.to_string()),
            ..self.own
        }
    }

    /// The file with `perms` written into its place's slot; a place other
    /// than the session's own leaves the top as it was.
    fn with(mut self, perms: &SessionPermissions) -> Self {
        match perms.place.as_deref() {
            None => self.own = SessionPermissions { place: None, ..perms.clone() },
            Some(key) => {
                self.places.insert(key.to_string(), perms.path_modes.clone());
            }
        }
        self
    }
}

impl SessionPermissions {
    /// Load the session's own place's grants from
    /// `{session_dir}/permission.json`. Returns default if missing.
    /// Tolerates legacy fields (`policy`, `locked`, `allows`, `denied_sigs`)
    /// — they're ignored, which is the migration.
    pub fn load(session_dir: &Path) -> Self {
        Self::load_at(session_dir, None)
    }

    /// Load the grants of `place` (a key, `None` for the session's own
    /// place). A file written before places holds only the session's own.
    pub fn load_at(session_dir: &Path, place: Option<&str>) -> Self {
        Self {
            own_dir: Some(session_dir.to_path_buf()),
            ..Self::read_file(session_dir).view(place)
        }
    }

    fn read_file(session_dir: &Path) -> PermissionFile {
        let file = session_dir.join("permission.json");
        if !file.exists() {
            return PermissionFile::default();
        }
        match fs::read_to_string(&file) {
            Ok(content) => Self::parse(&content).unwrap_or_default(),
            Err(e) => {
                warn!("Failed to read session permission.json: {}", e);
                PermissionFile::default()
            }
        }
    }

    fn parse(content: &str) -> Option<PermissionFile> {
        serde_json::from_str::<PermissionFile>(content)
            .map_err(|e| warn!("Failed to parse session permission.json: {}", e))
            .ok()
    }

    /// The grant table of `place` saved in `{session_dir}/permission.json`,
    /// or `None` when there is no readable file — a caller keeps what it
    /// holds rather than mistaking a missing or broken file for "no grants".
    pub fn read_path_modes(session_dir: &Path, place: Option<&str>) -> Option<Vec<PathMode>> {
        let content = fs::read_to_string(session_dir.join("permission.json")).ok()?;
        Self::parse(&content).map(|file| file.view(place).path_modes)
    }

    /// Save to `{session_dir}/permission.json`, into this place's slot: the
    /// other places' grants in the file are kept.
    pub fn save(&self, session_dir: &Path) {
        if let Err(e) = self.try_save(session_dir) {
            warn!("Failed to write session permission.json: {}", e);
        }
    }

    /// Persist an approval, reporting failure to the caller before it is used.
    pub fn try_save(&self, session_dir: &Path) -> std::io::Result<()> {
        fs::create_dir_all(session_dir)?;
        let file = Self::read_file(session_dir).with(self);
        let json = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
        fs::write(session_dir.join("permission.json"), json)
    }

    /// Add or update a path-mode grant. If a grant for the exact path exists, update it.
    /// Prunes child entries that are now redundant or that conflict with a downgrade.
    pub fn set_path_mode(&mut self, path: &str, mode: PermissionMode) {
        // A session-wide change replaces all prior, more-specific grants.
        // Later concrete grants may still override this baseline.
        if path == "*" {
            self.path_modes.clear();
            self.path_modes.push(PathMode {
                path: path.to_string(),
                mode,
            });
            return;
        }
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
}
