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
///   warning: "Runs a local HTTP daemon on 127.0.0.1:9888"
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

    /// Human-readable summary for approval prompts: `~/foo (write), /tmp (read)`.
    pub fn display_paths(&self) -> String {
        self.paths
            .iter()
            .map(|g| format!("{} ({})", g.path, g.mode))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Parse a frontmatter mode string into the engine enum. Accepts `"write"`
/// as an alias for `"edit"` because users naturally write "write" in YAML
/// even though the engine's internal vocabulary says "edit". Unknown
/// strings fall back to `Read` — the safest default.
pub fn parse_mode_str(m: &str) -> PermissionMode {
    match m {
        "edit" | "write" => PermissionMode::Edit,
        "admin" => PermissionMode::Admin,
        _ => PermissionMode::Read,
    }
}

/// Apply every grant in `perm` to `session` via `set_path_mode`. The only
/// legitimate writer into `path_modes[]` for skill/mission frontmatter
/// (the other two are explicit user approvals and the load path).
pub fn apply_grants(perm: &Grants, session: &mut SessionPermissions) {
    for (path, mode) in perm.iter_grants() {
        session.set_path_mode(path, mode);
    }
}
