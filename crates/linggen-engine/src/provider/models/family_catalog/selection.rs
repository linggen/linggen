//! Which model a family means: the one the list approves. No probing,
//! mutation, or ambient clock.
//!
//! The owner's list names one exact wire id per family (and per exact
//! entry). That id is the only target: there is no ranking by release date,
//! no tie, no runner-up. A newer sibling the list does not approve never
//! wins; an approved id the provider does not list for this key holds the
//! row where it is and never picks another. A deliberate rollback is a new
//! list revision that approves the older id.

use super::{ModelFacts, Stability};
use crate::config::ModelConfig;
use crate::provider::models::curated::Bundle;
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
    /// The Linggen account: the relay serves every model the list names, so
    /// there is no key and no provider list to check against.
    Account,
    NoKey,
    Unchecked,
    Failed,
    Listed(&'a AccessEvidence<'a>),
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum Hold {
    /// The facts say the approved model is not a plain chat model.
    NoChatLane,
    /// The provider's list for this key does not hold the approved model.
    NoAccess,
    /// The list is too old to move a row by itself.
    Stale,
    /// A version the list does not approve: a person may pick it, nothing follows it.
    NotApproved,
}

impl Hold {
    /// The word a settings answer carries for why a model is held back.
    pub(crate) fn code(self) -> &'static str {
        match self {
            Hold::NoChatLane => "not_chat",
            Hold::NoAccess => "no_access",
            Hold::Stale => "stale_catalog",
            Hold::NotApproved => "not_approved",
        }
    }
}

/// The approved model of a family: its exact wire id, and what the list
/// knows of it. Facts may be missing (a missing fact is not a missing row).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Target<'a> {
    pub id: &'a str,
    pub facts: Option<&'a ModelFacts>,
}

impl Target<'_> {
    pub(crate) fn name(&self) -> &str { self.facts.map_or(self.id, ModelFacts::name) }
    /// Whether a plain chat conversation can run on it: unknown facts do not say no.
    pub(crate) fn is_chat(&self) -> bool { self.facts.is_none_or(ModelFacts::text_chat) }
    pub(crate) fn released(&self) -> Option<&str> { self.facts.and_then(ModelFacts::release_date) }
}

#[derive(Debug)]
pub(crate) struct Choice<'a> {
    pub facts: &'a ModelFacts,
    pub held: Option<Hold>,
}

#[derive(Debug)]
pub(crate) enum FamilyCurrent<'a> {
    Current { current: Option<&'a str>, next: Target<'a> },
    Choose { reason: Hold, choices: Vec<Choice<'a>> },
    Checking { next: Target<'a> },
    ListFailed,
    NoKey,
    Unavailable,
}

/// Answer for one family at one listed address. `current` is the existing
/// exact ID, not the endpoint model used to describe a connection.
pub(crate) fn family_current<'a>(
    list: &'a Bundle,
    connection: Connection<'_>,
    family: &str,
    current: Option<&'a str>,
    access: Access<'_>,
    today: NaiveDate,
) -> FamilyCurrent<'a> {
    let Some(place) = list.place_of(connection.endpoint) else {
        return FamilyCurrent::Unavailable;
    };
    let Some(entry) = place.entry_for(family) else {
        return FamilyCurrent::Unavailable;
    };
    let next = Target { id: &entry.model, facts: list.facts_for(place.endpoint_key, &entry.model) };
    let account = matches!(access, Access::Account);
    let usable = match access {
        Access::Listed(e) if e.connection.credential_revision == connection.credential_revision
            && list.place_of(e.connection.endpoint).is_some_and(|p| p.endpoint_key == place.endpoint_key && p.is_at(&place.base)) => Some(e.usable),
        _ => None,
    };
    let choose = |reason| FamilyCurrent::Choose { reason, choices: choices(list, place.endpoint_key, family, next.id, reason) };
    // The owner approved this id, so its stability, date and release state do
    // not matter; only a fact that says it is not a chat model, or a provider
    // that does not list it for this key, holds it.
    if !next.is_chat() {
        return choose(Hold::NoChatLane);
    }
    if account {
        return FamilyCurrent::Current { current, next };
    }
    if usable.is_some_and(|ids| !ids.contains(next.id)) {
        return choose(Hold::NoAccess);
    }
    if usable.is_none() {
        return match access {
            Access::NoKey | Access::Account => FamilyCurrent::NoKey,
            Access::Failed => FamilyCurrent::ListFailed,
            Access::Unchecked | Access::Listed(_) => FamilyCurrent::Checking { next },
        };
    }
    if current.is_some_and(|id| id != next.id) && list.is_stale(today) {
        return choose(Hold::Stale);
    }
    FamilyCurrent::Current { current, next }
}

/// The versions of a family a person may pick by hand: the facts' members of
/// the family at this address, the approved one first. The approved one
/// carries `reason` when it is held; every other one is simply not approved.
fn choices<'a>(list: &'a Bundle, endpoint_key: &str, family: &str, approved: &str, reason: Hold) -> Vec<Choice<'a>> {
    let mut members: Vec<_> = list.facts_at(endpoint_key)
        .filter(|m| m.family() == Some(family) && m.stability != Stability::Alias)
        .collect();
    members.sort_by(|a, b| (a.id() != approved).cmp(&(b.id() != approved))
        .then_with(|| b.release_date().cmp(&a.release_date())).then_with(|| a.id().cmp(b.id())));
    members.into_iter().map(|facts| Choice {
        facts,
        held: Some(if facts.id() == approved { reason } else { Hold::NotApproved }),
    }).collect()
}

#[cfg(test)]
mod tests;
