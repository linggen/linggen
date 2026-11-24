//! Reading a note and searching, confined to the registered sources.
//!
//! A path is read only when, once links and `..` are resolved, it is a
//! regular note file inside one of a source's roots. Anything else — another
//! file of the person's, a session transcript beside a project's memory
//! folder, a path that does not exist — gets the same refusal, so a caller
//! cannot use this to learn what is on the disk.

use super::redact::{clean, Clean};
use super::sources::SOURCES;
use super::{shorten, Env};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// A note is returned up to this many bytes.
const NOTE_BYTES: u64 = 64 * 1024;
/// Search reads each note up to this many bytes.
const SEARCH_FILE_BYTES: u64 = 256 * 1024;
/// A search looks at most at this many files.
const SEARCH_FILES: usize = 5000;
/// A hit's line, in characters.
const HIT_CHARS: usize = 200;
const NOTE_EXTENSIONS: [&str; 3] = ["md", "markdown", "txt"];

#[derive(Debug, PartialEq, Eq)]
pub enum ReadError {
    NotAbsolute,
    /// Not a note inside a registered source (or not there at all).
    OutsideSources,
    Unreadable(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::NotAbsolute => {
                f.write_str("give the note's absolute path (or one starting with ~/)")
            }
            ReadError::OutsideSources => {
                f.write_str("not a note inside another agent's registered memory")
            }
            ReadError::Unreadable(why) => write!(f, "could not read the note: {why}"),
        }
    }
}

/// One note, as read.
#[derive(Clone, Debug)]
pub struct Note {
    pub source: &'static str,
    pub path: PathBuf,
    pub text: Clean,
    pub truncated: bool,
}

/// One line found by [`search`].
#[derive(Clone, Debug)]
pub struct Hit {
    pub source: &'static str,
    pub path: PathBuf,
    pub line: usize,
    pub text: Clean,
}

/// The note at `path` (absolute, or `~/…`), if it is inside a source.
pub fn read(env: &Env, path: &str) -> Result<Note, ReadError> {
    let wanted = expand(env, path).ok_or(ReadError::NotAbsolute)?;
    let (source, file) = locate(env, &wanted)?;
    let bytes = read_head(&file, NOTE_BYTES)
        .map_err(|e| ReadError::Unreadable(e.kind().to_string()))?;
    let truncated = fs::metadata(&file).is_ok_and(|m| m.len() > NOTE_BYTES);
    Ok(Note {
        source,
        path: file,
        text: clean(&String::from_utf8_lossy(&bytes)),
        truncated,
    })
}

/// Lines of the registered notes that hold the most of `words` (any case,
/// any language: plain substring), best first, then newest file first.
pub fn search(env: &Env, words: &str, limit: usize) -> Vec<Hit> {
    let words: Vec<String> = words.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(usize, u64, Hit)> = Vec::new();
    for (source, file) in note_files(env) {
        let Ok(bytes) = read_head(&file, SEARCH_FILE_BYTES) else {
            continue;
        };
        let changed = fs::metadata(&file)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        for (n, line) in String::from_utf8_lossy(&bytes).lines().enumerate() {
            let lower = line.to_lowercase();
            let found = words.iter().filter(|w| lower.contains(w.as_str())).count();
            if found == 0 {
                continue;
            }
            let hit = Hit {
                source,
                path: file.clone(),
                line: n + 1,
                text: clean(&shorten(line.trim(), HIT_CHARS)),
            };
            scored.push((found, changed, hit));
        }
    }
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.cmp(&a.1))
            .then(a.2.path.cmp(&b.2.path))
            .then(a.2.line.cmp(&b.2.line))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, _, hit)| hit)
        .collect()
}

/// One of another agent's notes, as the chronicle sees it: when it last
/// changed and its first line.
#[derive(Clone, Debug)]
pub struct Touched {
    pub source: &'static str,
    pub path: PathBuf,
    /// Seconds since the epoch the file last changed.
    pub changed: u64,
    pub first_line: Clean,
}

/// Every note of every source with the time it last changed and its first
/// line (a heading's `#` dropped) — what the monthly chronicle reads to say
/// what other agents learned that month. Read-only, like the rest.
pub fn touched(env: &Env) -> Vec<Touched> {
    note_files(env)
        .into_iter()
        .filter_map(|(source, path)| {
            let changed = fs::metadata(&path).and_then(|m| m.modified()).ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
            let head = read_head(&path, 4096).ok()?;
            let text = String::from_utf8_lossy(&head);
            let line = text.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty())?;
            Some(Touched { source, path, changed, first_line: clean(&shorten(line, HIT_CHARS)) })
        })
        .collect()
}

fn expand(env: &Env, path: &str) -> Option<PathBuf> {
    let path = path.trim();
    if let Some(rest) = path.strip_prefix("~/") {
        return Some(env.home().join(rest));
    }
    Path::new(path).is_absolute().then(|| PathBuf::from(path))
}

/// The source `wanted` is inside, and its real path.
fn locate(env: &Env, wanted: &Path) -> Result<(&'static str, PathBuf), ReadError> {
    let real = wanted
        .canonicalize()
        .map_err(|_| ReadError::OutsideSources)?;
    if !real.is_file() || !is_note(&real) {
        return Err(ReadError::OutsideSources);
    }
    for source in SOURCES {
        for root in (source.roots)(env) {
            let Ok(root) = root.canonicalize() else {
                continue;
            };
            if real == root || (root.is_dir() && real.starts_with(&root)) {
                return Ok((source.id, real));
            }
        }
    }
    Err(ReadError::OutsideSources)
}

fn is_note(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| NOTE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Every note file of every source: a root that is a file, or the files in
/// a root folder (two levels down). Links are not followed.
fn note_files(env: &Env) -> Vec<(&'static str, PathBuf)> {
    let mut found = Vec::new();
    for source in SOURCES {
        for root in (source.roots)(env) {
            collect(source.id, &root, 0, &mut found);
        }
    }
    found.truncate(SEARCH_FILES);
    found
}

fn collect(
    source: &'static str,
    path: &Path,
    depth: usize,
    found: &mut Vec<(&'static str, PathBuf)>,
) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if meta.is_file() && is_note(path) {
        found.push((source, path.to_path_buf()));
    } else if meta.is_dir() && depth < 2 {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        children.sort();
        for child in children {
            collect(source, &child, depth + 1, found);
        }
    }
}

fn read_head(path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(max).read_to_end(&mut bytes)?;
    Ok(bytes)
}
