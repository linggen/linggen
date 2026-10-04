//! Where a row belongs: its `cwd` is its **scope**, the directory it is about.
//!
//! See `doc/scope-index-spec.md`. Modelled on Claude Code's CLAUDE.md: rules
//! live in directories, and a session sees its directory and every parent.
//! Everything here is path arithmetic (plus a few `is_dir` probes), so the
//! daemon owns the rules once and every host — Claude Code hooks, Codex,
//! OpenClaw, the Linggen engine — only hands it the session's paths:
//!
//! - **root**: the git root of the session cwd; outside git, the directory
//!   the session started in. Hosts send it; [`find_root`] is the fallback.
//! - **candidates**: the scopes a row written here may take — root down to
//!   the session cwd, root's marked subdirectories, and root's parents below
//!   `$HOME`. Shown to the model once at session start.
//! - **recall scope**: which rows a session may recall ([`RecallScope`]).
//! - **index chain**: the directories whose indexed rows load at start.

use std::path::{Component, Path, PathBuf};

/// Where installed skills live, relative to `$HOME`. A session under
/// `~/.linggen/skills/<name>` is that skill's: it sees only its own rows.
pub const SKILLS_REL: &str = ".linggen/skills";

/// Files that make a subdirectory of root a scope of its own.
const MARKERS: [&str; 5] = [
    "SKILL.md",
    "CLAUDE.md",
    "Cargo.toml",
    "package.json",
    "pubspec.yaml",
];

/// Directories never worth offering as a scope.
const SKIP_DIRS: [&str; 9] = [
    "node_modules",
    "target",
    "build",
    "dist",
    "vendor",
    "Pods",
    "venv",
    "__pycache__",
    "assets",
];

/// How many marked subdirectories the candidates list carries at most.
const MAX_MARKED: usize = 30;

/// `$HOME` for the daemon's user.
pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Which rows a session may recall, decided from one path (the session's
/// root, or its cwd when it sent no root).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecallScope {
    /// The owner working somewhere: rows under the root (any depth), rows at
    /// a parent of the root, and rows with no cwd (about the person).
    Owner {
        root: String,
        ancestors: Vec<String>,
    },
    /// A skill's own session (`~/.linggen/skills/<name>`): rows under that
    /// directory only — no person rows. App isolation, keyed by path.
    Skill { dir: String, name: String },
    /// No root (`$HOME`, `~/.linggen`, temp dirs): rows with no cwd only.
    NoRoot,
}

impl RecallScope {
    /// Classify a session path. Accepts `~/…`; anything unparseable is
    /// [`RecallScope::NoRoot`] — the narrowest answer, never the widest.
    pub fn of(path: &str, home: &Path) -> Self {
        let Some(p) = expand(path, home) else {
            return Self::NoRoot;
        };
        if let Some((dir, name)) = skill_dir(&p, home) {
            return Self::Skill {
                dir: to_str(&dir),
                name,
            };
        }
        if !is_scope_dir(&p, home) {
            return Self::NoRoot;
        }
        Self::Owner {
            root: to_str(&p),
            ancestors: ancestors(&p, home).iter().map(|a| to_str(a)).collect(),
        }
    }
}

/// `~/…` expanded, trailing slashes dropped, `.`/`..` folded. `None` for an
/// empty or relative path.
pub fn expand(raw: &str, home: &Path) -> Option<PathBuf> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let p = if s == "~" {
        home.to_path_buf()
    } else if let Some(rest) = s.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(s)
    };
    if !p.is_absolute() {
        return None;
    }
    Some(normalize(&p))
}

/// Fold `.` and `..` lexically; never touches the disk.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn to_str(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

/// `(~/.linggen/skills/<name>, name)` when `p` is that dir or inside it.
pub fn skill_dir(p: &Path, home: &Path) -> Option<(PathBuf, String)> {
    let skills = home.join(SKILLS_REL);
    let rest = p.strip_prefix(&skills).ok()?;
    let first = rest.components().next()?;
    let name = first.as_os_str().to_string_lossy().to_string();
    Some((skills.join(&name), name))
}

