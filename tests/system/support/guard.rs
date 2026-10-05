//! The hermetic guard: a system test never touches the person's own Linggen.
//!
//! Every check here panics with `HERMETIC GUARD:` — loudly, before anything
//! starts — when a test world would reach the real `~/.linggen`, the real
//! engine port (9527) or the real ling-mem port (9528).

use std::path::{Path, PathBuf};

/// The ports of the person's own engine and memory daemon.
pub const REAL_PORTS: [u16; 2] = [9527, 9528];

fn real_linggen_home() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".linggen"))
}

fn refuse(what: impl std::fmt::Display) -> ! {
    panic!("HERMETIC GUARD: {what}")
}

/// A test root inside a git repo would hand the engine that repo's project
/// (its CLAUDE.md, its memory scope) — the world must stand on its own.
pub fn outside_git(root: &Path) {
    if let Some(repo) = root.ancestors().find(|d| d.join(".git").exists()) {
        refuse(format!(
            "test root {} is inside the git repo {}",
            root.display(),
            repo.display()
        ));
    }
}

/// A port a test process may listen on or point at.
pub fn port(port: u16, what: &str) {
    if REAL_PORTS.contains(&port) {
        refuse(format!("{what} would use port {port}, a real Linggen port"));
    }
}

/// A path a test world may own: under its root, never the real `~/.linggen`.
pub fn path_in_root(path: &Path, root: &Path, what: &str) {
    if let Some(real) = real_linggen_home() {
        if path.starts_with(&real) {
            refuse(format!(
                "{what} {} is inside the real {}",
                path.display(),
                real.display()
            ));
        }
    }
    if !path.starts_with(root) {
        refuse(format!(
            "{what} {} is outside the test root {}",
            path.display(),
            root.display()
        ));
    }
}

/// The env a child process gets: every path-valued variable inside the root.
pub fn env(vars: &[(String, String)], root: &Path) {
    for (key, value) in vars {
        let is_path = matches!(
            key.as_str(),
            "HOME"
                | "LINGGEN_HOME"
                | "LINGGEN_CONFIG"
                | "TMPDIR"
                | "XDG_CONFIG_HOME"
                | "XDG_DATA_HOME"
                | "XDG_CACHE_HOME"
                | "XDG_STATE_HOME"
        );
        if is_path {
            path_in_root(Path::new(value), root, key);
        }
    }
}

/// A rendered config: no real port anywhere in it.
pub fn config_text(text: &str) {
    for p in REAL_PORTS {
        if text.contains(&format!(":{p}")) {
            refuse(format!("the rendered config names port {p}"));
        }
    }
}

/// The engine's own startup banner names the config it loaded: it must be
/// the test's.
pub fn engine_banner(log: &str, root: &Path) {
    let Some(line) = log.lines().find(|l| l.contains("Config File: ")) else {
        refuse("the engine never named its config file");
    };
    let path = line.split("Config File: ").nth(1).unwrap_or("").trim();
    path_in_root(Path::new(path), root, "the engine's config file");
}
