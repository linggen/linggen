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
//! [`conditions`] is what lets a resident start a sentence, and [`trend`] is
//! what lets it say "in two days" rather than "nearly full".

pub mod activity;
pub mod conditions;
pub mod devices;
pub mod publish;
pub mod rotation;
pub mod state;
pub mod trend;

/// This machine, as the log names it.
pub fn host_name() -> String {
    crate::account::instance_name()
}
