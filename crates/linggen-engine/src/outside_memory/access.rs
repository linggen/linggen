//! Reading a note and searching, confined to the declared places.
//!
//! A path is read only when, once links and `..` are resolved, it is a note
//! file one of the registry's places names ([`Registry::locate`]). Anything
//! else — another file of the person's, a session transcript beside a
//! project's memory folder, a path that does not exist — gets the same
//! refusal, so a caller cannot use this to learn what is on the disk. The
//! files themselves are read by [`super::disk`] alone.

use super::disk;
use super::readers::{Agent, Located, Registry};
use super::redact::{clean, Clean};
use super::{shorten, Env};
use std::path::{Path, PathBuf};

/// A note is returned up to this many bytes.
const NOTE_BYTES: u64 = 64 * 1024;
/// Search reads each note up to this many bytes.
const SEARCH_FILE_BYTES: u64 = 256 * 1024;
/// A hit's line, in characters.
const HIT_CHARS: usize = 200;

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
    pub source: String,
    pub path: PathBuf,
    pub text: Clean,
    pub truncated: bool,
}

/// One line found by [`search`].
#[derive(Clone, Debug)]
pub struct Hit {
    pub source: String,
    pub path: PathBuf,
    pub line: usize,
    pub text: Clean,
}

/// The note at `path` (absolute, or `~/…`), if it is inside a declared place.
pub fn read(env: &Env, path: &str) -> Result<Note, ReadError> {
    let wanted = expand(env, path).ok_or(ReadError::NotAbsolute)?;
    let file = Registry::load(env).locate(&wanted).ok_or(ReadError::OutsideSources)?;
    let bytes = disk::read_note(&file, NOTE_BYTES).map_err(|e| ReadError::Unreadable(e.kind().to_string()))?;
    Ok(Note {
        source: file.source().to_string(),
        path: file.path().to_path_buf(),
        text: clean(&String::from_utf8_lossy(&bytes)),
        truncated: disk::size_of(&file) > NOTE_BYTES,
    })
}

/// Lines of the readable notes that hold the most of `words` (any case,
/// any language: plain substring), best first, then newest file first.
pub fn search(env: &Env, words: &str, limit: usize) -> Vec<Hit> {
    let words: Vec<String> = words.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(usize, u64, Hit)> = Vec::new();
    for file in Registry::load(env).files() {
        let Ok(bytes) = disk::read_note(&file, SEARCH_FILE_BYTES) else {
            continue;
        };
        let changed = disk::changed(&file);
        for (n, line) in String::from_utf8_lossy(&bytes).lines().enumerate() {
            let lower = line.to_lowercase();
            let found = words.iter().filter(|w| lower.contains(w.as_str())).count();
            if found == 0 {
                continue;
            }
            let hit = Hit {
                source: file.source().to_string(),
                path: file.path().to_path_buf(),
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
    scored.into_iter().take(limit).map(|(_, _, hit)| hit).collect()
}

/// One of another agent's notes, as the chronicle sees it: when it last
/// changed and its first line.
#[derive(Clone, Debug)]
pub struct Touched {
    pub source: String,
    pub path: PathBuf,
    /// Seconds since the epoch the file last changed.
    pub changed: u64,
    pub first_line: Clean,
}

/// Every readable note with the time it last changed and its first line (a
/// heading's `#` dropped) — what the monthly chronicle reads to say what
/// other agents learned that month. Read-only, like the rest.
pub fn touched(env: &Env) -> Vec<Touched> {
    Registry::load(env)
        .files()
        .into_iter()
        .filter_map(|file| {
            let head = disk::read_note(&file, 4096).ok()?;
            let text = String::from_utf8_lossy(&head);
            let line = text.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty())?;
            Some(Touched {
                source: file.source().to_string(),
                path: file.path().to_path_buf(),
                changed: disk::changed(&file),
                first_line: clean(&shorten(line, HIT_CHARS)),
            })
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

/// One note file a person can open, as the page lists it.
#[derive(Clone, Debug)]
pub struct NoteFile {
    pub path: PathBuf,
    pub name: String,
    pub bytes: u64,
    /// Seconds since the epoch.
    pub changed: u64,
}

/// A folder of an agent's notes.
#[derive(Clone, Debug)]
pub struct Place {
    pub root: PathBuf,
    pub files: Vec<NoteFile>,
}

/// One agent's memory as it is on this machine: every folder the registry
/// reads for it.
#[derive(Clone, Debug)]
pub struct AgentMemory {
    pub agent: Agent,
    pub places: Vec<Place>,
}

/// The agents the registry reads, each with the note files found now —
/// listed in the registry's order, an agent once. A listing only: the
/// files are read with [`read`].
pub fn listing(env: &Env) -> Vec<AgentMemory> {
    let registry = Registry::load(env);
    let mut out: Vec<AgentMemory> = registry.readers.iter().map(|r| AgentMemory { agent: r.agent(), places: Vec::new() }).collect();
    for file in registry.files() {
        let Some(memory) = out.iter_mut().find(|m| m.agent == *file.agent()) else {
            continue;
        };
        let root = file.path().parent().map(Path::to_path_buf).unwrap_or_default();
        let note = note_file(&file);
        match memory.places.iter_mut().find(|p| p.root == root) {
            Some(place) => place.files.push(note),
            None => memory.places.push(Place { root, files: vec![note] }),
        }
    }
    out
}

fn note_file(file: &Located) -> NoteFile {
    NoteFile {
        path: file.path().to_path_buf(),
        name: file.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        bytes: disk::size_of(file),
        changed: disk::changed(file),
    }
}
