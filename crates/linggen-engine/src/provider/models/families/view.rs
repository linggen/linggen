//! What the settings list says about an endpoint's families: facts, never
//! words. Built from the config, the catalog in force and the provider lists
//! already held; it asks no provider and fails no list.

use super::{binding, Endpoint, Families};
use super::super::curated::Bundle;
use super::super::family_catalog::{FamilyCurrent, Target};
use super::super::pricing::{price_in, Price, PriceSource};
use crate::config::{Config, ModelConfig};
use crate::credentials::Credentials;
use serde::Serialize;

/// How many families a page shows before 「更多」.
const SHOWN: usize = 4;

#[derive(Serialize)]
pub struct FamiliesView {
    catalog: CatalogView,
    #[serde(skip_serializing_if = "Option::is_none")]
    access: Option<&'static str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    rates_unavailable: bool,
    families: Vec<FamilyView>,
}

#[derive(Serialize)]
struct CatalogView {
    observed_at: String,
    age_days: u64,
    stale: bool,
}

#[derive(Serialize)]
struct FamilyView {
    family: String,
    /// The short name a page shows beside the checkbox: the list's label.
    label: String,
    ticked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    row: Option<String>,
    /// "follow" or "pinned", for a ticked family.
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<&'static str>,
    /// What the row runs now.
    #[serde(skip_serializing_if = "Option::is_none")]
    current: Option<ModelLine>,
    /// What the family means now and could be run on: a tick's target, or the
    /// newer model a following row has not moved to yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    next: Option<ModelLine>,
    /// The owner has not checked this entry in the offered edition's region.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    unverified: bool,
    /// Why the family cannot be moved or ticked by itself (a `Hold` code).
    #[serde(skip_serializing_if = "Option::is_none")]
    hold: Option<&'static str>,
    choose: Vec<ChoiceView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notice: Option<NoticeView>,
    /// Past the first few, unticked and without a notice: under 「更多」.
    folded: bool,
    references: Vec<Reference>,
}

#[derive(Serialize)]
struct ModelLine {
    model: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    released: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    price: Option<PriceLine>,
}

/// USD per million tokens at one date (pricing.rs decides both).
#[derive(Serialize)]
pub(in crate::provider::models) struct PriceLine {
    #[serde(flatten)]
    rates: Price,
    #[serde(flatten)]
    source: PriceSource,
}

impl PriceLine {
    /// The line the page shows for a priced model.
    pub(in crate::provider::models) fn of(p: super::super::pricing::Priced) -> Self {
        Self { rates: p.base_rates(), source: p.source }
    }
}

#[derive(Serialize)]
struct ChoiceView {
    model: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    released: Option<String>,
    /// Why it is held back from automatic choice; absent when it is not.
    #[serde(skip_serializing_if = "Option::is_none")]
    held: Option<&'static str>,
    /// A person may pick it.
    chooseable: bool,
}

#[derive(Serialize)]
struct NoticeView {
    /// The number the actions name.
    #[serde(rename = "move")]
    moved: u64,
    kind: &'static str,
    from: String,
    to: String,
    on: String,
    can_go_back: bool,
}

/// Where a row's id is used in the config.
#[derive(Serialize)]
struct Reference {
    place: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    who: Option<String>,
}

