//! What a person's action on one family does. Each action changes the config
//! in memory only, through `binding`; the caller saves it in one config write.
//! An action that names a move answers that move: a repeated or stale one
//! changes nothing and says so (`Ok(false)`).

use super::{binding, Endpoint, Families};
use crate::config::Config;
use crate::credentials::Credentials;
use super::super::family_catalog::{FamilyCurrent, Target};
use super::super::curated::EntryType;

pub enum Action {
    /// Tick the family: its row runs the model it means now.
    Tick,
    /// Remove the family's row. Refused while the row is the first default
    /// chat model, unless `replacement` names the model that takes its place.
    Untick { replacement: Option<String> },
    /// 恢复自动跟上.
    Follow,
    /// Run this exact version, pinned.
    Choose { model: String },
    /// 知道了.
    Ack { moved: u64 },
    /// 换回: back to the model before the move, pinned, after access is re-checked.
    Back { moved: u64 },
}

/// Why an action was not carried out. Each has one fixed code.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Refusal {
    /// This endpoint has nothing to tick in the list.
    Unsupported,
    /// No such family, row or version here.
    Missing,
    /// No key for this endpoint.
    NoKey,
    /// The list's approved model cannot be used yet: the person picks.
    NeedsChoice,
    /// The provider's list does not hold the model.
    NoAccess,
    /// The row is the first default chat model: name a replacement.
    FirstDefault,
    /// The model before the move is not a chat model, or the list no longer knows it.
    NoPrevious,
}

impl Action {
    /// Whether the action decides on the provider's list: gather it first.
    /// Notice answers, pins and unticks need no list.
    pub fn needs_list(&self) -> bool {
        matches!(self, Action::Tick | Action::Follow | Action::Choose { .. } | Action::Back { .. })
    }
}

impl Refusal {
    pub fn code(self) -> &'static str {
        match self {
            Refusal::Unsupported => "unsupported",
            Refusal::Missing => "missing",
            Refusal::NoKey => "no_key",
            Refusal::NeedsChoice => "needs_choice",
            Refusal::NoAccess => "no_access",
            Refusal::FirstDefault => "first_default",
            Refusal::NoPrevious => "no_previous",
        }
    }
}

pub(super) fn run(f: &Families, config: &mut Config, creds: &Credentials, (name, url): (&str, &str), family: &str, action: Action) -> Result<bool, Refusal> {
    let list = f.catalog();
    let endpoint = Endpoint::of(&list, config.model_catalog.offered(), name, url, creds).ok_or(Refusal::Unsupported)?;
    let today = f.today();
    let place = endpoint.place;
    let entry = place.entry_for(family).ok_or(Refusal::Missing)?;
    let listing = endpoint.listing(f);
    let row = binding::row_of(&list, config, place.endpoint_key, family).map(|row| (row.id.clone(), row.model.clone()));
    let needs_row = || row.clone().ok_or(Refusal::Missing);
    match action {
        Action::Tick => {
            if row.is_some() {
                return Ok(false);
            }
            // The account is its own key, and no provider list says what it serves.
            let listing = match place.is_account() {
                true => None,
                false => {
                    endpoint.key.as_ref().ok_or(Refusal::NoKey)?;
                    Some(listing.ok_or(Refusal::NeedsChoice)?)
                }
            };
            match endpoint.ask(&list, family, None, listing.as_deref(), today) {
                FamilyCurrent::Current { next, .. } => {
                    // An exact entry is a pin from the start; a family follows.
                    let mode = if entry.kind == EntryType::Exact { binding::Mode::Pinned } else { binding::Mode::Follow };
                    binding::tick(config, &place, family, next, mode);
                    Ok(true)
                }
                _ => Err(Refusal::NeedsChoice),
            }
        }
        Action::Untick { replacement } => {
            let Some((id, _)) = row else { return Ok(false) };
            if config.routing.default_models.first() == Some(&id) {
                let replacement = replacement.filter(|r| !r.is_empty() && *r != id).ok_or(Refusal::FirstDefault)?;
                config.make_default(&replacement);
            }
            f.disarm(&id);
            Ok(binding::remove(config, &id))
        }
        Action::Follow => {
            let (id, model) = needs_row()?;
            // An exact entry stays put: only a family follows.
            if entry.kind == EntryType::Exact {
                return Err(Refusal::Unsupported);
            }
            let resumed = binding::resume(config, &id);
            let next = match (&endpoint.key, &listing) {
                (Some(_), Some(listing)) => match endpoint.ask(&list, family, Some(&model), Some(listing), today) {
                    FamilyCurrent::Current { next, .. } if next.id != model => Some(next),
                    _ => None,
                },
                _ => None,
            };
            let moved = next.is_some_and(|target| binding::follow_to(config, &id, &model, target, today));
            if moved {
                f.arm(&id);
            }
            Ok(resumed || moved)
        }
        Action::Choose { model } => {
            endpoint.key.as_ref().ok_or(Refusal::NoKey)?;
            let facts = list.facts_for(place.endpoint_key, &model)
                .filter(|m| m.family() == Some(family) && m.chooseable())
                .ok_or(Refusal::Missing)?;
            if listing.as_ref().is_some_and(|l| !l.usable.contains(&model)) {
                return Err(Refusal::NoAccess);
            }
            let target = Target { id: facts.id(), facts: Some(facts) };
            match row {
                Some((id, _)) => {
                    f.disarm(&id);
                    Ok(binding::choose(config, &id, target))
                }
                None => {
                    binding::tick(config, &place, family, target, binding::Mode::Pinned);
                    Ok(true)
                }
            }
        }
        Action::Ack { moved } => {
            let (id, _) = needs_row()?;
            f.disarm(&id);
            Ok(binding::acknowledge(config, &id, moved))
        }
        Action::Back { moved } => {
            let (id, _) = needs_row()?;
            let before = binding::row_of(&list, config, place.endpoint_key, family)
                .and_then(|row| row.family_binding.as_ref()?.notice())
                .filter(|m| m.seq() == moved && m.kind() == binding::MoveKind::Followed)
                .map(|m| m.from().to_string());
            // A stale or repeated 换回 changes nothing.
            let Some(before) = before else { return Ok(false) };
            let previous = Target { id: &before, facts: list.facts_for(place.endpoint_key, &before) };
            if !previous.is_chat() {
                return Err(Refusal::NoPrevious);
            }
            if !listing.as_ref().is_some_and(|l| l.usable.contains(&before)) {
                return Err(Refusal::NoAccess);
            }
            f.disarm(&id);
            Ok(binding::rollback(config, &id, moved, previous))
        }
    }
}
