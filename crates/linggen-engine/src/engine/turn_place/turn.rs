//! A turn's settings: what one turn sets on the engine for itself — the
//! tools it withholds, the language it answers in, whether it may offer a
//! next-prompt hint, a mission's kickoff queue — set by
//! [`AgentEngine::begin_turn`] when the turn starts and put back by
//! [`AgentEngine::end_turn`] when it ends. Beginning a turn puts back
//! whatever an earlier turn left first, so a turn that ended by error, by
//! cancel or not at all leaves nothing to the next one either.
//!
//! Inside a turn, each run of the loop (a reply, a plan, a plan's execution)
//! is set up by [`AgentEngine::begin_run`]: its task, its images, how it
//! streams. The loop puts the run's wiring back when it returns, on every
//! path (`AgentEngine::run_agent_loop`).
//!
//! What a turn leaves for its caller to read — its run id, its last words,
//! what it spent — is cleared when the next turn begins, never when this
//! one ends.

use crate::engine::{AgentEngine, StreamedToken};
use std::collections::{BTreeMap, HashSet};
use tokio::sync::mpsc::UnboundedSender;

/// What a turn is started with. The default is an ordinary turn: nothing
/// withheld, the Linggen language, no hint, no kickoff.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnSettings {
    /// Tools this turn's caller withholds, on top of the session's own.
    pub withheld: HashSet<String>,
    /// The only tools this turn may use, when its caller restricts it.
    pub allowlist: Option<HashSet<String>>,
    /// The language a turn the person did not type answers in, when its
    /// caller knows it; `None` is the Linggen language.
    pub language: Option<String>,
    /// The kickoff hands the agent a mission fault in the engine's words.
    pub fault: bool,
    /// The turn's surface shows the next-prompt hint.
    pub followups: bool,
    /// A mission run's kickoff items after the first.
    pub kickoff: Kickoff,
}

/// A mission's kickoff (`Mission.kickoff[1..]` and its rules), drained one
/// item per final reply by the loop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Kickoff {
    pub queue: Vec<String>,
    pub stop: Vec<String>,
    pub fresh: bool,
    pub then: BTreeMap<String, Vec<String>>,
}

/// What one run of the loop inside a turn starts with.
#[derive(Debug, Default)]
pub struct RunSetup {
    task: String,
    images: Option<Vec<String>>,
    silence_ok: bool,
    plan_mode: bool,
    thinking: Option<UnboundedSender<StreamedToken>>,
}

impl RunSetup {
    /// A run that works on `task`.
    pub fn task(task: impl Into<String>) -> Self {
        Self {
            task: task.into(),
            ..Self::default()
        }
    }

    /// The images the run's first message carries (replacing any pending).
    pub fn images(mut self, images: Vec<String>) -> Self {
        self.images = Some(images);
        self
    }

    /// A reply that is exactly the silence marker is neither shown nor kept.
    pub fn silence_ok(mut self, ok: bool) -> Self {
        self.silence_ok = ok;
        self
    }

    /// The run researches and plans, read-only.
    pub fn planning(mut self) -> Self {
        self.plan_mode = true;
        self
    }

    /// Where the run's tokens stream.
    pub fn thinking(mut self, tx: Option<UnboundedSender<StreamedToken>>) -> Self {
        self.thinking = tx;
        self
    }
}

impl AgentEngine {
    /// Start a turn with `turn`. Whatever an earlier turn set is put back
    /// first, and what it left for its caller (its run, its last words, what
    /// it spent) is cleared.
    pub fn begin_turn(&mut self, turn: TurnSettings) {
        self.put_back_turn_settings();
        self.last_run_id = None;
        self.last_assistant_text = None;
        self.run_usage = Default::default();
        self.run_faults = Default::default();
        let TurnSettings {
            withheld,
            allowlist,
            language,
            fault,
            followups,
            kickoff,
        } = turn;
        self.withheld_tools = withheld;
        self.turn_tool_allowlist = allowlist;
        self.turn_language = language;
        self.turn_fault = fault;
        self.suggest_followups = followups;
        self.kickoff_queue = kickoff.queue.into();
        self.kickoff_stop = kickoff.stop;
        self.kickoff_fresh = kickoff.fresh;
        self.kickoff_then = kickoff.then;
    }

