//! One rule for which model a turn runs on, and what happens when that
//! model cannot answer.
//!
//! Every place that starts a turn — a chat turn, the companion's turn, a
//! mission's run, a delegated run — asks [`resolve`] and writes its answer
//! with [`AgentEngine::run_on`]. A place keeps only what is truly its own:
//! where its asked model comes from (a picker, a member's kept model, the
//! companion's setting, a mission's `model:`) and its standing model (what it
//! runs on when nothing is asked). The agent's own priority chain
//! (`AgentManager::resolve_model_id`) is how a standing model is found; it
//! uses [`live`] for its candidates. A model that fails during the call is
//! replaced by [`next_down`], in the stream (`stream_with_fallback`).
//!
//! The lock (`lock_tests.rs`) fails when a model id is written, or a model's
//! availability is checked, anywhere else.

use crate::engine::AgentEngine;
use crate::provider::models::ModelManager;

#[cfg(test)]
mod lock_tests;
#[cfg(test)]
mod tests;

/// Who asks for the model.
#[derive(Debug, Clone, Copy)]
pub enum Asker<'a> {
    /// The person this machine belongs to: any model it has.
    Owner,
    /// A room's guest: only the models the room shares (`shared_models`).
    Guest(&'a [String]),
}

/// What the session lets the choice do.
#[derive(Debug, Clone, Copy)]
pub struct Rule<'a> {
    pub asker: Asker<'a>,
    /// The session's model-switching switch (`SessionMeta::model_fallback`).
    /// Off, an owner's turn keeps an unavailable asked model and fails on it,
    /// named. A guest's list is an access rule, not a failure: it holds
    /// whatever the switch says.
    pub may_switch: bool,
}

/// Why the model asked for is not the one that runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// This machine has no model by that id (nor a successor of it).
    NotHere,
    /// The room does not share it with this guest. The stream refuses such
    /// a model (`stream_with_fallback`), so a guest never runs one.
    NotShared,
    /// The room shares no model this machine has. The choice names the
    /// standing model, which the stream then refuses as not shared.
    NothingShared,
}

impl Why {
    pub fn says(self) -> &'static str {
        match self {
            Why::NotHere => "not configured on this machine",
            Why::NotShared => "not shared in this room",
            Why::NothingShared => "this room shares no model",
        }
    }
}

/// How the choice came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The model asked for (a retired id as its successor).
    Asked,
    /// Nothing was asked: the place's standing model.
    Standing,
    /// The asked model cannot run and switching is off: it is kept, so the
    /// turn fails on it, named.
    Kept(Why),
    /// The asked model cannot run: another one runs instead.
    Replaced(Why),
}

/// The model a turn runs on. Only [`resolve`] makes one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    model: String,
    asked: Option<String>,
    outcome: Outcome,
}

