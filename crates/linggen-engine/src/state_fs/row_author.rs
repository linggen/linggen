//! Who wrote a session row — the one place a row's sender (`ChatMsg::from_id`)
//! is read.
//!
//! A row's sender is a person, the engine, one of the engine's own kinds of
//! note (a compaction summary, a kept world block, a memory recall), or an
//! agent by its id. The first six are a closed set: [`RowAuthor`] names
//! them, and every reader asks [`RowAuthor::of`] / [`ChatMsg::author`]
//! instead of comparing the stored text. The stored and wire form stays the
//! plain string it has always been (`"user"`, `"system"`, …): `from_id` is
//! kept as text on disk and sent as text to the UI and the phone, and
//! [`RowAuthor::id`] gives that text back unchanged.
//!
//! An agent's id is an open set and is not decided here.
//! Lock: `src/structure_lock_tests.rs` fails when production code compares a
//! sender with one of these names anywhere else.

use super::sessions::ChatMsg;

/// The stored sender of a person's message.
pub const PERSON_SENDER: &str = "user";
/// The stored sender of the engine's own rows: tool results, notices.
pub const SYSTEM_SENDER: &str = "system";
/// The pseudo-sender a compaction summary is kept under: context for every
/// member, never a line of dialogue, and no place's row
/// ([`ChatMsg::place_of`]).
pub const COMPACTION_SENDER: &str = "compaction";
/// The pseudo-sender of a kept world block's row (`engine::prompt::world::keep`).
pub const WORLD_SENDER: &str = "world";
/// The pseudo-sender of a memory recall's row. Not `"memory"`: that is the
/// memory agent's id.
pub const MEMORY_RECALL_SENDER: &str = "memory-recall";

/// The pseudo-sender of a session-start memory row: the core rows and index
/// the system prompt was given, shown to the person, never read back to a model.
pub const MEMORY_START_SENDER: &str = "memory-start";

/// Who wrote a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowAuthor<'a> {
    /// A person's message (`"user"`), whichever device sent it.
    Person,
    /// The engine itself (`"system"`): a tool's result, a notice.
    System,
    /// A compaction summary (`"compaction"`).
    Compaction,
    /// A kept world block (`"world"`).
    World,
    /// A memory recall shown with the turn (`"memory-recall"`).
    MemoryRecall,
    /// What memory the session started with (`"memory-start"`).
    MemoryStart,
    /// An agent, or any other sender, by its id.
    Agent(&'a str),
}

impl<'a> RowAuthor<'a> {
    /// The author a stored sender names.
    pub fn of(from_id: &'a str) -> Self {
        match from_id {
            PERSON_SENDER => Self::Person,
            SYSTEM_SENDER => Self::System,
            COMPACTION_SENDER => Self::Compaction,
            WORLD_SENDER => Self::World,
            MEMORY_RECALL_SENDER => Self::MemoryRecall,
            MEMORY_START_SENDER => Self::MemoryStart,
            id => Self::Agent(id),
        }
    }

    /// The stored and wire form, unchanged from what [`RowAuthor::of`] read.
    pub fn id(self) -> &'a str {
        match self {
            Self::Person => PERSON_SENDER,
            Self::System => SYSTEM_SENDER,
            Self::Compaction => COMPACTION_SENDER,
            Self::World => WORLD_SENDER,
            Self::MemoryRecall => MEMORY_RECALL_SENDER,
            Self::MemoryStart => MEMORY_START_SENDER,
            Self::Agent(id) => id,
        }
    }

    /// A person's message.
    pub fn is_person(self) -> bool {
        self == Self::Person
    }

    /// The engine's own row (`"system"`).
    pub fn is_system(self) -> bool {
        self == Self::System
    }

    /// An agent's (or another named sender's) row: none of the engine's kinds.
    pub fn agent(self) -> Option<&'a str> {
        match self {
            Self::Agent(id) => Some(id),
            _ => None,
        }
    }
}

impl ChatMsg {
    /// Who wrote this row.
    pub fn author(&self) -> RowAuthor<'_> {
        RowAuthor::of(&self.from_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows written before the enum read back as the same author, and write
    /// back byte for byte: the stored and wire form did not change.
    #[test]
    fn row_author_keeps_stored_and_wire_form_byte_for_byte() {
        let cases = [
            ("user", RowAuthor::Person),
            ("system", RowAuthor::System),
            ("compaction", RowAuthor::Compaction),
            ("world", RowAuthor::World),
            ("memory-recall", RowAuthor::MemoryRecall),
            ("memory-start", RowAuthor::MemoryStart),
            ("ling", RowAuthor::Agent("ling")),
            ("yinyue", RowAuthor::Agent("yinyue")),
            ("memory", RowAuthor::Agent("memory")),
            ("claude-code", RowAuthor::Agent("claude-code")),
        ];
        for (from, author) in cases {
            let old = format!(
                r#"{{"agent_id":"ling","from_id":"{from}","to_id":"ling","content":"hi","timestamp":1,"is_observation":false}}"#
            );
            let row: ChatMsg = serde_json::from_str(&old).unwrap();
            assert_eq!(row.author(), author, "{from}");
            assert_eq!(author.id(), from);
            assert_eq!(serde_json::to_string(&row).unwrap(), old, "{from}");
        }
    }
}
