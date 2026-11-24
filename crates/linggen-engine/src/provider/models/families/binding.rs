//! The one writer of a family binding.
//!
//! A ticked family is an ordinary `[[models]]` row with a stable local id, the
//! real endpoint and the current wire `model` (all fields the released 2.6.1
//! binary reads), plus [`FamilyBinding`]. Nothing outside this file builds or
//! changes a binding: the fields are private, every mutation is a function
//! here, and a source scan (`src/server/model_locks.rs`) fails when a second
//! file writes `family_binding`. A row without a binding is an exact row and
//! no function here ever moves it; nothing reads "follow" from an id or a wire
//! name.
//!
//! Every function works on a `&mut Config` the caller holds under the settings
//! write lock and saves in the same config write, so a move and its notice
//! are one change.

use super::super::family_catalog::{preset_for, ModelFacts};
use crate::config::{providers::Preset, Config, ModelConfig};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Whether a row's model moves when the catalog names a newer one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Follow,
    Pinned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveKind {
    /// The family moved to a newer model.
    Followed,
    /// The first call on the newer model was refused: the row went back by
    /// itself and is pinned.
    WentBack,
}

/// The latest change of a row's model, kept until acknowledged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LastMove {
    seq: u64,
    kind: MoveKind,
    from: String,
    to: String,
    on: String,
    #[serde(default)]
    acknowledged: bool,
}

impl LastMove {
    pub fn seq(&self) -> u64 { self.seq }
    pub fn kind(&self) -> MoveKind { self.kind }
    pub fn from(&self) -> &str { &self.from }
    pub fn to(&self) -> &str { &self.to }
    pub fn on(&self) -> &str { &self.on }
    pub fn acknowledged(&self) -> bool { self.acknowledged }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FamilyBinding {
    family: String,
    /// The preset's key: which endpoint's catalog the family is read from.
    preset: String,
    mode: Mode,
    /// The release date of the model the row runs, as the catalog gave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    released: Option<String>,
    /// Moves made so far; the latest carries this number.
    #[serde(default)]
    moves: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_move: Option<LastMove>,
}

impl FamilyBinding {
    pub fn family(&self) -> &str { &self.family }
    pub fn preset(&self) -> &str { &self.preset }
    pub fn mode(&self) -> Mode { self.mode }
    pub fn follows(&self) -> bool { self.mode == Mode::Follow }
    pub fn released(&self) -> Option<&str> { self.released.as_deref() }
    pub fn last_move(&self) -> Option<&LastMove> { self.last_move.as_ref() }
    /// The move a person has yet to answer.
    pub fn notice(&self) -> Option<&LastMove> { self.last_move.as_ref().filter(|m| !m.acknowledged) }
}

/// Preset keys whose starter families were ticked once.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FamilyStarts(Vec<String>);

impl FamilyStarts {
    pub fn is_empty(&self) -> bool { self.0.is_empty() }
    pub fn contains(&self, preset_key: &str) -> bool { self.0.iter().any(|k| k == preset_key) }
}

fn row_mut<'a>(config: &'a mut Config, id: &str) -> Option<(&'a mut ModelConfig, FamilyBinding)> {
    let row = config.models.iter_mut().find(|row| row.id == id)?;
    let binding = row.family_binding.clone()?;
    Some((row, binding))
}

/// The wire model and what the catalog says about it, in the fields the
/// released binary already reads.
fn run_on(row: &mut ModelConfig, facts: &ModelFacts) {
    row.model = facts.id().to_string();
    row.context_window = facts.context().map(|n| n as usize);
    row.supports_tools = facts.tool_call();
}

fn new_binding(preset: &Preset, family: &str, facts: &ModelFacts, mode: Mode) -> FamilyBinding {
    FamilyBinding {
        family: family.into(), preset: preset.key.into(), mode,
        released: facts.release_date().map(str::to_string), moves: 0, last_move: None,
    }
}

fn ordinary_at(row: &ModelConfig, preset: &Preset, model: &str) -> bool {
    !row.is_builtin && row.family_binding.is_none() && row.model == model
        && preset_for(row).is_some_and(|p| p.key == preset.key)
}

pub(super) fn duplicate<'a>(config: &'a Config, preset: &Preset, model: &str) -> Option<&'a ModelConfig> {
    config.models.iter().find(|row| ordinary_at(row, preset, model))
}

