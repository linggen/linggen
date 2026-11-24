//! Checking and parsing one signed bundle. The wire rules repeat the site's
//! list builder (`scripts/` in the site repository); the shared fixture holds the two together.

use super::super::family_catalog::{date, endpoint, word, ModelFacts};
use crate::config::providers::{known_names, Way};
use crate::config::{ModelConfig, ProviderKind, SignIn};
use crate::credentials::same_endpoint;
use anyhow::{anyhow, ensure, Result};
use base64::Engine as _;
use chrono::{Days, NaiveDate};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const MAX_BYTES: usize = 1024 * 1024;

/// The two lists a bundle carries. A closed set: a new edition is code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Edition { International, Mainland }

/// Named connections the engine owns: the list says which, never where.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Binding { Chatgpt, LinggenCloud, OllamaLocal }

impl Binding {
    /// The address an engine-owned connection speaks to, when it has a fixed
    /// one (Linggen Cloud's moves with the account).
    pub(crate) fn address(self) -> Option<&'static str> {
        match self {
            Binding::Chatgpt => Some(crate::provider::codex_auth::CHATGPT_API_BASE),
            Binding::OllamaLocal => Some("http://127.0.0.1:11434"),
            Binding::LinggenCloud => None,
        }
    }
}

/// The wires the engine implements. The list picks one; it never supplies a request shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Adapter { Anthropic, OpenaiCompatible, Chatgpt, LinggenCloud, Ollama }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EntryType { Family, Exact, Live }

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Endpoint {
    pub base: Option<String>,
    pub binding: Option<Binding>,
    /// For the owner and the proposal job; no page shows it yet.
    #[allow(dead_code)]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FamilyMatcher {
    /// Where the weekly proposal job looks for newer versions; never read by the engine.
    #[allow(dead_code)]
    pub source_id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LiveService { pub service: String, pub used_by: Vec<String> }

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub key: String,
    pub label: String,
    pub endpoint: String,
    #[serde(rename = "type")]
    pub kind: EntryType,
    /// The exact wire model id: the only version this entry ever means.
    pub model: String,
    pub family: Option<FamilyMatcher>,
    pub live: Option<LiveService>,
    pub starter: bool,
    /// Slice 3 shows these two on the page.
    #[serde(default)]
    #[allow(dead_code)]
    pub unverified: bool,
    #[allow(dead_code)]
    pub note: Option<String>,
}

