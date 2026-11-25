//! The one file that touches the disk. Reading only: nothing here writes,
//! moves or removes (`tests::this_module_never_writes`), and no other file
//! of this module reads a file or lists a folder
//! (`tests::only_disk_reads_the_disk`).
//!
//! A note of another agent is read only through a [`Located`]: a file the
//! registry found inside a declared place ([`super::readers::Registry`]).
//! Folders are walked by a place's declared path pattern and nothing wider;
//! a link is never followed past a literal segment, and a name that starts
//! with a dot is never matched by a `*`.

use super::decl::{wild_matches, Pattern, Segment};
use crate::project_root::find_git_root;
use super::readers::Located;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// A pattern yields at most this many files.
const MAX_FILES_PER_PATTERN: usize = 2000;

/// Which folders `{project}` stands for.
pub(crate) enum Projects<'a> {
    /// These folder names, each with its distance from the session's folder.
    Chain(&'a [(usize, String)]),
    /// Any folder (read and search reach every project's notes).
    Any,
}

/// A file found by a pattern, and which chain folder it was for.
pub(crate) struct Expanded {
    pub path: PathBuf,
    /// The distance of the chain folder `{project}` was, when it was one.
    pub distance: Option<usize>,
}

/// The note files `pattern` names under `home`, in a stable order.
pub(crate) fn expand(home: &Path, pattern: &Pattern, projects: &Projects) -> Vec<Expanded> {
    let mut at: Vec<Expanded> = vec![Expanded { path: home.to_path_buf(), distance: None }];
    let last = pattern.segments.len() - 1;
    for (n, segment) in pattern.segments.iter().enumerate() {
        let mut next = Vec::new();
        for here in &at {
            match segment {
                Segment::Literal(name) => next.push(Expanded { path: here.path.join(name), distance: here.distance }),
                Segment::Project => match projects {
                    Projects::Chain(chain) => {
                        next.extend(chain.iter().map(|(d, name)| Expanded { path: here.path.join(name), distance: Some(*d) }))
                    }
                    Projects::Any => next.extend(children(&here.path, "*", n < last, here.distance)),
                },
                Segment::Wild(wild) => next.extend(children(&here.path, wild, n < last, here.distance)),
            }
        }
        at = next;
    }
    at.retain(|e| fs::symlink_metadata(&e.path).is_ok_and(|m| m.is_file()));
    at.sort_by(|a, b| a.path.cmp(&b.path));
    at.truncate(MAX_FILES_PER_PATTERN);
    at
}

/// The entries of `dir` whose names match, folders when `folders`, else
/// files. Links are skipped.
fn children(dir: &Path, wild: &str, folders: bool, distance: Option<usize>) -> Vec<Expanded> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|name| wild_matches(wild, name)))
        .filter(|e| e.file_type().is_ok_and(|t| if folders { t.is_dir() } else { t.is_file() }))
        .map(|e| Expanded { path: e.path(), distance })
        .collect()
}

/// The real path of `path`, links and `..` resolved.
pub(crate) fn canonical(path: &Path) -> Option<PathBuf> {
    path.canonicalize().ok()
}

/// A repository lists at most this many other worktrees (agents leave dozens).
const MAX_WORKTREES: usize = 64;

