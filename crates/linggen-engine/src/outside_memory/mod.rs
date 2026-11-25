//! What other agents remember, read and forwarded — never written.
//!
//! Claude Code, Codex and the rest keep their own memory their own way, and
//! their teams keep improving it; Linggen does not touch it. It reads each
//! agent's files where they are, and hands the lines of an agent's index to
//! the OTHER agents at session start (`slice`), a note's body when asked
//! (`read`), and words found across all of it (`search`). Design:
//! memory-v2 design § 0, 3.1, 3.2.
//!
//! One place decides each thing:
//! - Which agents' memories exist, where, and how a file becomes lines: a
//!   declared reader ([`decl`]), one file per agent — the embedded ones and
//!   those installed skills carry ([`readers`]). A new agent is a file; no
//!   Rust names a path of theirs (`tests::no_rust_file_names_another_agents_folder`).
//! - What may be read: only files a declared place names, and only through
//!   the registry ([`readers::Registry::locate`]); only [`disk`] touches the
//!   disk (`tests::only_disk_reads_the_disk`).
//! - What leaves: only [`Clean`] text, made only by [`redact::clean`].
//! - That nothing here writes: no module but `tests` may name a write call
//!   (`tests::this_module_never_writes`).
//!
//! A place that is missing or cannot be read is skipped and reported
//! ([`SourceNote`]), never fatal.

mod access;
mod decl;
mod disk;
mod lines;
mod paths;
mod readers;
mod redact;
mod slice;
#[cfg(test)]
mod tests;

pub use access::{listing, read, search, touched, AgentMemory, Hit, Note, NoteFile, Place, ReadError, Touched};
pub use lines::{reports, SourceReport};
pub use readers::Agent;
pub use redact::Clean;
// The chronicle cleans what it quotes of the person's sessions the same way.
pub(crate) use redact::clean;
pub use slice::{slice, Slice, SourceNote, BUDGET_BYTES};
// Linggen's own memory (`own_memory`) files its project notes the same way
// Claude Code does and walks a session's folder up the same chain.
pub(crate) use paths::{encode_project_path, project_chain};

/// The agent a calling host's name stands for (`agent_run`'s `host` words),
/// by the readers' declared `hosts`; `None` for a host with no memory
/// Linggen reads (Cursor, …), which is forwarded everyone's.
pub fn host_agent(env: &Env, host: &str) -> Option<Agent> {
    readers::Registry::load(env).agent_of_host(host)
}

use std::path::{Path, PathBuf};

/// Where the agents' folders are looked for: a home folder. Tests pass a
/// scratch one; the engine passes the machine's.
#[derive(Clone, Debug)]
pub struct Env {
    home: PathBuf,
}

impl Env {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }

    /// This machine's home, or `None` when it has none.
    pub fn this_machine() -> Option<Self> {
        dirs::home_dir().map(Self::new)
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// `path` with the home folder written as `~`, for text a person or an
    /// agent reads.
    pub fn tilde(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        }
    }
}

/// Where a session stands: the folder it started in.
#[derive(Clone, Debug, Default)]
pub struct Scope {
    pub cwd: Option<PathBuf>,
}

/// `text` cut to `max` characters, with `…` after a cut (never in the middle
/// of a character).
pub(crate) fn shorten(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}