pub(super) fn of(f: &Families, config: &Config, creds: &Credentials, (name, url): (&str, &str)) -> Option<FamiliesView> {
    let list = f.catalog();
    let endpoint = Endpoint::of(&list, config.model_catalog.offered(), name, url, creds)?;
    // A provider the list approves nothing at (Groq, OpenRouter) has no families to show.
    endpoint.place.tickable().next()?;
    let today = f.today();
    let result = endpoint.read_result(f);
    let listing = result.as_ref().and_then(|entry| entry.as_deref());
    // No facts at this address: no rate is shown and none is borrowed from another.
    let rates_unavailable = list.facts_at(endpoint.place.endpoint_key).next().is_none();
    let access = if rates_unavailable && endpoint.key.is_some() {
        match &result { None => Some("checking"), Some(None) => Some("list_failed"), _ => None }
    } else { None };
    let mut families = Vec::new();
    for (family, entry) in endpoint.place.tickable() {
        let row = binding::row_of(&list, config, endpoint.place.endpoint_key, family);
        let current = row.map(|r| r.model.as_str());
        let answer = if matches!(result, Some(None)) {
            endpoint.select(&list, family, current, super::Access::Failed, today)
        } else {
            endpoint.ask(&list, family, current, listing, today)
        };
        let (next, hold, choose) = match &answer {
            FamilyCurrent::Current { next, current } => {
                let moves = current.is_none_or(|c| c != next.id);
                (moves.then(|| line(&endpoint, &list, *next)), None, Vec::new())
            }
            FamilyCurrent::Choose { reason, choices, .. } => (None, Some(reason.code()), choices.iter().map(|c| ChoiceView {
                model: c.facts.id().into(),
                name: c.facts.name().into(),
                released: c.facts.release_date().map(str::to_string),
                held: c.held.map(|h| h.code()),
                chooseable: c.facts.chooseable(),
            }).collect()),
            FamilyCurrent::Checking { next } => (Some(line(&endpoint, &list, *next)), Some("checking"), Vec::new()),
            FamilyCurrent::ListFailed => (None, Some("list_failed"), Vec::new()),
            FamilyCurrent::NoKey => (None, Some("no_key"), Vec::new()),
            FamilyCurrent::Unavailable => (None, Some("unavailable"), Vec::new()),
        };
        let held = row.and_then(|r| r.family_binding.as_ref());
        let notice = held.and_then(|b| b.notice()).map(|m| NoticeView {
            moved: m.seq(),
            kind: match m.kind() {
                binding::MoveKind::Followed => "followed",
                binding::MoveKind::WentBack => "went_back",
            },
            from: m.from().into(),
            to: m.to().into(),
            on: m.on().into(),
            can_go_back: m.kind() == binding::MoveKind::Followed
                && Target { id: m.from(), facts: list.facts_for(endpoint.place.endpoint_key, m.from()) }.is_chat()
                && listing.as_ref().is_some_and(|l| l.usable.contains(m.from())),
        });
        let folded = families.len() >= SHOWN && row.is_none() && notice.is_none();
        families.push(FamilyView {
            ticked: row.is_some(),
            row: row.map(|r| r.id.clone()),
            mode: held.map(|b| if b.follows() { "follow" } else { "pinned" }),
            current: row.map(|r| line(&endpoint, &list, Target { id: &r.model, facts: list.exact(r) })),
            next,
            hold,
            choose,
            folded,
            notice,
            references: row.map(|r| references(config, &r.id)).unwrap_or_default(),
            label: entry.label.clone(),
            unverified: entry.unverified,
            family: family.to_string(),
        });
    }
    Some(FamiliesView {
        access,
        rates_unavailable,
        catalog: CatalogView {
            observed_at: list.facts_observed_on().to_string(),
            age_days: list.age_days(today),
            stale: list.is_stale(today),
        },
        families,
    })
}

/// A model of the endpoint as a line: its name, date and dated price.
fn line(endpoint: &Endpoint, list: &Bundle, target: Target<'_>) -> ModelLine {
    let mut row: ModelConfig = endpoint.probe.clone();
    row.model = target.id.into();
    ModelLine {
        model: target.id.into(),
        name: target.name().into(),
        released: target.released().map(str::to_string),
        price: price_in(&row, list).map(PriceLine::of),
    }
}

/// Every place in the config that names a row by id, so a person can be told
/// before it is removed. (Sessions, missions, skills and rooms name models
/// in their own stores and are not read here.)
fn references(config: &Config, id: &str) -> Vec<Reference> {
    let chain = config.routing.default_models.iter().enumerate().filter(|(_, m)| *m == id)
        .map(|(i, _)| Reference { place: if i == 0 { "default_chat" } else { "fallback" }, who: None });
    let pet = (config.pet.model == id).then(|| Reference { place: "pet", who: None });
    let agents = config.agents.iter().filter(|a| a.model.as_deref() == Some(id))
        .map(|a| Reference { place: "agent", who: Some(a.id.clone()) });
    chain.chain(pet).chain(agents).collect()
}