/// The other worktrees of the git repository one of `dirs` stands in: the
/// main worktree first, then the linked ones by path. `dirs` is a session's
/// folder chain, nearest first; the repository's root is the engine's one
/// project-root walk (`project_root::find_git_root`). Empty outside a repository, in a
/// submodule, and when nothing else is checked out.
///
/// Read from the `.git` files, no `git` process: a linked worktree's `.git`
/// file names its admin folder (`gitdir: <common>/worktrees/<name>`), whose
/// `commondir` file points to the shared `.git`; that folder's `worktrees/*/gitdir`
/// files name every linked worktree's `.git`. Folders that are gone are left out.
pub(crate) fn sibling_worktrees(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let Some(common) = dirs.iter().find_map(|d| find_git_root(d)).and_then(|root| common_git_dir(&root)) else {
        return Vec::new();
    };
    let main = (common.file_name().is_some_and(|n| n == ".git")).then(|| common.parent()).flatten().map(Path::to_path_buf);
    let mut linked: Vec<PathBuf> = fs::read_dir(common.join("worktrees"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|admin| linked_root(&admin.path()))
        .collect();
    linked.sort();
    let mut roots: Vec<PathBuf> = main.into_iter().chain(linked).filter(|r| r.is_dir()).collect();
    roots.truncate(MAX_WORKTREES);
    roots
}

/// The shared `.git` folder of the worktree at `root` (which holds a `.git`).
fn common_git_dir(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    if fs::symlink_metadata(&dot_git).ok()?.is_dir() {
        return canonical(&dot_git);
    }
    let admin = root.join(small_text(&dot_git)?.strip_prefix("gitdir:")?.trim());
    // A submodule's admin folder has no `commondir`: not a worktree.
    let common = admin.join(small_text(&admin.join("commondir"))?.trim());
    canonical(&common)
}

/// The folder a worktree's admin folder (`<common>/worktrees/<name>`) belongs to.
fn linked_root(admin: &Path) -> Option<PathBuf> {
    let text = small_text(&admin.join("gitdir"))?;
    let dot_git = admin.join(text.trim());
    let root = dot_git.parent()?;
    // Git writes an absolute path; keep it as written. A relative one is resolved.
    Path::new(text.trim()).is_absolute().then(|| root.to_path_buf()).or_else(|| canonical(root))
}

/// A small plain file's text (at most 4 KB), or `None`.
fn small_text(path: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    fs::File::open(path).ok()?.take(4096).read_to_end(&mut bytes).ok()?;
    String::from_utf8(bytes).ok()
}

/// The first `max` bytes of a note.
pub(crate) fn read_note(file: &Located, max: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(file.path())?.take(max).read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub(crate) fn size_of(file: &Located) -> u64 {
    fs::metadata(file.path()).map_or(0, |m| m.len())
}

/// Seconds since the epoch the note last changed.
pub(crate) fn changed(file: &Located) -> u64 {
    fs::metadata(file.path())
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// One installed skill, for the readers it may carry.
pub(crate) struct SkillFolder {
    pub name: String,
    pub dir: PathBuf,
    /// The head of its `SKILL.md`; `None` when there is none.
    pub head: Option<String>,
}

/// The skills under `skills_dir` (a folder, not a link), by name. Only the
/// head of each `SKILL.md` is read: the frontmatter is at the top.
pub(crate) fn skill_folders(skills_dir: &Path) -> Vec<SkillFolder> {
    let Ok(entries) = fs::read_dir(skills_dir) else {
        return Vec::new();
    };
    let mut out: Vec<SkillFolder> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            let dir = e.path();
            let head = fs::File::open(dir.join("SKILL.md")).ok().map(|f| {
                let mut bytes = Vec::new();
                let _ = f.take(16 * 1024).read_to_end(&mut bytes);
                String::from_utf8_lossy(&bytes).into_owned()
            });
            Some(SkillFolder { name, dir, head })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// A reader file a skill names: a plain file inside the skill's own folder
/// (not a link, not a path that leaves it), at most 64 KB.
pub(crate) fn reader_text(skill_dir: &Path, relative: &str) -> Result<String, String> {
    let named = Path::new(relative);
    let plain = !relative.is_empty() && named.components().all(|c| matches!(c, std::path::Component::Normal(_)));
    if !plain {
        return Err(format!("`{relative}` must be a path inside the skill"));
    }
    let file = skill_dir.join(named);
    let meta = fs::symlink_metadata(&file).map_err(|_| format!("`{relative}` is not there"))?;
    if !meta.is_file() || meta.len() > 64 * 1024 {
        return Err(format!("`{relative}` is not a plain file of 64 KB or less"));
    }
    fs::read_to_string(&file).map_err(|e| format!("could not read `{relative}`: {}", e.kind()))
}