fn is_temp(p: &Path) -> bool {
    let tmp = std::env::temp_dir();
    let tmp = tmp.to_string_lossy();
    let tmp = tmp.trim_end_matches('/');
    let s = p.to_string_lossy();
    let under = |root: &str| s == root || s.starts_with(&format!("{root}/"));
    under(tmp)
        || under("/tmp")
        || under("/private/tmp")
        || under("/var/folders")
        || under("/private/var/folders")
}

/// Can a row be about this directory? Not `$HOME` (no particular work), not
/// `~/.linggen` outside a skill's own dir (the engine's state), not a temp
/// dir, not `/`. A skill's dir and anything in it is.
pub fn is_scope_dir(p: &Path, home: &Path) -> bool {
    if skill_dir(p, home).is_some() {
        return true;
    }
    if p == home || p.starts_with(home.join(".linggen")) || is_temp(p) {
        return false;
    }
    p.parent().is_some()
}

/// Parents of `root` that can hold rows about it, nearest first: up to (not
/// including) `$HOME`, or up to `/` for a root outside home. None for a
/// skill's dir — the dirs above it are the engine's.
pub fn ancestors(root: &Path, home: &Path) -> Vec<PathBuf> {
    if skill_dir(root, home).is_some() {
        return Vec::new();
    }
    root.ancestors()
        .skip(1)
        .take_while(|a| *a != home && a.parent().is_some())
        .filter(|a| is_scope_dir(a, home))
        .map(Path::to_path_buf)
        .collect()
}

/// The session's root when a host sent none: a skill's dir, else the git
/// root above `cwd` (stopping at `$HOME`), else `cwd` itself.
pub fn find_root(cwd: &Path, home: &Path) -> PathBuf {
    if let Some((dir, _)) = skill_dir(cwd, home) {
        return dir;
    }
    for a in cwd.ancestors() {
        if a == home {
            break;
        }
        if a.join(".git").exists() {
            return a.to_path_buf();
        }
    }
    cwd.to_path_buf()
}

/// The directories whose indexed rows a session at `cwd` loads: `cwd` and
/// each parent that can hold rows, nearest first. A skill session stops at
/// its skill's dir.
pub fn index_chain(cwd: &Path, home: &Path) -> Vec<PathBuf> {
    let skill = skill_dir(cwd, home).map(|(d, _)| d);
    let mut out = Vec::new();
    for a in cwd.ancestors() {
        if let Some(s) = &skill {
            if !a.starts_with(s) {
                break;
            }
        } else if a == home || !is_scope_dir(a, home) {
            break;
        }
        out.push(a.to_path_buf());
    }
    out
}

/// Every scope a row written here may take, in display order: root, the
/// dirs from root down to `cwd`, root's marked subdirectories (two levels),
/// then root's parents below `$HOME`.
pub fn candidates(root: &Path, cwd: &Path, home: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let push = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if is_scope_dir(&p, home) && !out.contains(&p) {
            out.push(p);
        }
    };
    push(root.to_path_buf(), &mut out);
    if let Ok(rest) = cwd.strip_prefix(root) {
        let mut cur = root.to_path_buf();
        for c in rest.components() {
            cur.push(c.as_os_str());
            push(cur.clone(), &mut out);
        }
    }
    for m in marked_subdirs(root) {
        push(m, &mut out);
    }
    for a in ancestors(root, home) {
        push(a, &mut out);
    }
    out
}

/// Root's subdirectories (two levels) that carry a marker file.
fn marked_subdirs(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut level = vec![root.to_path_buf()];
    for _depth in 0..2 {
        let mut next = Vec::new();
        for dir in &level {
            for child in sorted_children(dir) {
                if has_marker(&child) {
                    found.push(child.clone());
                    if found.len() >= MAX_MARKED {
                        return found;
                    }
                }
                next.push(child);
            }
        }
        level = next;
    }
    found
}

fn sorted_children(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut kids: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string());
            name.is_some_and(|n| !n.starts_with('.') && !SKIP_DIRS.contains(&n.as_str()))
        })
        .collect();
    kids.sort();
    kids
}