pub(super) fn adopt(config: &mut Config, preset: &Preset, family: &str, facts: &ModelFacts) -> bool {
    let Some(row) = config.models.iter_mut().find(|row| ordinary_at(row, preset, facts.id())) else { return false };
    row.family_binding = Some(new_binding(preset, family, facts, Mode::Follow));
    true
}

/// The named target must still be the ordinary duplicate we offered.
pub(super) fn merge(config: &mut Config, preset: &Preset, family: &str, target: &str) -> Option<String> {
    let row = row_of(config, preset.key, family)?;
    let source = row.id.clone();
    let binding = row.family_binding.clone()?;
    let original = duplicate(config, preset, &row.model).filter(|r| r.id == target)?;
    let target = original.id.clone();
    config.models.iter_mut().find(|r| r.id == target)?.family_binding = Some(binding);
    let mut seen_target = false;
    config.routing.default_models.retain_mut(|id| {
        if *id == source { *id = target.clone(); }
        if *id != target { return true; }
        !std::mem::replace(&mut seen_target, true)
    });
    if config.pet.model == source { config.pet.model = target.clone(); }
    for agent in &mut config.agents {
        if agent.model.as_deref() == Some(&source) { agent.model = Some(target.clone()); }
    }
    remove(config, &source);
    Some(source)
}

/// File a family as a new row running `facts`. The caller has checked that
/// the family is not ticked at this endpoint.
pub(super) fn tick(config: &mut Config, preset: &Preset, family: &str, facts: &ModelFacts, mode: Mode) -> String {
    let id = config.fresh_model_id("fam");
    let mut row = ModelConfig {
        id: id.clone(),
        provider: preset.name.into(),
        url: preset.url.into(),
        model: String::new(),
        api_key: None,
        keep_alive: None,
        context_window: None,
        tags: Vec::new(),
        supports_tools: None,
        auth_mode: None,
        reasoning_effort: None,
        max_output_tokens: None,
        price: None,
        provided_by: None,
        is_builtin: false,
        family_binding: Some(new_binding(preset, family, facts, mode)),
    };
    run_on(&mut row, facts);
    config.models.push(row);
    id
}

/// Unfile a family's row. References to its id are left as they are: they
/// become unavailable, never another provider's model.
pub(super) fn remove(config: &mut Config, id: &str) -> bool {
    let before = config.models.len();
    config.models.retain(|row| !(row.id == id && row.family_binding.is_some()));
    config.models.len() != before
}

/// Move a following row from the model it was seen on to `facts`, with the
/// notice, in the same change. Only a row that still runs `from` and still
/// follows moves.
pub(super) fn follow_to(config: &mut Config, id: &str, from: &str, facts: &ModelFacts, today: NaiveDate) -> bool {
    let Some((row, mut binding)) = row_mut(config, id) else { return false };
    if !binding.follows() || row.model != from || facts.id() == from {
        return false;
    }
    binding.moves += 1;
    binding.released = facts.release_date().map(str::to_string);
    binding.last_move = Some(LastMove {
        seq: binding.moves,
        kind: MoveKind::Followed,
        from: from.into(),
        to: facts.id().into(),
        on: today.to_string(),
        acknowledged: false,
    });
    run_on(row, facts);
    row.family_binding = Some(binding);
    true
}

/// The newer model refused its first call: back to the one before, pinned,
/// with a notice that says so. Only the move still outstanding is undone.
pub(super) fn went_back(config: &mut Config, id: &str, seq: u64, previous: Option<&ModelFacts>, today: NaiveDate) -> bool {
    let Some((row, mut binding)) = row_mut(config, id) else { return false };
    let Some(last) = binding.notice().filter(|m| m.seq == seq && m.kind == MoveKind::Followed).cloned() else {
        return false;
    };
    if row.model != last.to {
        return false;
    }
    binding.mode = Mode::Pinned;
    binding.moves += 1;
    binding.last_move = Some(LastMove {
        seq: binding.moves,
        kind: MoveKind::WentBack,
        from: last.to,
        to: last.from.clone(),
        on: today.to_string(),
        acknowledged: false,
    });
    match previous {
        Some(facts) => {
            binding.released = facts.release_date().map(str::to_string);
            run_on(row, facts);
        }
        None => row.model = last.from,
    }
    row.family_binding = Some(binding);
    true
}

