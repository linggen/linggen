//! Look again — when a note's look-again time comes, she gets a turn of her
//! own with that note as the reason: a quiet turn like a glance
//! ([`Reach::SpeakOnly`]), so she may speak, ask, keep her notes, or end
//! SILENT, and never act.
//!
//! Code only keeps the time. Whether the note is worth a sentence is hers.
//!
//! - A time fires once: it is cleared, and saved, before her turn runs; the
//!   note stays open without a time until she finishes it or sets another.
//! - Times that passed while the engine was down fire after start-up
//!   ([`SETTLE_SECS`]), oldest first, at least [`SPACING_SECS`] apart.
//! - The wake waits while the user is typing (the moments gate's own
//!   reading) or a turn of hers is running; it waits, without being used up,
//!   while she is off or not here yet.
//! - In quiet hours, or with the hour's ceiling reached, the wake is held
//!   until they pass (`guard`), recorded once.

use super::guard::{self, Held, Withheld};
use super::triggers::wake_and_speak;
use super::*;
use crate::notes::Note;

/// How often due notes are looked for. A file read, no model.
const TICK_SECS: u64 = 5;
/// After start-up, before the first look: the engine settles first.
pub(super) const SETTLE_SECS: u64 = 20;
/// At least this long between two woken notes, so missed ones never arrive
/// as a burst.
pub(super) const SPACING_SECS: u64 = 120;

pub async fn yinyue_look_again_loop(state: Arc<ServerState>) {
    tracing::info!("[yinyue-look-again] started");
    tokio::time::sleep(Duration::from_secs(SETTLE_SECS)).await;
    let mut last_wake = None;
    loop {
        if let Some(at) = look_once(&state, last_wake).await {
            last_wake = Some(at);
        }
        tokio::time::sleep(Duration::from_secs(TICK_SECS)).await;
    }
}

/// What the room says about waking her now.
#[derive(Clone, Debug, Default)]
pub(super) struct Room {
    /// The pet is on and she is here (met).
    pub ready: bool,
    pub typing: bool,
    /// A turn of hers is running, anywhere.
    pub her_turn: bool,
    /// A guardrail that holds an unprompted line now.
    pub held: Option<Held>,
}

/// What to do with a due note.
#[derive(Debug, PartialEq)]
pub(super) enum Step {
    /// Nothing is due.
    Rest,
    /// Not yet: spacing, typing, her turn, or she is not there.
    Wait,
    /// A guardrail holds it.
    Hold(Held),
    Wake,
}

/// Pure, so the rules are tested, not trusted.
pub(super) fn step(now: u64, due: bool, last_wake: Option<u64>, room: &Room) -> Step {
    if !due {
        return Step::Rest;
    }
    if last_wake.is_some_and(|t| now.saturating_sub(t) < SPACING_SECS) {
        return Step::Wait;
    }
    if !room.ready || room.typing || room.her_turn {
        return Step::Wait;
    }
    match &room.held {
        Some(held) => Step::Hold(held.clone()),
        None => Step::Wake,
    }
}

/// One look: wake her for the oldest due note when the room allows.
/// Returns when she was woken.
async fn look_once(state: &Arc<ServerState>, last_wake: Option<u64>) -> Option<u64> {
    let now = crate::util::now_ts_secs();
    let note = crate::notes::read(YINYUE_AGENT).next_due(now).cloned()?;
    let at = note.look_again_at?;
    let room = room_now(state, now).await;
    match step(now, true, last_wake, &room) {
        Step::Rest | Step::Wait => None,
        Step::Hold(held) => {
            guard::record_held(&withheld(&note, at), &held);
            None
        }
        Step::Wake => wake(state, note, at, now).await,
    }
}

async fn wake(state: &Arc<ServerState>, note: Note, at: u64, now: u64) -> Option<u64> {
    // Cleared and saved first: a time never fires twice, even if this
    // engine stops mid-turn.
    let fired = crate::notes::change(YINYUE_AGENT, |book| {
        book.fire(&note.id, at)
            .then_some(())
            .ok_or_else(|| "changed meanwhile".to_string())
    });
    if fired.is_err() {
        return None;
    }
    tracing::info!("[yinyue-look-again] waking her for {}", note.id);
    wake_and_speak(
        state.clone(),
        (kickoff(&note, now), None),
        "neutral",
        LOOK_AGAIN_SOURCE,
        Reach::SpeakOnly,
        None,
    )
    .await;
    Some(now)
}

/// The trigger source of a woken note's turn.
pub(super) const LOOK_AGAIN_SOURCE: &str = "note";

fn withheld(note: &Note, at: u64) -> Withheld<'static> {
    Withheld {
        what: format!("a look-again on {}", note.id),
        source: "note",
        key: format!("note-{}-{at}", note.id),
    }
}

async fn room_now(state: &Arc<ServerState>, now: u64) -> Room {
    let pet = state.manager.get_config_snapshot().await.pet;
    Room {
        ready: pet.enabled && here(state).await,
        typing: crate::server::yinyue_moments::user_typing(state, now),
        her_turn: her_turn_running(state),
        held: guard::held_now(&pet),
    }
}

/// A turn of hers in any session, or an app moment's wake under way.
fn her_turn_running(state: &Arc<ServerState>) -> bool {
    crate::server::yinyue_moments::wake_in_flight()
        || state.manager.run_store.list_runs(None).iter().any(|r| {
            r.agent_id == YINYUE_AGENT
                && matches!(r.status, crate::engine::agent::AgentRunStatus::Running)
        })
}

/// The kickoff she is woken with: the note, marked as the reason, and what
/// the turn may do — never whether the note deserves a word.
pub(super) fn kickoff(note: &Note, now: u64) -> String {
    let age = crate::perception::activity::ago(now.saturating_sub(note.written_at) as i64);
    format!(
        "You set a time to look again at your note {} (written {age}): \"{}\". That time has \
         come; it is why you are here. Your notes, Right now and the World are in front of you. \
         You may say one short line in your own voice, spoken aloud, plain prose, or ask \
         something; you may write, rewrite or finish notes; or say nothing. The note stays open \
         until you finish it. You cannot act this turn; if something needs doing, say so in your \
         line and ask.",
        note.id,
        note.text.replace('\n', " / "),
    )
}

#[cfg(test)]
mod tests;
