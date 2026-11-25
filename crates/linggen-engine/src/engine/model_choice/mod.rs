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

use crate::config::{ModelConfig, ProviderKind, SignIn};
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

/// Who asks, as an owner of what a guest's [`Asker`] borrows: a call that
/// needs the asker across an `await` or beside a mutable use of the engine
/// holds this, and takes [`AskedBy::asker`] from it. The one place that turns
/// "the models this asker was given, if it was given a list" into an
/// [`Asker`] (lock: `lock_tests::an_asker_is_made_in_one_place`).
#[derive(Debug, Clone)]
pub struct AskedBy(Option<Vec<String>>);

impl AskedBy {
    /// `shared`: the models a guest or a household person was given; `None`
    /// is the owner.
    pub fn of(shared: Option<Vec<String>>) -> Self {
        Self(shared)
    }

    pub fn asker(&self) -> Asker<'_> {
        match &self.0 {
            Some(shared) => Asker::Guest(shared),
            None => Asker::Owner,
        }
    }
}

/// What the session lets the choice do.
#[derive(Debug, Clone, Copy)]
pub struct Rule<'a> {
    pub asker: Asker<'a>,
    /// Whether an asked model that cannot run gives way to the standing one:
    /// `Switching::session_allows` at a door, or a named constant of this
    /// module for a place with no switch of its own.
    ///
    /// Off, an owner's turn keeps an unavailable asked model and fails on it,
    /// named. A guest's list is an access rule, not a failure: it holds
    /// whatever the switch says.
    pub may_switch: bool,
}

/// The two switches that say whether a turn moves to another model: the
/// routing config's (`routing.model_fallback`, held by the engine as it last
/// read its settings) and the session's own (`SessionMeta::model_fallback`).
/// Each answers its own question, and who asks it:
///
/// - [`Switching::session_allows`] — may a model asked for before the turn
///   starts, which this machine lacks, give way to the standing one? Only the
///   session's switch says no. Asked by the doors (chat, her turn, a mission)
///   and by the stream for a model this machine lacks.
/// - [`Switching::may`] — may a call that failed move down the chain? Both
///   switches must be on. Asked by the mid-call fallback.
///
/// Only [`AgentEngine::switching`] reads them (lock:
/// `lock_tests::the_two_switches_are_each_read_in_one_place`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Switching {
    config: bool,
    session: bool,
}

impl Switching {
    /// Whether a failed call may move to another model: both switches are on.
    pub fn may(self) -> bool {
        self.config && self.session
    }

    /// The session's switch alone: whether an asked model this machine lacks
    /// gives way to the standing one. A session that may not switch fails on
    /// it, and says that switching is off for the session.
    pub fn session_allows(self) -> bool {
        self.session
    }
}

/// A delegated run (`agent_run`) never switches: a model that cannot run
/// fails the run, named.
pub const DELEGATED_RUN_MAY_SWITCH: bool = false;

