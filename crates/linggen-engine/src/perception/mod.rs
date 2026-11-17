//! What a resident agent knows about the machine it lives in, without being
//! told. See `doc/perception-spec.md`.
//!
//! Three parts that cost three different amounts:
//!
//! - **State** ([`state`]) — what is true now. Rebuilt every turn from live
//!   readings, never stored.
//! - **The doorbell** ([`state`]) — two lines saying whether history is worth
//!   opening. The whole always-on cost of the log.
//! - **The log** ([`activity`]) — what changed and who did it. A file on this
//!   machine, read only through the `recent_activity` tool, never streamed into
//!   a turn: an agent whose context fills with its own event stream remembers
//!   the user's clicks and forgets the user.
//!
//! [`world`] puts them together into the one block every person's session
//! is given. [`apps`] is one line per app, declared by its skill and read from its own
//! file — the engine names no app.
//!
//! [`conditions`] is what lets a resident start a sentence, and [`trend`] is
//! what lets it say "in two days" rather than "nearly full". [`told`] is the
//! record of what the person has already been told, so a problem is said once
//! and again only when it is worse or has come back.

pub mod activity;
pub mod apps;
pub mod conditions;
pub mod devices;
pub mod publish;
pub mod query;
pub mod rotation;
pub mod state;
pub mod stretch;
pub mod told;
pub mod trend;
pub mod when;
pub mod world;

/// This machine, as the log names it.
pub fn host_name() -> String {
    crate::account::instance_name()
}
