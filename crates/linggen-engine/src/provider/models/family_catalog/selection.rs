//! Public BYOK family choice. No probing, mutation, or ambient clock.

use super::{date, preset_for, ModelFacts, Snapshot, Stability};
use crate::config::ModelConfig;
use chrono::NaiveDate;
use std::collections::BTreeSet;

pub(crate) struct Connection<'a> {
    pub endpoint: &'a ModelConfig,
    /// Opaque, non-secret revision of the effective credential.
    pub credential_revision: u64,
}

pub(crate) struct AccessEvidence<'a> {
    pub connection: Connection<'a>,
    /// Exact IDs with proven invocation access, not merely listed IDs.
    pub usable: &'a BTreeSet<String>,
}

pub(crate) enum Access<'a> {
    NoKey,
    Unchecked,
    Failed,
    Listed(&'a AccessEvidence<'a>),
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum Hold {
    NonStable(Stability),
    MissingDate,
    NoChatLane,
    NoAccess,
    Older,
    AmbiguousHead,
    Stale,
    NotNewer,
}

impl Hold {
    /// The word a settings answer carries for why a model is held back.
    pub(crate) fn code(self) -> &'static str {
        match self {
            Hold::NonStable(Stability::Preview) => "preview",
            Hold::NonStable(Stability::Experimental) => "experimental",
            Hold::NonStable(Stability::Alias) => "alias",
            Hold::NonStable(Stability::Deprecated) => "deprecated",
            Hold::NonStable(_) => "not_stable",
            Hold::MissingDate => "missing_date",
            Hold::NoChatLane => "not_chat",
            Hold::NoAccess => "no_access",
            Hold::Older => "older",
            Hold::AmbiguousHead => "same_day",
            Hold::Stale => "stale_catalog",
            Hold::NotNewer => "not_newer",
        }
    }
}

#[derive(Debug)]
pub(crate) struct Choice<'a> {
    pub facts: &'a ModelFacts,
    pub held: Option<Hold>,
}

#[derive(Debug)]
pub(crate) enum FamilyCurrent<'a> {
    Current { current: Option<&'a str>, next: &'a ModelFacts },
    Choose { current: Option<&'a str>, reason: Hold, choices: Vec<Choice<'a>> },
    Checking { next: &'a ModelFacts },
    ListFailed,
    NoKey,
    Unavailable { current: Option<&'a str> },
}

/// Answer for one family at one approved key endpoint. `current` is the
/// existing exact ID, not the endpoint model used to describe a connection.
pub(crate) fn family_current<'a>(
    snapshot: &'a Snapshot,
    connection: Connection<'_>,
    family: &str,
    current: Option<&'a str>,
    access: Access<'_>,
    today: NaiveDate,
) -> FamilyCurrent<'a> {
    let Some(preset) = preset_for(connection.endpoint) else {
        return FamilyCurrent::Unavailable { current };
    };
    let Some(binding) = preset.catalog.filter(|b| b.source_id != "openrouter") else {
        return FamilyCurrent::Unavailable { current };
    };
    let Some(models) = snapshot.providers.get(binding.source_id) else {
        return FamilyCurrent::Unavailable { current };
    };
    let usable = match access {
        Access::Listed(e) if e.connection.credential_revision == connection.credential_revision
            && preset_for(e.connection.endpoint).is_some_and(|p| p.key == preset.key) => Some(e.usable),
        _ => None,
    };
    // Aliases stay in the catalog as exact facts, not in the candidate set.
    let mut family: Vec<_> = models.values().filter(|m| {
        m.family.as_deref() == Some(family) && m.stability != Stability::Alias
            && (!binding.borrowed || usable.is_some_and(|ids| ids.contains(&m.id)))
    }).collect();
    if family.is_empty() {
        return FamilyCurrent::Unavailable { current };
    }
    family.sort_by(|a, b| b.release_date.cmp(&a.release_date).then_with(|| a.id.cmp(&b.id)));
    let head = family.iter().filter_map(|m| day(m)).max();
    let frontier: Vec<_> = family.iter().copied().filter(|m| day(m) == head).collect();
    // A normally deprecated version is harmless beside a stable release on
    // the same date. A deprecated experiment was normalized as experimental.
    let contenders: Vec<_> = frontier.iter().copied()
        .filter(|m| m.stability != Stability::Deprecated).collect();
    let hold_row = |m: &ModelFacts| {
        if m.stability != Stability::Stable { return Some(Hold::NonStable(m.stability)); }
        if day(m).is_none() { return Some(Hold::MissingDate); }
        if !m.text_chat() { return Some(Hold::NoChatLane); }
        if day(m) != head { return Some(Hold::Older); }
        if usable.is_some_and(|ids| !ids.contains(&m.id)) { return Some(Hold::NoAccess); }
        None
    };
    let choose = |reason| FamilyCurrent::Choose {
        current, reason,
        choices: family.iter().map(|m| Choice { facts: m, held: hold_row(m).or(Some(reason)) }).collect(),
    };
    if family.iter().any(|m| m.stability != Stability::Deprecated && day(m).is_none()) {
        return choose(Hold::MissingDate);
    }
    let next = match contenders.as_slice() {
        [only] => *only,
        [] => return choose(Hold::NonStable(Stability::Deprecated)),
        _ => return choose(Hold::AmbiguousHead),
    };
    if let Some(reason) = hold_row(next) {
        return choose(reason);
    }
    if usable.is_none() {
        return match access {
            Access::NoKey => FamilyCurrent::NoKey,
            Access::Failed => FamilyCurrent::ListFailed,
            Access::Unchecked | Access::Listed(_) => FamilyCurrent::Checking { next },
        };
    }
    if let Some(id) = current {
        if next.id != id {
            if snapshot.is_stale(today) { return choose(Hold::Stale); }
            let Some(previous) = models.get(id).and_then(day) else { return choose(Hold::MissingDate); };
            if Some(previous) >= head { return choose(Hold::NotNewer); }
        }
    }
    FamilyCurrent::Current { current, next }
}

fn day(model: &ModelFacts) -> Option<NaiveDate> {
    model.release_date.as_deref().and_then(date)
}

#[cfg(test)]
mod tests;
