//! The privacy fence: who may see which notes. Decided here and nowhere
//! else.
//!
//! What a person says about their body, their mind, the people close to them,
//! and Yinyue's dated life moments never go to a coding host (Claude Code,
//! Codex, a coding session): [`Audience::Work`] does not see them. Every way
//! out of the store — the index, a read by id, a read by path, a search hit,
//! and the lookup a write does by id — goes through this file, and what comes
//! out is a [`Seen`] or a [`SeenFile`], which only this file can make
//! (`tests::only_the_fence_loads_and_only_write_saves` fails when another
//! file loads a topic for a caller).

use super::format::{Area, Entry, Scope};
use super::store::{Loaded, Store};
use std::path::{Component, Path, PathBuf};

/// Who is asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Audience {
    /// Yinyue, outside a coding session: the person's whole life record.
    Companion,
    /// Everyone else: Ling, apps, Claude Code, Codex, any host on the door.
    Work,
}

/// The audience of a Linggen session: the companion's, unless a coding
/// session. The one place that says so.
pub fn audience_of(is_companion: bool, is_coding_session: bool) -> Audience {
    match is_companion && !is_coding_session {
        true => Audience::Companion,
        false => Audience::Work,
    }
}

impl Audience {
    /// Whether this audience may see a topic of `scope`. The one decision.
    pub fn sees(self, scope: &Scope) -> bool {
        match (self, scope) {
            (Audience::Companion, _) => true,
            (Audience::Work, Scope::Person | Scope::Project(_)) => true,
            (Audience::Work, Scope::Moments) => false,
            (Audience::Work, Scope::Area(area)) => !matches!(area, Area::Body | Area::Mind | Area::People),
        }
    }
}

/// One entry an audience may see, with where it lives.
#[derive(Clone, Debug)]
pub struct Seen {
    pub rel: String,
    pub version: String,
    pub scope: Scope,
    pub topic: String,
    pub entry: Entry,
    _fence: (),
}

/// One topic file an audience may see.
#[derive(Clone, Debug)]
pub struct SeenFile {
    pub rel: String,
    pub version: String,
    pub scope: Scope,
    pub topic: String,
    pub entries: Vec<Entry>,
    _fence: (),
}

/// Every entry `audience` may see (void ones too: callers leave them out of
/// what they show), and the files that could not be read.
pub(super) fn seen(store: &Store, audience: Audience) -> (Vec<Seen>, Vec<String>) {
    let (files, problems) = store.load_all_unfiltered();
    let mut out = Vec::new();
    for file in files.into_iter().filter(|f| audience.sees(&f.topic.scope)) {
        let Loaded { rel, version, topic } = file;
        for entry in topic.entries {
            out.push(Seen {
                rel: rel.clone(),
                version: version.clone(),
                scope: topic.scope.clone(),
                topic: topic.name.clone(),
                entry,
                _fence: (),
            });
        }
    }
    (out, problems)
}

/// The entry with `id`, if `audience` may see it.
pub(super) fn find(store: &Store, audience: Audience, id: &str) -> Option<Seen> {
    seen(store, audience).0.into_iter().find(|s| s.entry.id == id.trim())
}

#[derive(Debug, PartialEq, Eq)]
pub enum OpenError {
    /// Not a topic file inside the store (or one the audience may not see:
    /// the same words, so the answer tells nothing about what is there).
    NotANote,
    Unreadable(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::NotANote => f.write_str("no such note in Linggen's memory"),
            OpenError::Unreadable(why) => write!(f, "could not read the note: {why}"),
        }
    }
}

/// A topic file by its path: inside the store (relative, or absolute / `~/`
/// under it), a `.md` file, never `.state`.
pub(super) fn open(store: &Store, audience: Audience, path: &str) -> Result<SeenFile, OpenError> {
    let rel = rel_inside(store, path).ok_or(OpenError::NotANote)?;
    let file = match store.load(&rel) {
        Ok(Some(file)) => file,
        Ok(None) => return Err(OpenError::NotANote),
        Err(why) => return Err(OpenError::Unreadable(why)),
    };
    if !audience.sees(&file.topic.scope) {
        return Err(OpenError::NotANote);
    }
    Ok(SeenFile {
        rel: file.rel,
        version: file.version,
        scope: file.topic.scope,
        topic: file.topic.name,
        entries: file.topic.entries,
        _fence: (),
    })
}

/// `path` as a `/`-joined path inside the store, or `None`.
fn rel_inside(store: &Store, path: &str) -> Option<String> {
    let path = path.trim();
    let mut full = match path.strip_prefix("~/") {
        Some(rest) => store.env().home().join(rest),
        None => PathBuf::from(path),
    };
    if full.is_absolute() {
        full = full.strip_prefix(store.root()).ok()?.to_path_buf();
    }
    let parts: Vec<&str> = full
        .components()
        .map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let plain = !parts.is_empty() && !parts.iter().any(|p| p.starts_with('.'));
    let rel = parts.join("/");
    let note = Path::new(&rel).extension().is_some_and(|e| e == "md") && Store::is_topic_path(&rel);
    (plain && note).then_some(rel)
}