impl Choice {
    fn new(model: impl Into<String>, asked: Option<&str>, outcome: Outcome) -> Self {
        Self {
            model: model.into(),
            asked: asked.map(str::to_string),
            outcome,
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    /// Whether the model that runs is the one asked for.
    pub fn is_asked(&self) -> bool {
        self.outcome == Outcome::Asked
    }

    /// Why the asked model does not run as asked, if it does not.
    pub fn why(&self) -> Option<Why> {
        match self.outcome {
            Outcome::Kept(why) | Outcome::Replaced(why) => Some(why),
            Outcome::Asked | Outcome::Standing => None,
        }
    }

    /// Whether a model a session keeps for the member should be cleared: it
    /// was replaced, so keeping it would make every turn ask for it again.
    pub fn clears_pin(&self) -> bool {
        matches!(self.outcome, Outcome::Replaced(_))
    }
}

/// The registered id `id` runs as for `asker`: itself, or the successor of
/// a retired id — or why it cannot run.
pub fn live(models: &ModelManager, asker: Asker<'_>, id: &str) -> Result<String, Why> {
    let live = models.resolve_id(id).ok_or(Why::NotHere)?;
    let Asker::Guest(shared) = asker else {
        return Ok(live);
    };
    let is_shared = shared
        .iter()
        .any(|m| models.resolve_id(m).as_deref() == Some(live.as_str()));
    is_shared.then_some(live).ok_or(Why::NotShared)
}

/// Whether this machine has a model by `id` (or a successor of it).
pub fn is_here(models: &ModelManager, id: &str) -> bool {
    models.has_model(id)
}

/// The model a turn runs on: `asked` when it can run for this asker, else
/// the standing model — unless switching is off, when an owner's turn keeps
/// what it asked for and fails on it, named.
pub fn resolve(
    models: &ModelManager,
    asked: Option<&str>,
    standing: &str,
    rule: Rule<'_>,
) -> Choice {
    let Some(id) = asked else {
        return standing_for(models, standing, rule.asker);
    };
    let why = match live(models, rule.asker, id) {
        Ok(model) => return Choice::new(model, asked, Outcome::Asked),
        Err(why) => why,
    };
    let keeps_asked = !rule.may_switch && matches!(rule.asker, Asker::Owner);
    if keeps_asked {
        return Choice::new(id, asked, Outcome::Kept(why));
    }
    instead(models, standing, asked, rule.asker, why)
}

/// Nothing asked: the owner's standing model, a guest's first shared one.
fn standing_for(models: &ModelManager, standing: &str, asker: Asker<'_>) -> Choice {
    let Asker::Guest(shared) = asker else {
        return Choice::new(standing, None, Outcome::Standing);
    };
    match first_shared(models, shared) {
        Some(model) => Choice::new(model, None, Outcome::Standing),
        None => Choice::new(standing, None, Outcome::Replaced(Why::NothingShared)),
    }
}

/// What runs in place of an asked model that cannot.
fn instead(
    models: &ModelManager,
    standing: &str,
    asked: Option<&str>,
    asker: Asker<'_>,
    why: Why,
) -> Choice {
    let Asker::Guest(shared) = asker else {
        return Choice::new(standing, asked, Outcome::Replaced(why));
    };
    match first_shared(models, shared) {
        Some(model) => Choice::new(model, asked, Outcome::Replaced(why)),
        None => Choice::new(standing, asked, Outcome::Replaced(Why::NothingShared)),
    }
}

fn first_shared(models: &ModelManager, shared: &[String]) -> Option<String> {
    shared.iter().find_map(|id| models.resolve_id(id))
}

/// The next model down when the one in use cannot answer during a call: the
/// routing chain in order, then the always-present Linggen Cloud model —
/// each one this asker may run, signed in, not benched and not yet tried.
pub fn next_down(
    models: &ModelManager,
    asker: Asker<'_>,
    chain: &[String],
    tried: &[String],
) -> Option<String> {
    let cloud = crate::provider::models::LINGGEN_CLOUD_MODEL_ID.to_string();
    chain
        .iter()
        .cloned()
        .chain(std::iter::once(cloud))
        .find(|m| {
            !tried.contains(m)
                && live(models, asker, m).is_ok()
                && models.model_auth_ok(m)
                && !crate::provider::models::in_cooldown(m)
        })
}

impl AgentEngine {
    /// Run this turn on `choice`. The one writer of the turn's model before
    /// it starts; `place` names the turn in the log.
    pub async fn run_on(&mut self, choice: &Choice, place: &str) {
        log_choice(choice, place);
        self.model_id = choice.model.clone();
    }
}

fn log_choice(choice: &Choice, place: &str) {
    let asked = choice.asked.as_deref().unwrap_or("(default)");
    match choice.outcome {
        Outcome::Asked | Outcome::Standing => {}
        Outcome::Kept(why) => tracing::warn!(
            "{place}: model '{asked}' {} — model switching is off, not falling back",
            why.says()
        ),
        Outcome::Replaced(why) => tracing::warn!(
            "{place}: model '{asked}' {} — running on '{}'",
            why.says(),
            choice.model
        ),
    }
}