impl Entry {
    /// What a ticked row calls this entry in its binding: the family the list
    /// names for a family entry, the entry's own key for an exact one. A live
    /// entry, or a family with no matcher (a sign-in built-in), has none.
    pub(crate) fn family_key(&self) -> Option<&str> {
        match self.kind {
            EntryType::Family => self.family.as_ref().map(|f| f.name.as_str()),
            EntryType::Exact => Some(self.key.as_str()),
            EntryType::Live => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Provider {
    pub key: String,
    pub label: String,
    /// The provider name a model row is filed under (a name the config knows).
    pub name: String,
    pub sign_in: Way,
    pub adapter: Adapter,
    pub default_endpoint: String,
    #[serde(default)]
    pub endpoints: Vec<String>,
    #[allow(dead_code)]
    pub note: Option<String>,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FactsRecord {
    address: String,
    source_id: String,
    models: Vec<ModelFacts>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bundle {
    schema: u32,
    revision: u32,
    list_revision: u32,
    facts_observed_at: String,
    endpoints: BTreeMap<String, Endpoint>,
    international: Vec<Provider>,
    mainland: Vec<Provider>,
    facts: Vec<FactsRecord>,
    #[serde(skip)]
    observed: NaiveDate,
    /// The exact bytes this value was read from.
    #[serde(skip)]
    raw: Vec<u8>,
}

impl Bundle {
    pub(crate) fn revision(&self) -> u32 { self.revision }
    pub(crate) fn list_revision(&self) -> u32 { self.list_revision }
    pub(crate) fn facts_observed_on(&self) -> NaiveDate { self.observed }
    pub(crate) fn raw(&self) -> &[u8] { &self.raw }

    pub(crate) fn providers(&self, edition: Edition) -> &[Provider] {
        match edition {
            Edition::International => &self.international,
            Edition::Mainland => &self.mainland,
        }
    }


    /// How old the facts are: a price's date when it names none, and the
    /// measure of whether the list may still move an existing row.
    pub(crate) fn age_days(&self, today: NaiveDate) -> u64 {
        today.signed_duration_since(self.observed).num_days().max(0) as u64
    }

    /// An unchecked, stale list can be shown but cannot move a row.
    pub(crate) fn is_stale(&self, today: NaiveDate) -> bool { self.age_days(today) > AUTOMATIC_MOVE_TRUST_DAYS }

    /// Every connection the list offers an edition, in the list's order.
    pub(crate) fn offered(&self, edition: Edition) -> impl Iterator<Item = Place<'_>> {
        self.providers(edition).iter().flat_map(|p| p.places(self))
    }

    /// The connection at `url`, whatever name a row there is filed under:
    /// the exact address, in either edition (a row kept from the other
    /// edition still has its label, facts and price). The offered edition wins.
    pub(crate) fn place_at(&self, url: &str) -> Option<Place<'_>> {
        [Edition::International, Edition::Mainland].into_iter()
            .flat_map(|edition| self.offered(edition))
            .find(|place| place.is_at(url))
    }

    /// The key connection a row speaks to: its sign-in is a key, its wire is
    /// the one the list names for that address (Anthropic's native client or
    /// a compatible one), and its address is exactly the list's.
    pub(crate) fn place_of(&self, row: &ModelConfig) -> Option<Place<'_>> {
        if row.sign_in() != SignIn::Key || !matches!(row.provider_kind(), ProviderKind::Anthropic | ProviderKind::OpenAiCompatible) {
            return None;
        }
        let native = row.provider_kind() == ProviderKind::Anthropic;
        // Anthropic's native client supplies its own base for an omitted address:
        // only a place AT that base is the row's. A list that moves the base
        // never lends the row's key to the new address.
        let at = |place: &Place<'_>| {
            let url = if row.url.trim().is_empty() && native { crate::provider::claude_auth::ANTHROPIC_API_BASE } else { row.url.as_str() };
            place.is_at(url)
        };
        [Edition::International, Edition::Mainland].into_iter()
            .flat_map(|edition| self.offered(edition))
            .find(|place| place.provider.sign_in == Way::Key && (place.provider.adapter == Adapter::Anthropic) == native && at(place))
    }

    /// A provider's label: found by its ADDRESS first, so rows filed under a
    /// generic name at a listed address show that provider's label; then by
    /// the name alone (the first provider of that name); else the name as written.
    pub(crate) fn label_for(&self, name: &str, url: &str) -> String {
        if let Some(place) = self.place_at(url) {
            return place.provider.label.clone();
        }
        [Edition::International, Edition::Mainland].into_iter()
            .flat_map(|edition| self.providers(edition))
            .find(|p| p.name.eq_ignore_ascii_case(name.trim()))
            .map_or_else(|| name.trim().to_string(), |p| p.label.clone())
    }

    /// What the list says about a row's own model, at the row's own address.
    pub(crate) fn exact(&self, row: &ModelConfig) -> Option<&ModelFacts> {
        self.facts_for(self.place_of(row)?.endpoint_key, &row.model)
    }

    /// The connection a live service is shown under (its entry's provider), if the list names one.
    pub(crate) fn live_place(&self, service: &str) -> Option<(Place<'_>, &Entry)> {
        [Edition::International, Edition::Mainland].into_iter()
            .flat_map(|edition| self.offered(edition))
            .find_map(|place| {
                let entry = place.entries().find(|e| e.live.as_ref().is_some_and(|l| l.service == service))?;
                Some((place, entry))
            })
    }

    /// The facts for one wire id at one list endpoint: joined by the endpoint's
    /// normalized address and the exact id, never by name or by another address.
    pub(crate) fn facts_for(&self, endpoint_key: &str, model: &str) -> Option<&ModelFacts> {
        self.facts_at(endpoint_key).find(|m| m.id() == model)
    }

    /// Every model the facts carry for one list endpoint, same join.
    pub(crate) fn facts_at(&self, endpoint_key: &str) -> impl Iterator<Item = &ModelFacts> {
        let address = self.endpoints.get(endpoint_key).and_then(|e| e.base.as_deref()).and_then(endpoint);
        let record = address.and_then(|address| self.facts.iter().find(|f| endpoint(&f.address).as_ref() == Some(&address)));
        record.into_iter().flat_map(|r| r.models.iter())
    }

    /// The same bytes or a newer revision: never an older one.
    fn is_not_older_than(&self, floor: &Bundle) -> bool {
        self.revision > floor.revision || (self.revision == floor.revision && self.raw == floor.raw)
    }
}

/// The one entry for a bundle's bytes: size, shape, rules. A bundle that
/// fails any step is refused as a whole. No signature is required to read:
/// the list travels over HTTPS and the owner decided against a check for now
/// (the signing code stays: [`verify`], the key in `trust`, the site's signer).
pub(super) fn decode(bytes: &[u8]) -> Result<Bundle> {
    ensure!(bytes.len() <= MAX_BYTES, "oversized list");
    let mut bundle: Bundle = serde_json::from_slice(bytes)?;
    bundle.raw = bytes.to_vec();
    bundle.observed = date(&bundle.facts_observed_at).ok_or_else(|| anyhow!("invalid facts date"))?;
    bundle.check()?;
    Ok(bundle)
}

/// Whether `signature` (Ed25519, base64) is the owner's over exactly `bytes`.
/// Not part of reading a list today; kept so a later decision to require it
/// is one call at [`decode`]'s callers.
#[allow(dead_code)]
pub(super) fn verify(bytes: &[u8], signature: &str, key: &[u8; 32]) -> Result<()> {
    let signature = base64::engine::general_purpose::STANDARD.decode(signature.trim())
        .map_err(|_| anyhow!("unreadable signature"))?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(bytes, &signature).map_err(|_| anyhow!("signature does not match the list"))
}

/// [`decode`], and not older than the floors (the shipped copy, the last good one).
pub(super) fn accept(bytes: &[u8], floors: &[&Bundle], today: NaiveDate) -> Result<Bundle> {
    let next = decode(bytes)?;
    ensure!(next.observed <= today.checked_add_days(Days::new(1)).unwrap_or(today), "future facts date");
    ensure!(floors.iter().all(|floor| next.is_not_older_than(floor)), "older or conflicting list revision");
    Ok(next)
}

/// The facts older than this no longer move an existing row by themselves.
/// The list is a weekly artifact, so three missed weeks is the point.
pub(crate) const AUTOMATIC_MOVE_TRUST_DAYS: u64 = 21;

/// One provider at one address: what a page lists as a connection card, and
/// what a row at that address is bound to. Borrowed from the list read at
/// the decision, so a decision never mixes two lists. A key is filed under
/// the place's EXACT address; a list that changes an address is a different
/// place, with no key, and the old key stays where it was.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Place<'a> {
    pub provider: &'a Provider,
    /// The key of the endpoint in the list: also what a stored binding and a
    /// starter record call this connection.
    pub endpoint_key: &'a str,
    /// The address, as written in the list (or the engine's own for a binding).
    pub base: &'a str,
}

