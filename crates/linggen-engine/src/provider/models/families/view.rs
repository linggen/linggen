//! What the settings list says about an endpoint's families: facts, never
//! words. Built from the config, the catalog in force and the provider lists
//! already held; it asks no provider and fails no list.

use super::{binding, Endpoint, Families};
use super::super::family_catalog::{FamilyCurrent, ModelFacts, Snapshot};
use super::super::pricing::{price_in, Price, PriceSource};
use crate::config::{Config, ModelConfig};
use crate::credentials::Credentials;
use serde::Serialize;

/// How many families a page shows before 「更多」.
const SHOWN: usize = 4;

#[derive(Serialize)]
pub struct FamiliesView {
    catalog: CatalogView,
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
struct PriceLine {
    #[serde(flatten)]
    rates: Price,
    #[serde(flatten)]
    source: PriceSource,
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
    let endpoint = Endpoint::of(name, url, creds)?;
    let snapshot = f.catalog();
    let today = f.today();
    let listing = endpoint.listing(f);
    let families = snapshot.chat_families(endpoint.preset).into_iter().enumerate().map(|(rank, (family, _))| {
        let row = binding::row_of(config, endpoint.preset.key, &family);
        let answer = endpoint.ask(&snapshot, &family, row.map(|r| r.model.as_str()), listing.as_deref(), today);
        let (next, hold, choose) = match &answer {
            FamilyCurrent::Current { next, current } => {
                let moves = current.is_none_or(|c| c != next.id());
                (moves.then(|| line(&endpoint, &snapshot, next)), None, Vec::new())
            }
            FamilyCurrent::Choose { reason, choices, .. } => (None, Some(reason.code()), choices.iter().map(|c| ChoiceView {
                model: c.facts.id().into(),
                name: c.facts.name().into(),
                released: c.facts.release_date().map(str::to_string),
                held: c.held.map(|h| h.code()),
                chooseable: c.facts.chooseable(),
            }).collect()),
            FamilyCurrent::Unavailable { .. } => (None, Some("unavailable"), Vec::new()),
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
                && snapshot.entry(endpoint.preset, m.from()).is_some_and(ModelFacts::chooseable)
                && listing.as_ref().is_some_and(|l| l.usable.contains(m.from())),
        });
        FamilyView {
            ticked: row.is_some(),
            row: row.map(|r| r.id.clone()),
            mode: held.map(|b| if b.follows() { "follow" } else { "pinned" }),
            current: row.and_then(|r| snapshot.exact(r).map(|facts| line(&endpoint, &snapshot, facts))),
            next,
            hold,
            choose,
            folded: rank >= SHOWN && row.is_none() && notice.is_none(),
            notice,
            references: row.map(|r| references(config, &r.id)).unwrap_or_default(),
            family,
        }
    }).collect();
    Some(FamiliesView {
        catalog: CatalogView {
            observed_at: snapshot.observed_on().to_string(),
            age_days: snapshot.age_days(today),
            stale: snapshot.is_stale(today),
        },
        families,
    })
}

/// A model of the endpoint as a line: its name, date and dated price.
fn line(endpoint: &Endpoint, snapshot: &Snapshot, facts: &ModelFacts) -> ModelLine {
    let mut row: ModelConfig = endpoint.probe.clone();
    row.model = facts.id().into();
    ModelLine {
        model: facts.id().into(),
        name: facts.name().into(),
        released: facts.release_date().map(str::to_string),
        price: price_in(&row, snapshot).map(|p| PriceLine { rates: p.base_rates(), source: p.source }),
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
