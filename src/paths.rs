use std::path::PathBuf;
use std::sync::OnceLock;

static LINGGEN_HOME: OnceLock<PathBuf> = OnceLock::new();

/// Returns the Linggen home directory (`~/.linggen/`).
/// Supports `$LINGGEN_HOME` env override. Cached via `OnceLock`.
pub fn linggen_home() -> &'static PathBuf {
    LINGGEN_HOME.get_or_init(|| {
        if let Ok(val) = std::env::var("LINGGEN_HOME") {
            let p = PathBuf::from(val);
            if !p.as_os_str().is_empty() {
                return p;
            }
        }
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".linggen")
    })
}

/// `~/.linggen/config/`
pub fn config_dir() -> PathBuf {
    linggen_home().join("config")
}

/// `~/.linggen/logs/`
pub fn logs_dir() -> PathBuf {
    linggen_home().join("logs")
}

/// `~/.linggen/agents/`
pub fn global_agents_dir() -> PathBuf {
    linggen_home().join("agents")
}

/// Process-wide override for the skill discovery directory. Set once at
/// startup by the `--skills-dir` CLI flag; bundled apps point at their
/// sealed `Resources/skills/` so they don't load the user's global skills.
static SKILLS_DIR_OVERRIDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

pub fn set_skills_dir_override(path: PathBuf) {
    let _ = SKILLS_DIR_OVERRIDE.set(path);
}

/// `~/.linggen/skills/` (or the `--skills-dir` override if set).
pub fn global_skills_dir() -> PathBuf {
    if let Some(p) = SKILLS_DIR_OVERRIDE.get() {
        return p.clone();
    }
    linggen_home().join("skills")
}

/// Compat skill dirs with labels: `~/.claude/skills/` → "Claude", `~/.codex/skills/` → "Codex"
pub fn compat_skills_dirs() -> Vec<(PathBuf, &'static str)> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    vec![
        (home.join(".claude/skills"), "Claude"),
        (home.join(".codex/skills"), "Codex"),
    ]
}

/// `~/.linggen/sessions/` — flat directory for all sessions
pub fn global_sessions_dir() -> PathBuf {
    linggen_home().join("sessions")
}

/// `~/.linggen/missions/`
pub fn global_missions_dir() -> PathBuf {
    linggen_home().join("missions")
}


/// `~/.linggen/memory/` — root of the memory tree.
///
/// Holds the two built-in files (`identity.md`, `style.md`) inlined into
/// the stable system prompt, plus per-skill data under
/// `~/.linggen/memory/<skill-name>/` for installed memory providers.
/// See `doc/memory-spec.md`.
pub fn memory_dir() -> PathBuf {
    linggen_home().join("memory")
}


/// Resolve the workspace root: the explicit `--root` argument when given,
/// otherwise the cwd canonicalized to the nearest enclosing git root (or
/// cwd itself when not in a repo).
pub fn resolve_workspace_root(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p);
    }
    let cwd = std::env::current_dir()?;
    Ok(crate::engine::tools::search_exec_find_git_root(&cwd).unwrap_or(cwd))
}
