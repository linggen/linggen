//! The entrance of a session turn: the engine-side steps every door that
//! sets a session turn up takes, in one order, in one function
//! ([`AgentEngine::open_session_turn`]). A person's chat, Yinyue's turn, a
//! task's end and a delivered `agent_chat` pass it through
//! `run_session_turn`; an approved plan's run, `agent_run`'s new run and a
//! mission call it themselves. [`AgentEngine::begin_turn`] is called nowhere
//! else but here and by the light reply (lock:
//! `server/chat/entrance_lock.rs`).
//!
//! The order: the session on the tools, the turn's settings, the machine's
//! live state, the model, the seat, the session's withheld tools (always).
//! What each door adds stays at the door: standing for reads, her recall and
//! her model, the place and its mission, a mission's grants and kickoff, a
//! delegate's toolset and bridges, the loop and the end of the turn.
//!
//! A turn that continues a person's session is set up as that session's
//! turns are. A door skips a step only when the step cannot apply to it, and
//! the variant says why. `grep AsFound` lists what is left undecided.
//!
//! Not in the entrance, and why: a light reply (no tools, no loop; it begins
//! a turn for its language alone), a delegated child (a fresh engine its
//! parent builds, never a session's), the prompt export (no loop). The
//! place is bound by the door's body (`bind_turn_place`), which needs the
//! skills, the policy and the page events.

use crate::engine::agent::AgentManager;
use crate::engine::model_choice::{resolve, AskedBy, Rule, DELEGATED_RUN_MAY_SWITCH};
use crate::engine::turn_place::TurnSettings;
use crate::engine::AgentEngine;

/// Whether the session is put on the engine's tools (`set_session_id`).
/// What reads it: a skill tool's `LINGGEN_SESSION_ID`; a background shell or
/// Task (`run_in_background` needs a session); `SendMessage`, `TaskList`;
/// the loop-break of `agent_chat`; a memory write's `source_session` stamp;
/// the session's own place and folder for a call no turn made; a Task child
/// joining the session's live permissions; the perception world block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOnTools {
    Bound,
    /// Not bound: `agent_run`'s run and a mission have no person's chat to
    /// continue — each runs in a session made for it, and every reader above
    /// then sees no session (decided 2026-11: leave; see the R4 report's
    /// findings before binding).
    NotBound,
}

/// Whether the turn starts on the machine as it is now (`read_live_state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveState {
    Read,
    /// Read by the door before the entrance, because the door chooses the
    /// model right after reading it and the model choice needs the live
    /// state first: a person's chat and her turn. Meant.
    ReadAtTheDoor,
}

/// Which model the turn's door asks for here, and on what rule.
#[derive(Debug)]
pub enum ModelStep<'a> {
    /// The entrance chooses by the one rule (`model_choice::resolve`) against
    /// the agent's own default, and runs the turn on it.
    Chosen {
        /// The model the door asks for; `None` asks for none.
        asked: Option<&'a str>,
        /// Who asks; `None` is whoever the engine's policy says
        /// (`AgentEngine::asked_by`).
        asked_by: Option<AskedBy>,
        may_switch: MaySwitch,
        /// Names the turn in the log and in a fallback's notice.
        place: String,
    },
    /// The door chooses it by `resolve_effective_model` (the member's own
    /// model, with its kept-model rules): a person's chat and her turn before
    /// the entrance, a plan's run right after it.
    ChosenByTheDoor,
    /// Not chosen: a task's end and a delivered message run on the model the
    /// engine was left on (kept as found, 2026-11: see R4 stage 2 — not
    /// settled; after a restart a member's kept model is not the engine's).
    NotChosenAsFound,
}

/// Whether an asked model that cannot run may be replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaySwitch {
    /// A delegated run never switches (`DELEGATED_RUN_MAY_SWITCH`).
    DelegatedRun,
    /// The session's own switch alone (`switching(session).session_allows()`).
    SessionAllows,
}

/// Where the member sits this turn (`seat_at`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seat {
    /// At a lead's table with the lead's tools, or the lead itself (`None`).
    At(Option<(String, Option<Vec<String>>)>),
    /// The turn has no session, so no table to sit at.
    NoSession,
    /// A mission runs alone in the session made for it: it has no table.
    AMissionHasNone,
    /// Seated by the door: `agent_run`'s run takes the delegate's seat with
    /// its toolset (`seat_delegate`).
    SeatedByTheDoor,
}

/// What a door hands the entrance.
pub struct SessionTurn<'a> {
    pub manager: &'a AgentManager,
    pub session: Option<&'a str>,
    pub settings: TurnSettings,
    pub session_on_tools: SessionOnTools,
    pub live_state: LiveState,
    pub model: ModelStep<'a>,
    pub seat: Seat,
}

impl AgentEngine {
    /// Set a session turn up: the one function every session turn passes
    /// through, in its one order (module comment). The door runs the turn
    /// and ends it with [`AgentEngine::end_turn`].
    pub async fn open_session_turn(&mut self, turn: SessionTurn<'_>) {
        let SessionTurn {
            manager,
            session,
            settings,
            session_on_tools,
            live_state,
            model,
            seat,
        } = turn;
        if session_on_tools == SessionOnTools::Bound {
            // Delegated tasks must inherit the session, whoever drives the turn.
            self.tools.builtins.set_session_id(session.map(str::to_string));
        }
        self.begin_turn(settings);
        if live_state == LiveState::Read {
            self.read_live_state(manager).await;
        }
        if let ModelStep::Chosen { asked, asked_by, may_switch, place } = model {
            self.take_model(session, (asked, asked_by, may_switch), &place).await;
        }
        if let Seat::At(lead) = seat {
            self.seat_at(lead);
        }
        // The session's own withheld tools, whoever the door is: a plan's
        // run on a rebuilt engine holds what the record withholds.
        self.withhold_session_tools(&manager.global_sessions, session);
    }

    /// The turn's model by the one rule, against the agent's own default.
    async fn take_model(
        &mut self,
        session: Option<&str>,
        (asked, asked_by, may_switch): (Option<&str>, Option<AskedBy>, MaySwitch),
        place: &str,
    ) {
        let may_switch = match may_switch {
            MaySwitch::DelegatedRun => DELEGATED_RUN_MAY_SWITCH,
            MaySwitch::SessionAllows => self.switching(session).session_allows(),
        };
        let asked_by = asked_by.unwrap_or_else(|| self.asked_by());
        let rule = Rule { asker: asked_by.asker(), may_switch };
        let choice = resolve(&self.model_manager, asked, &self.default_model_id, rule);
        self.run_on(&choice, place).await;
    }
}