/// 知道了: the notice of move `seq` is answered.
pub(super) fn acknowledge(config: &mut Config, id: &str, seq: u64) -> bool {
    let Some((row, mut binding)) = row_mut(config, id) else { return false };
    let Some(last) = binding.last_move.as_mut().filter(|m| m.seq == seq && !m.acknowledged) else { return false };
    last.acknowledged = true;
    row.family_binding = Some(binding);
    true
}

/// 固定在这个版本: stop following. A move named by `seq` must still be the
/// latest; an unnamed pin is a plain choice.
pub(super) fn pin(config: &mut Config, id: &str, seq: Option<u64>) -> bool {
    let Some((row, mut binding)) = row_mut(config, id) else { return false };
    if binding.mode == Mode::Pinned || seq.is_some_and(|n| binding.last_move.as_ref().is_none_or(|m| m.seq != n)) {
        return false;
    }
    binding.mode = Mode::Pinned;
    row.family_binding = Some(binding);
    true
}

/// 恢复自动跟上: follow again. The caller then asks the selector; nothing
/// moves here.
pub(super) fn resume(config: &mut Config, id: &str) -> bool {
    let Some((row, mut binding)) = row_mut(config, id) else { return false };
    if binding.follows() {
        return false;
    }
    binding.mode = Mode::Follow;
    row.family_binding = Some(binding);
    true
}

/// A version chosen by hand: the row runs it, pinned.
pub(super) fn choose(config: &mut Config, id: &str, facts: &ModelFacts) -> bool {
    let Some((row, mut binding)) = row_mut(config, id) else { return false };
    if row.model == facts.id() && binding.mode == Mode::Pinned {
        return false;
    }
    binding.mode = Mode::Pinned;
    binding.released = facts.release_date().map(str::to_string);
    run_on(row, facts);
    row.family_binding = Some(binding);
    true
}

/// 换回: back to the model before move `seq`, pinned, and that notice answered.
pub(super) fn rollback(config: &mut Config, id: &str, seq: u64, previous: &ModelFacts) -> bool {
    let Some((row, mut binding)) = row_mut(config, id) else { return false };
    let Some(last) = binding.last_move.as_mut().filter(|m| m.seq == seq && m.kind == MoveKind::Followed && !m.acknowledged) else {
        return false;
    };
    if last.from != previous.id() || row.model != last.to {
        return false;
    }
    last.acknowledged = true;
    binding.mode = Mode::Pinned;
    binding.released = previous.release_date().map(str::to_string);
    run_on(row, previous);
    row.family_binding = Some(binding);
    true
}

/// Record that an endpoint's starter families were handled, once.
pub(super) fn mark_started(config: &mut Config, preset_key: &str) -> bool {
    if config.family_starts.contains(preset_key) {
        return false;
    }
    config.family_starts.0.push(preset_key.into());
    true
}

/// A full-config save from the legacy settings route, made against an older
/// read: it never changes a bound row's model or binding, never sets a
/// binding, and never forgets the recorded starts. `current` is the config
/// in force under the same write lock.
pub(crate) fn keep_bindings(next: &mut Config, current: &Config) {
    next.family_starts = current.family_starts.clone();
    for row in &mut next.models {
        row.family_binding = None;
    }
    for held in current.models.iter().filter(|row| row.family_binding.is_some()) {
        let Some(row) = next.models.iter_mut().find(|row| row.id == held.id) else { continue };
        row.family_binding = held.family_binding.clone();
        row.model = held.model.clone();
        row.context_window = held.context_window;
        row.supports_tools = held.supports_tools;
    }
}

/// Whether a row still sits at the endpoint of the preset its binding names.
/// A row whose address was edited away is an exact row from then on.
fn at_its_preset(row: &ModelConfig, binding: &FamilyBinding) -> bool {
    preset_for(row).is_some_and(|p| p.key == binding.preset)
}

/// The row a preset's family is ticked as.
pub(super) fn row_of<'a>(config: &'a Config, preset_key: &str, family: &str) -> Option<&'a ModelConfig> {
    config.models.iter().find(|row| {
        row.family_binding.as_ref().is_some_and(|b| b.preset == preset_key && b.family == family && at_its_preset(row, b))
    })
}

/// Every row that follows its family, at its preset's endpoint.
pub(super) fn following(config: &Config) -> Vec<&ModelConfig> {
    config.models.iter()
        .filter(|row| row.family_binding.as_ref().is_some_and(|b| b.follows() && at_its_preset(row, b)))
        .collect()
}
