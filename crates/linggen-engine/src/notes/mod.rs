//! An agent's own notes: a few short lines it wrote for itself, in front of
//! it at every turn until it finishes them, each with an optional time to look
//! again (`doc/yinyue-companion-spec.md` § Notes).
//!
//! The agent decides what is worth a note, what it says and when to look
//! again; this module only keeps them. A note belongs to the agent that wrote
//! it: one small file per agent under `~/.linggen/notes/`, read and written
//! under one lock, so the agent's tools and the engine's look-again wake never
//! interleave a change.

mod book;
pub mod shown;

pub use book::{Note, NoteBook};

use std::sync::Mutex;

/// The most characters one note holds — a few short lines.
pub const TEXT_MAX: usize = 200;
/// The most open notes an agent may hold. Writing one more is refused with
/// the open ones listed, so the agent chooses which to finish.
pub const OPEN_MAX: usize = 8;
/// The most characters of notes shown in one turn.
pub const SHOWN_MAX_CHARS: usize = 2400;

/// The note tools. An agent has notes when its own spec lists them; they
/// are never part of an unrestricted (`*`) tool set.
pub const TOOLS: [&str; 2] = [WRITE_TOOL, DONE_TOOL];
pub const WRITE_TOOL: &str = "note_write";
pub const DONE_TOOL: &str = "note_done";

/// Whether `name` is one of the note tools.
pub fn is_note_tool(name: &str) -> bool {
    TOOLS.contains(&name)
}

/// One change at a time, from any caller.
static LOCK: Mutex<()> = Mutex::new(());

/// `agent`'s notes as they are on disk now.
pub fn read(agent: &str) -> NoteBook {
    let _held = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    NoteBook::open(&crate::paths::notes_dir(), agent)
}

/// Read `agent`'s notes, change them with `change`, and save them when it
/// says something changed (`Ok`). The answer is `change`'s.
pub fn change<T>(
    agent: &str,
    change: impl FnOnce(&mut NoteBook) -> Result<T, String>,
) -> Result<T, String> {
    let _held = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut book = NoteBook::open(&crate::paths::notes_dir(), agent);
    let out = change(&mut book)?;
    book.save()
        .map_err(|e| format!("the note could not be saved: {e}"))?;
    Ok(out)
}