impl<'a> Place<'a> {
    pub(crate) fn is_at(&self, url: &str) -> bool { same_endpoint("", self.base, "", url) }

    /// Whether Anthropic's own wire is spoken here (a list is asked differently).
    pub(crate) fn native_anthropic(&self) -> bool { self.provider.adapter == Adapter::Anthropic }

    /// The entries of this provider that live at this address.
    pub(crate) fn entries(&self) -> impl Iterator<Item = &'a Entry> + '_ {
        self.provider.entries.iter().filter(|e| e.endpoint == self.endpoint_key)
    }

    /// The entries a person can tick: families and exact versions with a key
    /// to follow them under. A provider the engine runs by sign-in has none.
    pub(crate) fn tickable(&self) -> impl Iterator<Item = (&'a str, &'a Entry)> + '_ {
        let keyed = self.provider.sign_in == Way::Key;
        self.entries().filter(move |_| keyed).filter_map(|e| Some((e.family_key()?, e)))
    }

    pub(crate) fn entry_for(&self, family: &str) -> Option<&'a Entry> {
        self.tickable().find(|(key, _)| *key == family).map(|(_, e)| e)
    }

    /// The live voices the list names here, as the page may offer them.
    pub(crate) fn offered_live_models(&self) -> Vec<&'static str> {
        crate::live::offered_to_pages().into_iter()
            .filter(|voice| self.entries().any(|e| e.live.as_ref().is_some_and(|l| l.service == voice.id)))
            .map(|voice| voice.model).collect()
    }
}

impl Provider {
    /// Its own addresses, each with the key it is filed under. An engine-owned
    /// connection with no fixed address has no place.
    fn places<'a>(&'a self, list: &'a Bundle) -> impl Iterator<Item = Place<'a>> + 'a {
        std::iter::once(&self.default_endpoint).chain(&self.endpoints).filter_map(move |key| {
            let endpoint = list.endpoints.get(key)?;
            let base = endpoint.base.as_deref().or_else(|| endpoint.binding.and_then(Binding::address))?;
            Some(Place { provider: self, endpoint_key: key, base })
        })
    }
}

impl Bundle {
    fn check(&self) -> Result<()> {
        ensure!(self.schema == 2 && self.revision > 0 && self.list_revision > 0, "unknown list schema or revision");
        self.check_endpoints()?;
        self.check_edition(&self.international)?;
        self.check_edition(&self.mainland)?;
        self.check_facts()
    }