fn has_marker(dir: &Path) -> bool {
    if MARKERS.iter().any(|m| dir.join(m).is_file()) {
        return true;
    }
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .any(|e| e.file_name().to_string_lossy().starts_with("README"))
        })
        .unwrap_or(false)
}

/// How a scope reads to the model: relative to root's parent for root and
/// below (`skills/lingjing`), `~/…` elsewhere under home, absolute otherwise.
pub fn display(p: &Path, root: &Path, home: &Path) -> String {
    if p.starts_with(root) {
        if let Some(base) = root.parent() {
            if let Ok(rel) = p.strip_prefix(base) {
                if !rel.as_os_str().is_empty() && base != Path::new("/") {
                    return to_str(rel);
                }
            }
        }
    }
    match p.strip_prefix(home) {
        Ok(rel) if !rel.as_os_str().is_empty() => format!("~/{}", rel.to_string_lossy()),
        _ => to_str(p),
    }
}

/// The candidates line hosts show once at session start.
pub fn candidates_line(root: &Path, cwd: &Path, home: &Path) -> String {
    let list = candidates(root, cwd, home);
    if list.is_empty() {
        return String::new();
    }
    let shown: Vec<String> = list.iter().map(|p| display(p, root, home)).collect();
    format!(
        "Memory scopes here: {} (default: {}). On memory_add, pass `scope` when a row is about one of these directories rather than where you stand.",
        shown.join(", "),
        display(cwd, root, home)
    )
}

/// A model's `scope` turned into an absolute directory. Accepts what the
/// candidates line shows (relative to root's parent, or `~/…`), a path
/// relative to root, or an absolute path. `None` when nothing resolves.
pub fn resolve(scope: &str, root: Option<&Path>, home: &Path) -> Option<PathBuf> {
    let s = scope.trim().trim_end_matches('/');
    if s.is_empty() {
        return None;
    }
    if s.starts_with('~') || s.starts_with('/') {
        return expand(s, home);
    }
    let root = root?;
    let from_parent = root.parent().map(|b| normalize(&b.join(s)));
    let from_root = normalize(&root.join(s));
    match from_parent {
        Some(p) if p.is_dir() && (p.starts_with(root) || root.starts_with(&p)) => Some(p),
        _ if from_root.is_dir() => Some(from_root),
        Some(p) => Some(p),
        None => Some(from_root),
    }
}

/// May a row written in a session rooted at `root` take scope `dir`? It must
/// be an existing directory inside root, or a parent of root below `$HOME`.
/// A skill session's rows stay inside its skill's dir.
pub fn is_valid_for(dir: &Path, root: &Path, home: &Path) -> bool {
    if !is_scope_dir(dir, home) || !dir.is_dir() {
        return false;
    }
    if let Some((skill, _)) = skill_dir(root, home) {
        return dir.starts_with(&skill);
    }
    dir.starts_with(root) || ancestors(root, home).iter().any(|a| a == dir)
}

