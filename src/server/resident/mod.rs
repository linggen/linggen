//! Yinyue's event-reactive watch loop.
//!
//! Taps the server event bus and, on a few coarse, report-worthy events, wakes
//! the Yinyue agent to decide whether to tell the user. First slice: react only
//! to a *non-Yinyue* mission finishing.
//!
//! The reaction is launched as a plain **agent run** (not a mission), so it (a)
//! never persists a `missions/yinyue-react/` dir that would pollute the mission
//! list, and (b) runs with Yinyue's full `yinyue.md` system prompt rather than a
//! mission body that replaces it.
//!
//! Guards:
//! 1. No self-loop — an agent run does not emit `MissionCompleted`, so a reaction
//!    can't re-trigger this loop. The `yinyue` mission-id check below is kept as
//!    belt-and-suspenders for any future mission-shaped reaction.
//! 2. Cost — match only the coarse event(s); the per-token firehose
//!    (`Token` / `TextSegment` / `ContentBlock*`) falls through the `else` arm at
//!    near-zero cost. The LLM is woken only on a narrow trigger.

use crate::util::LockExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;

use super::state::ServerState;
use crate::engine::events::{NotificationPayload, ServerEvent};

mod ambient;
mod guest;
mod session_roll;
mod spoken;
mod triggers;
mod turn;

pub use ambient::yinyue_ambient_loop;
pub(crate) use guest::{answer_as_guest, is_own_session};
use session_roll::*;
use spoken::*;
pub use triggers::yinyue_watch_loop;
pub(crate) use triggers::{wake_asked, wake_herald};
use turn::run_guest_turn;
pub(crate) use turn::run_yinyue_turn;

const YINYUE_AGENT: &str = crate::engine::agent::COMPANION_AGENT_ID;
/// Yinyue's sessions roll daily (`sess-yinyue-YYYY-MM-DD`) with an extra
/// segment (`…-2`, `…-3`) when a day's thread nears its context limit. One
/// session is active at a time, so turns still serialize through a single
/// engine lock and read as a continuing thread.
fn yinyue_session_prefix() -> String {
    format!("sess-{YINYUE_AGENT}")
}
/// Roll to a fresh segment once the live engine crosses this fraction of its
/// soft context limit — she starts clean and leans on memory rather than
/// compacting a long companion transcript.
const ROLL_TOKEN_FRACTION: f32 = 0.7;
/// Cap on how many recent messages stay in Yinyue's live prompt. Her one rolling
/// session would otherwise drag a whole day of turns into every quick reply; the
/// rest lives on disk + in recalled memory. Small = snappy.
const YINYUE_MAX_LIVE_MSGS: usize = 10;
/// The metered Linggen Cloud model — Yinyue's default brain for signed-in
/// (paid/free) users when `pet.model` is "auto". BYOK users keep the engine
/// default unless they pick a model in settings.
const CLOUD_DEFAULT_MODEL: &str = crate::provider::models::LINGGEN_CLOUD_MODEL_ID;