    fn check_endpoints(&self) -> Result<()> {
        ensure!(!self.endpoints.is_empty(), "no endpoints");
        for (key, e) in &self.endpoints {
            ensure!(word(key) && e.base.is_some() != e.binding.is_some(), "endpoint needs one of base or binding");
            ensure!(e.base.as_deref().is_none_or(|base| endpoint(base).is_some()), "endpoint base must be an https address");
        }
        Ok(())
    }

    fn check_edition(&self, providers: &[Provider]) -> Result<()> {
        let names = known_names();
        let mut keys = BTreeSet::new();
        for p in providers {
            ensure!(word(&p.key) && word(&p.label) && keys.insert(&p.key), "provider key");
            ensure!(names.contains(&p.name.as_str()), "provider name is not one the config knows");
            let own: Vec<&String> = std::iter::once(&p.default_endpoint).chain(&p.endpoints).collect();
            ensure!(own.iter().all(|k| self.endpoints.contains_key(*k)), "provider endpoint is not in the list");
            ensure!(own.iter().collect::<BTreeSet<_>>().len() == own.len(), "provider endpoint repeated");
            let mut entry_keys = BTreeSet::new();
            for entry in &p.entries {
                ensure!(word(&entry.key) && word(&entry.label) && word(&entry.model) && entry_keys.insert(&entry.key), "entry fields");
                ensure!(own.contains(&&entry.endpoint), "entry endpoint is not one of its provider's");
                entry.check_kind()?;
                self.check_live_host(entry)?;
            }
        }
        Ok(())
    }

    fn check_facts(&self) -> Result<()> {
        let bases: Vec<_> = self.endpoints.values().filter_map(|e| e.base.as_deref()).filter_map(endpoint).collect();
        let mut addresses = Vec::new();
        for record in &self.facts {
            let address = endpoint(&record.address).ok_or_else(|| anyhow!("facts address must be https"))?;
            ensure!(bases.contains(&address) && !addresses.contains(&address), "facts address is orphaned or repeated");
            addresses.push(address);
            ensure!(word(&record.source_id) && !record.models.is_empty(), "facts source");
            let mut ids = BTreeSet::new();
            for m in &record.models {
                m.validate(self.observed)?;
                ensure!(ids.insert(m.id()), "duplicate model id in facts");
            }
        }
        Ok(())
    }
}

impl Bundle {
    /// A live entry sits at the host its service's key is filed for.
    fn check_live_host(&self, entry: &Entry) -> Result<()> {
        let Some(live) = &entry.live else { return Ok(()) };
        let host = self.endpoints.get(&entry.endpoint).and_then(|e| e.base.as_deref())
            .and_then(|base| url::Url::parse(base).ok()).and_then(|u| u.host_str().map(str::to_ascii_lowercase));
        let service = crate::live::PROVIDERS.iter().find(|p| p.id == live.service);
        ensure!(service.zip(host).is_some_and(|(p, host)| p.key_host().eq_ignore_ascii_case(&host)), "live entry is not at its service's host");
        Ok(())
    }
}

impl Entry {
    fn check_kind(&self) -> Result<()> {
        match self.kind {
            EntryType::Family => ensure!(self.live.is_none(), "a family entry has no live descriptor"),
            EntryType::Exact => ensure!(self.live.is_none() && self.family.is_none(), "an exact entry is a pin"),
            EntryType::Live => {
                let live = self.live.as_ref().ok_or_else(|| anyhow!("live entry needs its descriptor"))?;
                ensure!(!self.starter && self.family.is_none() && !live.used_by.is_empty(), "a live entry is a reference");
                // Only a voice this engine's dialect runs: the list cannot claim a newer one.
                ensure!(crate::live::PROVIDERS.iter().any(|p| p.id == live.service && p.model == self.model),
                    "live model is not one the engine declares");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
impl Bundle {
    /// The shipped list with its facts replaced by a few entries at one list
    /// address (`models` is a JSON array), for the readers of facts (`kinds`,
    /// `pricing`): small, and independent of the facts the release ships.
    pub(crate) fn for_tests(observed: &str, address: &str, models: serde_json::Value) -> Self {
        Self::variant(|value| {
            value["facts_observed_at"] = observed.into();
            value["facts"] = serde_json::json!([{ "address": address, "source_id": "test", "models": models }]);
        })
    }

    /// The shipped list with `change` applied: a list an owner could have written.
    pub(crate) fn variant(change: impl FnOnce(&mut serde_json::Value)) -> Self {
        let mut value: serde_json::Value = serde_json::from_slice(super::store::shipped().raw()).unwrap();
        change(&mut value);
        decode(&serde_json::to_vec(&value).unwrap()).expect("test list")
    }
}
