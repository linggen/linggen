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
//! - Which agents' memories exist and where: [`sources::SOURCES`], a declared
//!   list. A new agent is one entry; nothing else names a path of theirs
//!   (`tests::only_the_source_list_names_other_agents_folders`).
//! - What may be read: only files inside a source's roots ([`access`]).
//! - What leaves: only [`Clean`] text, made only by [`redact::clean`].
//! - That nothing here writes: no module but `tests` may name a write call
//!   (`tests::this_module_never_writes`).
//!
//! A source that is missing or cannot be read is skipped and reported
//! ([`SourceNote`]), never fatal.

mod access;
mod redact;
mod slice;
mod sources;
#[cfg(test)]
mod tests;

pub use access::{listing, read, search, AgentMemory, Hit, Note, NoteFile, Place, ReadError};
pub use redact::Clean;
pub use slice::{slice, Slice, SourceNote, BUDGET_BYTES};
pub use sources::{reports, SourceReport};
// Linggen's own memory (`own_memory`) files its project notes the same way
// Claude Code does and walks a session's folder up the same chain.
pub(crate) use sources::{encode_project_path, project_chain};

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

/// An agent whose memory Linggen can read — and a calling host that has
/// one. Closed: the compiler finds the place a new one is missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    ClaudeCode,
    Codex,
}

impl Agent {
    pub fn label(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "Claude Code",
            Agent::Codex => "Codex",
        }
    }

    /// The agent's name in an address and in the page's tabs.
    pub fn id(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "claude-code",
            Agent::Codex => "codex",
        }
    }

    /// The agent a calling host's name stands for (`agent_run`'s `host`
    /// words); `None` for a host with no memory Linggen reads (Cursor, …),
    /// which is forwarded everyone's.
    pub fn of_host(host: &str) -> Option<Agent> {
        match host.trim().to_ascii_lowercase().as_str() {
            "claude-code" | "claude" | "cc" => Some(Agent::ClaudeCode),
            "codex" => Some(Agent::Codex),
            _ => None,
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
