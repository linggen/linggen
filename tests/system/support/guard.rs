//! The hermetic guard: a system test never touches the person's own Linggen.
//!
//! Every check here panics with `HERMETIC GUARD:` — loudly, before anything
//! starts — when a test world would reach the real `~/.linggen`, the real
//! engine port (9527) or the real ling-mem port (9528).

use super::home::Home;
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
        if key == "LING_MEM_URL" {
            mem_url(value);
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
/// the test's — or, for a world with no config (`has_config` false), none
/// at all ("(default)"), and nothing else.
pub fn engine_banner(log: &str, root: &Path, has_config: bool) {
    let Some(line) = log.lines().find(|l| l.contains("Config File: ")) else {
        refuse("the engine never named its config file");
    };
    let path = line.split("Config File: ").nth(1).unwrap_or("").trim();
    match (has_config, path) {
        (false, "(default)") => {}
        (false, other) => refuse(format!(
            "a world with no config, but the engine loaded the config {other}"
        )),
        (true, "(default)") => refuse("the engine loaded no config, not the test's"),
        (true, path) => path_in_root(Path::new(path), root, "the engine's config file"),
    }
}

/// A world with no config has none — not the rendered one, not any other
/// under its `LINGGEN_HOME/config`.
pub fn no_config(home: &Home) {
    let dir = home.linggen_home().join("config");
    if dir.exists() {
        refuse(format!(
            "a world with no config has a config folder {}",
            dir.display()
        ));
    }
}

/// The ling-mem URL a child is told (`LING_MEM_URL`): loopback, and not
/// the real ling-mem's port.
pub fn mem_url(url: &str) {
    let rest = url
        .strip_prefix("http://127.0.0.1:")
        .unwrap_or_else(|| refuse(format!("LING_MEM_URL {url} is not a loopback http URL")));
    match rest.trim_end_matches('/').parse::<u16>() {
        Ok(p) => port(p, "LING_MEM_URL"),
        Err(_) => refuse(format!("LING_MEM_URL {url} names no port")),
    }
}

/// The rendered config sends memory to the world's own ling-mem — the one
/// address every memory path in the engine reads (`[agent].ling_mem_url`;
/// unset, it would default to 9528).
pub fn config_mem_url(text: &str, mem_url: &str) {
    if !text.contains(&format!("ling_mem_url = \"{mem_url}\"")) {
        refuse(format!(
            "the rendered config does not send memory to the test's ling-mem {mem_url}"
        ));
    }
}

/// Every file of a rendered home: no real port, no path into the real
/// `~/.linggen`. `skip` is a subtree not to read (the linked model cache).
pub fn home_files(home: &Path, skip: &Path) {
    let real = real_linggen_home().map(|p| p.display().to_string());
    for file in files_under(home, skip) {
        let bytes = std::fs::read(&file).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        if let Some(why) = real_reference(&text, real.as_deref()) {
            refuse(format!("{} names {why}", file.display()));
        }
    }
}

/// After a world ran: what the engine logged and what the model was sent
/// never named the real engine or ling-mem.
pub fn after_run(what: &str, text: &str) {
    for p in REAL_PORTS {
        for host in ["127.0.0.1", "localhost"] {
            if text.contains(&format!("{host}:{p}")) {
                refuse(format!("{what} names {host}:{p}, a real Linggen port"));
            }
        }
    }
}

fn real_reference(text: &str, real_home: Option<&str>) -> Option<String> {
    if let Some(p) = REAL_PORTS.iter().find(|p| text.contains(&format!(":{p}"))) {
        return Some(format!("port {p}"));
    }
    real_home
        .filter(|real| text.contains(real))
        .map(|real| format!("the real {real}"))
}

fn files_under(dir: &Path, skip: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if path.starts_with(skip) {
            continue;
        }
        if kind.is_dir() {
            out.extend(files_under(&path, skip));
        } else if kind.is_file() {
            out.push(path);
        } else if kind.is_symlink() {
            link_stays_out(&path);
        }
    }
    out
}

/// A link in a world never leads into the real `~/.linggen`.
fn link_stays_out(link: &Path) {
    let (Some(real), Ok(to)) = (real_linggen_home(), std::fs::read_link(link)) else {
        return;
    };
    if to.starts_with(&real) {
        refuse(format!(
            "{} links into the real {}",
            link.display(),
            real.display()
        ));
    }
}