    /// The turn is over: its settings and its place are put back. What it
    /// left for its caller stays until the next turn begins.
    pub fn end_turn(&mut self) {
        self.put_back_turn_settings();
        self.leave_place();
    }

    /// The settings of the running turn, as it was started.
    pub fn turn_settings(&self) -> TurnSettings {
        TurnSettings {
            withheld: self.withheld_tools.clone(),
            allowlist: self.turn_tool_allowlist.clone(),
            language: self.turn_language.clone(),
            fault: self.turn_fault,
            followups: self.suggest_followups,
            kickoff: Kickoff {
                queue: self.kickoff_queue.iter().cloned().collect(),
                stop: self.kickoff_stop.clone(),
                fresh: self.kickoff_fresh,
                then: self.kickoff_then.clone(),
            },
        }
    }

    fn put_back_turn_settings(&mut self) {
        self.withheld_tools.clear();
        self.turn_tool_allowlist = None;
        self.turn_language = None;
        self.turn_fault = false;
        self.suggest_followups = false;
        self.kickoff_queue.clear();
        self.kickoff_stop.clear();
        self.kickoff_fresh = false;
        self.kickoff_then.clear();
    }

    /// Set up the next run of the loop inside this turn: its task (what it
    /// observed before is dropped), its images, whether it may answer with
    /// silence, whether it plans, where it streams.
    pub fn begin_run(&mut self, run: RunSetup) {
        self.task = Some(run.task);
        self.observations.clear();
        if let Some(images) = run.images {
            self.pending_images = images;
        }
        self.silence_ok = run.silence_ok;
        self.plan_mode = run.plan_mode;
        self.thinking_tx = run.thinking;
    }

    /// The run is over, however it ended: nothing streams to its channel,
    /// it plans no more, and its offer of silence and its inbox go.
    pub(crate) fn end_run(&mut self) {
        self.thinking_tx = None;
        self.silence_ok = false;
        self.plan_mode = false;
        self.inbox = None;
    }

    /// Start the run `run_id` names, kept as the turn's last run after it
    /// ends, so what closes the turn names it.
    pub fn run_as(&mut self, run_id: Option<String>) {
        self.set_run_id(run_id.clone());
        self.last_run_id = run_id;
    }

    /// What this turn spent and what failed in it, taken by its caller.
    pub fn take_spend(
        &mut self,
    ) -> (
        crate::provider::models::RunUsage,
        crate::engine::run_faults::RunFaults,
    ) {
        (
            std::mem::take(&mut self.run_usage),
            std::mem::take(&mut self.run_faults),
        )
    }

    /// Where this member sits this turn: at `lead`'s table, with the lead's
    /// declared tools as the session's set — or, with `None`, the lead
    /// itself.
    pub fn seat_at(&mut self, lead: Option<(String, Option<Vec<String>>)>) {
        let (lead, tools) = match lead {
            Some((lead, tools)) => (Some(lead), tools),
            None => (None, None),
        };
        self.session_lead = lead;
        self.session_tools = tools;
    }

    /// The thread was cleared or compacted away: what its runs observed
    /// goes with it.
    pub fn forget_observations(&mut self) {
        self.observations.clear();
    }

    /// Whether images wait for the next run's first message.
    pub fn has_pending_images(&self) -> bool {
        !self.pending_images.is_empty()
    }

    /// Where a person's messages sent during this run wait.
    pub fn take_messages_from(&mut self, inbox: Option<crate::engine::types::Inbox>) {
        self.inbox = inbox;
    }
}

#[cfg(test)]
mod tests;
