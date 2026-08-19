pub fn now_ts_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn now_ts_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Bin directories a user's shell has but launchd's stock
/// `PATH=/usr/bin:/bin:/usr/sbin:/sbin` does not.
const EXTRA_BIN_DIRS: [&str; 5] = [
    "~/.local/bin",
    "/usr/local/bin",
    "/usr/local/sbin",
    "/opt/homebrew/bin",
    "/opt/homebrew/sbin",
];

/// `$PATH` for shell children, with the user bin dirs above appended.
///
/// A GUI-launched daemon (Linggen.app → launchd) inherits launchd's stock
/// PATH and hands it to every `sh -c` child, so tools the usual installers
/// drop — `ling-mem` in `~/.local/bin`, anything from Homebrew — are simply
/// not found. A daemon started from a terminal inherits the full interactive
/// PATH and works, which is what makes the failure look intermittent.
///
/// Appended, never prepended: system binaries keep the precedence they
/// already have, so this only resolves commands that would otherwise fail.
/// Computed once — the directory set doesn't change while the daemon runs.
pub fn shell_path() -> &'static str {
    static PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let inherited = std::env::var("PATH").unwrap_or_default();
        let mut dirs: Vec<String> = inherited
            .split(':')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect();
        for extra in EXTRA_BIN_DIRS {
            let resolved = resolve_path(std::path::Path::new(extra));
            if !resolved.is_dir() {
                continue;
            }
            let dir = resolved.to_string_lossy().to_string();
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        // Managed-runtime bins go last, and unconditionally: the prewarm
        // task creates them while the daemon runs, so an is_dir() gate here
        // would leave a first boot without them until restart.
        for dir in crate::runtime::path_dirs() {
            let dir = dir.to_string_lossy().to_string();
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        dirs.join(":")
    })
}

/// Resolve a path. Expands a leading `~` to `$HOME` first, then follows
/// symlinks via `canonicalize`. On macOS, `/tmp` → `/private/tmp`.
///
/// Falls back to the expanded-but-not-canonicalized path when the target
/// doesn't exist, so callers that want a best-effort absolute path (e.g.
/// mission cwd before directories are created) still get a usable value.
pub fn resolve_path(path: &std::path::Path) -> std::path::PathBuf {
    let expanded: std::path::PathBuf = {
        let s = path.to_string_lossy();
        if let Some(rest) = s.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                home.join(rest)
            } else {
                path.to_path_buf()
            }
        } else if s == "~" {
            dirs::home_dir().unwrap_or_else(|| path.to_path_buf())
        } else {
            path.to_path_buf()
        }
    };
    expanded.canonicalize().unwrap_or(expanded)
}