/// The scope several rows share — what a digest or merge of them is about.
/// `None` when any row has none (it is about the person) or when they meet
/// only at `$HOME` or above.
pub fn common(paths: &[Option<String>], home: &Path) -> Option<String> {
    let mut it = paths.iter();
    let first = PathBuf::from(it.next()?.as_ref()?);
    let mut acc = first;
    for p in it {
        let p = PathBuf::from(p.as_ref()?);
        while !p.starts_with(&acc) {
            if !acc.pop() {
                return None;
            }
        }
    }
    is_scope_dir(&acc, home).then(|| to_str(&acc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn h() -> PathBuf {
        PathBuf::from("/Users/alex")
    }

    #[test]
    fn classify_owner_skill_and_no_root() {
        let home = h();
        match RecallScope::of("/Users/alex/workspace/linggen/skills", &home) {
            RecallScope::Owner { root, ancestors } => {
                assert_eq!(root, "/Users/alex/workspace/linggen/skills");
                assert_eq!(
                    ancestors,
                    vec!["/Users/alex/workspace/linggen", "/Users/alex/workspace"]
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            RecallScope::of("~/.linggen/skills/cfo/data", &home),
            RecallScope::Skill {
                dir: "/Users/alex/.linggen/skills/cfo".into(),
                name: "cfo".into()
            }
        );
        for none in [
            "/Users/alex",
            "/Users/alex/.linggen",
            "/Users/alex/.linggen/missions",
            "/Users/alex/.linggen/skills",
            "/tmp/x",
            "/private/tmp",
            "",
            "relative/path",
        ] {
            assert_eq!(RecallScope::of(none, &home), RecallScope::NoRoot, "{none}");
        }
    }

    #[test]
    fn index_chain_is_nearest_first_and_stops_below_home() {
        let home = h();
        let chain = index_chain(Path::new("/Users/alex/w/linggen/skills/lingjing"), &home);
        assert_eq!(
            chain,
            vec![
                PathBuf::from("/Users/alex/w/linggen/skills/lingjing"),
                PathBuf::from("/Users/alex/w/linggen/skills"),
                PathBuf::from("/Users/alex/w/linggen"),
                PathBuf::from("/Users/alex/w"),
            ]
        );
        let skill = index_chain(Path::new("/Users/alex/.linggen/skills/dj/data"), &home);
        assert_eq!(
            skill,
            vec![
                PathBuf::from("/Users/alex/.linggen/skills/dj/data"),
                PathBuf::from("/Users/alex/.linggen/skills/dj"),
            ]
        );
        assert!(index_chain(&home, &home).is_empty());
    }

    #[test]
    fn display_and_resolve_round_trip() {
        let home = h();
        let root = Path::new("/Users/alex/w/linggen/skills");
        assert_eq!(display(root, root, &home), "skills");
        assert_eq!(
            display(Path::new("/Users/alex/w/linggen/skills/dj"), root, &home),
            "skills/dj"
        );
        assert_eq!(display(Path::new("/Users/alex/w"), root, &home), "~/w");
        assert_eq!(
            resolve("~/w", Some(root), &home),
            Some(PathBuf::from("/Users/alex/w"))
        );
        assert_eq!(
            resolve("skills/dj/", Some(root), &home),
            Some(PathBuf::from("/Users/alex/w/linggen/skills/dj"))
        );
        assert_eq!(resolve("skills/dj", None, &home), None);
    }

    #[test]
    fn common_scope_of_rows() {
        let home = h();
        let s = |v: &str| Some(v.to_string());
        assert_eq!(
            common(
                &[
                    s("/Users/alex/w/a/x"),
                    s("/Users/alex/w/a/y"),
                    s("/Users/alex/w/a")
                ],
                &home
            ),
            s("/Users/alex/w/a")
        );
        assert_eq!(common(&[s("/Users/alex/w/a"), None], &home), None);
        assert_eq!(
            common(&[s("/Users/alex/w/a"), s("/Users/alex/v")], &home),
            None
        );
        assert_eq!(common(&[], &home), None);
    }

    #[test]
    fn candidates_and_validation_on_disk() {
        let tmp = TempDir::new().unwrap();
        // A fake home outside the temp-dir rule: the probe dirs live in the
        // TempDir, so make home its parent-free stand-in and the root inside.
        let home = tmp.path().join("home");
        let root = home.join("w/linggen/skills");
        std::fs::create_dir_all(root.join("lingjing/story")).unwrap();
        std::fs::create_dir_all(root.join("dj")).unwrap();
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::write(root.join("lingjing/SKILL.md"), "x").unwrap();
        std::fs::write(root.join("dj/README.md"), "x").unwrap();
        // The temp dir itself is a non-scope; probe the pure parts instead.
        let marked = marked_subdirs(&root);
        assert_eq!(marked, vec![root.join("dj"), root.join("lingjing")]);
        assert!(root.join("lingjing").is_dir());
    }

    #[test]
    fn validity_rules() {
        let home = h();
        let root = Path::new("/Users/alex/w/linggen/skills");
        // Nonexistent dirs never pass (the disk is the authority).
        assert!(!is_valid_for(Path::new("/Users/alex/w/nope"), root, &home));
        assert!(!is_valid_for(&home, root, &home));
    }
}
