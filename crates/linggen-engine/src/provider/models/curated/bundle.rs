//! Checking and parsing one signed bundle. The wire rules repeat the site's
//! list builder (`scripts/` in the site repository); the shared fixture holds the two together.

use super::super::family_catalog::{date, endpoint, word, ModelFacts};
use crate::config::providers::{known_names, Way};
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
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FamilyMatcher { pub source_id: String, pub name: String }

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
    #[serde(default)]
    pub unverified: bool,
    pub note: Option<String>,
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

    pub(crate) fn endpoint(&self, key: &str) -> Option<&Endpoint> { self.endpoints.get(key) }

    /// The facts for one wire id at one list endpoint: joined by the endpoint's
    /// normalized address and the exact id, never by name or by another address.
    pub(crate) fn facts_for(&self, endpoint_key: &str, model: &str) -> Option<&ModelFacts> {
        let address = endpoint(self.endpoints.get(endpoint_key)?.base.as_deref()?)?;
        let record = self.facts.iter().find(|f| endpoint(&f.address).as_ref() == Some(&address))?;
        record.models.iter().find(|m| m.id() == model)
    }

    /// The same bytes or a newer revision: never an older one.
    fn is_not_older_than(&self, floor: &Bundle) -> bool {
        self.revision > floor.revision || (self.revision == floor.revision && self.raw == floor.raw)
    }
}

/// The one entry for a bundle's bytes: size, signature, shape, rules.
/// A bundle that fails any step is refused as a whole.
pub(super) fn decode(bytes: &[u8], signature: &str, key: &[u8; 32]) -> Result<Bundle> {
    ensure!(bytes.len() <= MAX_BYTES, "oversized list");
    let signature = base64::engine::general_purpose::STANDARD.decode(signature.trim())
        .map_err(|_| anyhow!("unreadable signature"))?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(bytes, &signature).map_err(|_| anyhow!("signature does not match the list"))?;
    let mut bundle: Bundle = serde_json::from_slice(bytes)?;
    bundle.raw = bytes.to_vec();
    bundle.observed = date(&bundle.facts_observed_at).ok_or_else(|| anyhow!("invalid facts date"))?;
    bundle.check()?;
    Ok(bundle)
}

/// [`decode`], and not older than the floors (the shipped copy, the last good one).
pub(super) fn accept(bytes: &[u8], signature: &str, key: &[u8; 32], floors: &[&Bundle], today: NaiveDate) -> Result<Bundle> {
    let next = decode(bytes, signature, key)?;
    ensure!(next.observed <= today.checked_add_days(Days::new(1)).unwrap_or(today), "future facts date");
    ensure!(floors.iter().all(|floor| next.is_not_older_than(floor)), "older or conflicting list revision");
    Ok(next)
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
