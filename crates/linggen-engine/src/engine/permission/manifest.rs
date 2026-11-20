//! YAML-frontmatter grammar for permission grants declared by extensions
//! (skills + missions). The frontmatter shape is intentionally lenient
//! (mode is a free-form string with a `write→edit` alias); the runtime
//! shape in `model.rs` is the strict enum.
//!
//! Both `SkillPermission` and `MissionPermission` are now type aliases
//! over `Grants` — they're the same thing in disk format and in memory.
//!
//! See `doc/permission-spec.md` → Skill invocation.
use super::model::PermissionMode;
use super::store::SessionPermissions;
use serde::{Deserialize, Serialize};

/// One path grant declared in YAML frontmatter. `mode` is the lenient
/// string form (`"read"`, `"edit"`/`"write"`, `"admin"`); parse to a
/// `PermissionMode` via [`parse_mode_str`] when handing off to the engine.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PathGrant {
    pub path: String,
    pub mode: String,
}

/// Permission block parsed out of skill/mission frontmatter.
///
/// Disk shape:
/// ```yaml
/// permission:
///   paths:
///     - { path: ~/.linggen, mode: write }
///     - { path: /tmp,        mode: read  }
///   warning: "Runs a local HTTP server on 127.0.0.1:4100"
/// ```
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Grants {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<PathGrant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

impl Grants {
    /// Yield `(path, parsed_mode)` for each declared grant.
    pub fn iter_grants(&self) -> impl Iterator<Item = (&str, PermissionMode)> + '_ {
        self.paths
            .iter()
            .map(|g| (g.path.as_str(), parse_mode_str(&g.mode)))
    }
}

/// Parse a frontmatter mode string into the engine enum
/// ([`PermissionMode::parse_written`], so `"write"` is edit, `"chat"` chat).
/// Unknown strings fall back to `Read` — the safest default for a word
/// that names no mode.
pub fn parse_mode_str(m: &str) -> PermissionMode {
    PermissionMode::parse_written(m).unwrap_or(PermissionMode::Read)
}

/// Apply every grant in `perm` to `session` via `set_path_mode`, in order.
/// A session takes its skill's and mission's grants through its preset
/// grants (`session_spec::PresetGrants`), which write them the same way.
pub fn apply_grants(perm: &Grants, session: &mut SessionPermissions) {
    for (path, mode) in perm.iter_grants() {
        session.set_path_mode(path, mode);
    }
}
