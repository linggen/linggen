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
//! live state, the model, the seat, the session's withheld tools. What each
//! door adds stays at the door: standing for reads, her recall and her
//! model, the place and its mission, a mission's grants and kickoff, a
//! delegate's toolset and bridges, the loop and the end of the turn.
//!
//! STAGE 1 (R4, 2026-11): each door's behaviour is kept exactly as it was,
//! including the steps it does not take. A step a door does not take is a
//! named variant ending in `AsFound`; stage 2 decides each (a fault — fixed
//! and named in the changelog — or meant, with a comment at the door).
//! `grep AsFound` lists them all.
//!
//! Not in the entrance, and why: a light reply (no tools, no loop; it begins
//! a turn for its language alone), a delegated child (a fresh engine its
//! parent builds, never a session's), the prompt export (no loop). The
//! place is bound by the door's body (`bind_turn_place`), which needs the
//! skills, the policy and the page events.

use crate::engine::agent::AgentManager;
use crate::engine::model_choice::{resolve, Asker, Rule, DELEGATED_RUN_MAY_SWITCH};
use crate::engine::turn_place::TurnSettings;
use crate::engine::AgentEngine;

/// Whether the session is put on the engine's tools (`set_session_id`): what
/// a delegated task and a memory write are stamped with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOnTools {
    Bound,
    /// Not bound: an approved plan's run, `agent_run`'s run and a mission
    /// (kept as found, 2026-11: see R4 stage 2).
    UnboundAsFound,
}

/// Whether the turn starts on the machine as it is now (`read_live_state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveState {
    Read,
    /// Not read by the entrance (kept as found, 2026-11: see R4 stage 2):
    /// a person's chat and her turn read it at their own doors before the
    /// turn; a task's end, a delivered message and an approved plan's run
    /// do not read it before the loop at all (the loop reads a changed skill
    /// list and settings when it starts; what a door does before the loop
    /// sees the last reading).
    NotReadAsFound,
}

/// Which model the turn's door asks for here, and on what rule.
#[derive(Debug, Clone)]
pub enum ModelStep<'a> {
    Chosen {
        /// The model the door asks for; `None` asks for none.
        asked: Option<&'a str>,
        standing: StandingModel,
        may_switch: MaySwitch,
        /// Names the turn in the log and in a fallback's notice.
        place: String,
    },
    /// Chosen at the door, before the turn (a person's chat, her turn), or
    /// not chosen: the engine runs on the model it was left on (a task's
    /// end, a delivered message, an approved plan's run) — kept as found,
    /// 2026-11: see R4 stage 2.
    AtTheDoorAsFound,
}

/// The model an asked one gives way to when it cannot run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandingModel {
    /// The agent's default (`default_model_id`): a mission.
    Default,
    /// The model the engine runs on now (`model_id`): `agent_run`
    /// (kept as found, 2026-11: see R4 stage 2). Its rule never switches,
    /// so the standing model is not what runs there either way
    /// (`a_delegated_runs_rule_never_answers_with_the_standing_model`).
    CurrentAsFound,
}

/// Whether an asked model that cannot run may be replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaySwitch {
    /// A delegated run never switches (`DELEGATED_RUN_MAY_SWITCH`).
    DelegatedRun,
    /// What the session's switch and the config allow, read after the live
    /// state (`switching`).
    SessionAllows,
}

/// Where the member sits this turn (`seat_at`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seat {
    /// At a lead's table with the lead's tools, or the lead itself (`None`).
    At(Option<(String, Option<Vec<String>>)>),
    /// Not set by the entrance: a turn with no session, and the doors that
    /// seat themselves or not at all (an approved plan's run, `agent_run`'s
    /// run — `seat_delegate` — and a mission): kept as found, 2026-11: see
    /// R4 stage 2.
    LeftAsFound,
}

/// Whether the session's own withheld tools are applied
/// (`withhold_session_tools`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionsWithheld {
    Applied,
    /// Not applied: an approved plan's run and a mission (kept as found,
    /// 2026-11: see R4 stage 2). The engine keeps what an earlier turn read
    /// from the record (`session_withheld`), which a rebuilt engine lacks.
    SkippedAsFound,
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
    pub withheld: SessionsWithheld,
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
            withheld,
        } = turn;
        if session_on_tools == SessionOnTools::Bound {
            // Delegated tasks must inherit the session, whoever drives the turn.
            self.tools.builtins.set_session_id(session.map(str::to_string));
        }
        self.begin_turn(settings);
        if live_state == LiveState::Read {
            self.read_live_state(manager).await;
        }
        if let ModelStep::Chosen { asked, standing, may_switch, place } = model {
            self.take_model(session, (asked, standing, may_switch), &place).await;
        }
        if let Seat::At(lead) = seat {
            self.seat_at(lead);
        }
        if withheld == SessionsWithheld::Applied {
            self.withhold_session_tools(&manager.global_sessions, session);
        }
    }

    /// The turn's model by the one rule, as the owner asks for it.
    async fn take_model(
        &mut self,
        session: Option<&str>,
        (asked, standing, may_switch): (Option<&str>, StandingModel, MaySwitch),
        place: &str,
    ) {
        let may_switch = match may_switch {
            MaySwitch::DelegatedRun => DELEGATED_RUN_MAY_SWITCH,
            MaySwitch::SessionAllows => self.switching(session).session_allows(),
        };
        let standing = match standing {
            StandingModel::Default => self.default_model_id.clone(),
            StandingModel::CurrentAsFound => self.model_id.clone(),
        };
        let rule = Rule { asker: Asker::Owner, may_switch };
        let choice = resolve(&self.model_manager, asked, &standing, rule);
        self.run_on(&choice, place).await;
    }
}