/// Asking outside a turn (the prompt preview, a summary, a push line, a
/// member's named model) always lets an unavailable asked model give way to
/// the standing one: there is no turn to fail on it.
pub const OUTSIDE_A_TURN_MAY_SWITCH: bool = true;

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

    /// Whether the asker was given no model that can run: the choice names
    /// the standing model, which a turn's stream refuses (`Why::NothingShared`).
    /// A call made outside a stream (a summary, a push line) must not run it.
    pub fn nothing_given(&self) -> bool {
        self.outcome == Outcome::Replaced(Why::NothingShared)
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

/// The first model of `chain` that is Anthropic's and can run now, for a
/// conversation that must go on on a Claude model (an old Claude session
/// continued). "Can run" is a key, or Claude's sign-in when it holds the
/// right scope and is not expired. Nothing when there is none: the caller
/// takes its default.
pub fn first_claude(models: &ModelManager, chain: &[String]) -> Option<String> {
    chain.iter().find_map(|id| {
        let live = models.resolve_id(id)?;
        let config = models.list_models().into_iter().find(|m| m.id == live)?;
        can_run_claude(config, || models.model_auth_ok(&live)).then_some(live)
    })
}

/// Whether `config` is an Anthropic model with what it needs to answer:
/// a non-empty key, or a Claude sign-in that `signed_in` vouches for.
fn can_run_claude(config: &ModelConfig, signed_in: impl FnOnce() -> bool) -> bool {
    if config.provider_kind() != ProviderKind::Anthropic {
        return false;
    }
    match config.sign_in() {
        SignIn::Key => config.api_key.as_deref().is_some_and(|key| !key.is_empty()),
        SignIn::Claude => signed_in(),
        SignIn::ChatGpt | SignIn::LinggenAccount | SignIn::Unknown => false,
    }
}

/// The model a sign-in makes the default: when the default cannot run now,
/// the first model that signs in the way the person just did and can run.
/// A default that can run is never changed. The one place this is decided;
/// the sign-in's end asks it.
pub fn after_sign_in(
    models: &ModelManager,
    default: Option<&str>,
    signed_in: SignIn,
) -> Option<String> {
    let default_ready = default
        .and_then(|id| live(models, Asker::Owner, id).ok())
        .and_then(|id| models.readiness(&id))
        // A default only rate-limited for a minute still runs: a person who
        // chose one keeps it. Only a missing sign-in or key strands it.
        .is_some_and(|r| !r.lacks_credentials());
    pick_after_sign_in(default_ready, &models.ready_in_order(), signed_in)
}

/// The one line a page says when a sign-in made `model` the default.
pub fn made_default_line(model: &str) -> String {
    format!("{model} is now your default model.")
}

/// A run that cannot start for want of a sign-in says what to do next:
/// the owner is told the first ready model (or that none is). A room's
/// guest is told nothing of the owner's models; a refusal that is not a
/// missing sign-in is left as it was.
pub fn say_next_step(err: anyhow::Error, models: &ModelManager, asker: Asker<'_>) -> anyhow::Error {
    let signed_out = err.to_string().contains("AUTH_REQUIRED");
    if !signed_out || !matches!(asker, Asker::Owner) {
        return err;
    }
    anyhow::anyhow!("{err} {}", models.next_step())
}

/// [`after_sign_in`]'s rule, on facts: whether the default can run, and the
/// ready models in the order they are offered.
fn pick_after_sign_in(
    default_ready: bool,
    ready: &[&ModelConfig],
    signed_in: SignIn,
) -> Option<String> {
    if default_ready {
        return None;
    }
    ready
        .iter()
        .find(|m| m.sign_in() == signed_in)
        .map(|m| m.id.clone())
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
    /// Start a turn on the machine as it is now: the models as they are (a
    /// model registered after this engine was built — a room's proxy model,
    /// joined mid-session — is one its turn can run on), and whatever
    /// settings or skills changed since (`read_changes_if_any`). Called by
    /// every door before it chooses the turn's model.
    pub async fn read_live_state(&mut self, manager: &crate::engine::agent::AgentManager) {
        self.model_manager = manager.models.read().await.clone();
        self.read_changes_if_any(manager).await;
    }

    /// What the two switches let the turn in session `session_id` do (`None`:
    /// no session, which never forbids). The one reader of both: the config's
    /// is the engine's last reading of its settings, so a door asks after
    /// [`Self::read_live_state`].
    pub fn switching(&self, session_id: Option<&str>) -> Switching {
        let session = match (self.tools.get_manager(), session_id) {
            (Some(manager), Some(id)) => manager.global_sessions.model_fallback_allowed(id),
            _ => true,
        };
        Switching {
            config: self.allow_model_fallback,
            session,
        }
    }

    /// The agent's default model, read again after a change of settings
    /// (`read_changes_if_any`): an engine that followed its default follows
    /// the new one, and one on a model this machine no longer has goes to
    /// it — a removed model is not a later turn's. `None`: the chain gave
    /// none, and the standing default is kept.
    pub(crate) fn follow_default_model(&mut self, default: Option<String>) {
        let following = self.model_id == self.default_model_id;
        if let Some(default) = default {
            self.default_model_id = default;
        }
        if following || !is_here(&self.model_manager, &self.model_id) {
            self.model_id = self.default_model_id.clone();
        }
    }

    /// Run this turn on `choice`. The one writer of the turn's model before
    /// it starts; `place` names the turn in the log. A replaced model is told
    /// as one replaced during a call is: the ModelFallback event.
    pub async fn run_on(&mut self, choice: &Choice, place: &str) {
        log_choice(choice, place);
        self.model_id = choice.model.clone();
        self.tell_replaced(choice).await;
    }

    /// Start a Task child: it runs on its parent's model, as a Claude Code
    /// subagent does — unless its agent declares a model of its own
    /// (`declares_own_model`), which wins. The child is asked as its policy
    /// names: an owner's runs the parent's model, a guest's the parent's too
    /// (the room shares it) or the first the room shares. Whether a model
    /// that cannot run gives way follows the parent's session
    /// ([`Switching::session_allows`]). `parent_model` is `None` where the
    /// caller does not know the parent's (the child then runs its agent's own
    /// model).
    pub async fn run_as_child(&mut self, parent_model: Option<&str>, place: &str) {
        let asker = self
            .tools
            .builtins
            .session_policy
            .as_ref()
            .map_or(Asker::Owner, |p| p.asker());
        let session = self.tools.builtins.session_id.clone();
        let rule = Rule {
            asker,
            may_switch: self.switching(session.as_deref()).session_allows(),
        };
        let asked = parent_model.filter(|_| !self.declares_own_model);
        let choice = resolve(&self.model_manager, asked, &self.default_model_id, rule);
        self.run_on(&choice, place).await;
    }

    /// Who asks for models on this engine, by its policy: the models a
    /// guest's policy shares, or the owner.
    pub fn asked_by(&self) -> AskedBy {
        let shared = self
            .tools
            .builtins
            .session_policy
            .as_ref()
            .and_then(|policy| policy.allowed_models.clone());
        AskedBy::of(shared)
    }

    async fn tell_replaced(&self, choice: &Choice) {
        let (Outcome::Replaced(why), Some(asked)) = (choice.outcome, choice.asked.as_deref())
        else {
            return;
        };
        // Nothing shared runs nothing: the stream's refusal is what is told.
        if why == Why::NothingShared {
            return;
        }
        self.emit(crate::engine::agent::AgentEvent::ModelFallback {
            agent_id: self.agent_id.clone().unwrap_or_else(|| "unknown".into()),
            preferred_model: asked.to_string(),
            actual_model: choice.model.clone(),
            reason: why.says().to_string(),
            run_id: self.run_id.clone(),
            subagent: self.parent_run_id.is_some(),
        })
        .await;
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
