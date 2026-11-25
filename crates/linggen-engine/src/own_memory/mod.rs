//! Linggen's own memory — what Ling and Yinyue keep about the person and the
//! work, as plain Markdown topic files under `$LINGGEN_HOME/memory/`.
//! "Memory V2": build spec in the memory-v2 design § 4a; this module is its
//! store, its one writer, its index and its reads (`doc/memory-spec.md`,
//! "Memory V2"). Other agents' memory is [`crate::outside_memory`]: read,
//! never written; this is the memory Linggen does write.
//!
//! One place decides each thing:
//! - The file form: [`format`]. Nothing else knows the text.
//! - Who may see what: [`fence`], by [`fence::Audience`]. Every way out of
//!   the store ends there (`tests::only_the_fence_loads_and_only_write_saves`).
//! - What is written and how: [`write::write`], the only saver.
//! - What a session is shown at start: [`index::index`]; what Yinyue is
//!   told of today's date: [`companion::today`]. What Settings shows of the
//!   notes: [`page`] (read only; its edits are [`write::write`]'s).
//! - The owner's phone under v2, its old verbs answered from these notes:
//!   [`phone`] (the translation only; the writer and the fence are the same).
//! - What the chronicle reads and what it asks the model to write:
//!   [`chronicle`] (it saves nothing itself; the writer does).
//! - Which memory a session has, v1 or v2: [`Mode`] (`[memory] linggen`).
//!   The tool's ops: [`call::call`].

pub mod call;
pub mod chronicle;
pub mod companion;
pub mod fence;
pub mod find;
pub mod format;
pub mod index;
pub mod page;
pub mod phone;
pub mod prompt;
pub mod spec;
mod store;
#[cfg(test)]
mod tests;
pub mod write;

pub use call::{call, Caller};
pub use fence::{agent_audience, audience_of, Audience};
pub use store::Store;

/// The tool's name, in a Linggen session and on the MCP door.
pub const TOOL: &str = "memory_notes";

/// Which memory Linggen's own sessions (Ling, Yinyue, apps) have — `[memory]
/// linggen = "v1" | "v2"`. The one place that decides; every site that
/// differs between the two matches on it
/// (`tests::the_switch_is_read_in_the_listed_places_only`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Today's: the ling-mem store, recalled every turn, the capture reminder.
    #[default]
    V1,
    /// These notes: an index at session start, the `memory_notes` tool, no
    /// per-turn recall or reminder.
    V2,
}

impl Mode {
    pub fn word(self) -> &'static str {
        match self {
            Mode::V1 => "v1",
            Mode::V2 => "v2",
        }
    }
}

/// `YYYY-MM-DD`, in the person's zone, of an instant in epoch seconds.
pub fn day_of(secs: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(secs, 0)
        .single()
        .map_or_else(|| "1970-01-01".to_string(), |t| t.format("%Y-%m-%d").to_string())
}
