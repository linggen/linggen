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
