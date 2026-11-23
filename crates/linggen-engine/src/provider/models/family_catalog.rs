//! The sole public-catalog reader. The endpoint in a file is descriptive;
//! only an engine preset can bind its data to a connection.

use crate::config::{providers::{Preset, Way, PRESETS}, ModelConfig, ProviderKind, SignIn};
use anyhow::{ensure, Result};
use chrono::{Days, NaiveDate};
use serde::Deserialize;
use std::{collections::{BTreeMap, BTreeSet}, sync::OnceLock};

mod refresh;
// The family binding writer is a later slice.
#[allow(dead_code)]
mod selection;
#[cfg(test)]
mod tests;
pub(crate) use refresh::start;

/// Capture once at a decision boundary. Never performs I/O.
pub(crate) fn current() -> std::sync::Arc<Snapshot> {
    refresh::current().unwrap_or_else(|| seed().clone())
}

const SHIPPED: &[u8] = include_bytes!("model-catalog-v1.json");
/// The served catalog is regenerated weekly, so three missed weeks is the
/// point where its facts are no longer trusted to move an existing row.
pub(crate) const AUTOMATIC_MOVE_TRUST_DAYS: u64 = 21;
const MAX_BYTES: usize = 512 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelFacts {
    id: String,
    family: Option<String>,
    name: String,
    release_date: Option<String>,
    status: Option<String>,
    stability: Stability,
    task: Option<String>,
    modalities: Modalities,
    context: Option<u32>,
    output_limit: Option<u32>,
    tool_call: Option<bool>,
    reasoning_options: Option<Vec<ReasoningOption>>,
    cost: Option<Cost>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stability { Stable, Alias, Preview, Experimental, Deprecated, Unknown }

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Modalities { input: Vec<String>, output: Vec<String> }

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ReasoningOption {
    Toggle,
    Effort { values: Vec<String> },
    BudgetTokens { min: Option<u32>, max: Option<u32> },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rates {
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
}

// Flattened rate fields in the wire layout, but still checked for unknown fields.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tier {
    over: u32,
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
}

impl Tier {
    fn rates(&self) -> Rates {
        Rates { input: self.input, output: self.output, cache_read: self.cache_read, cache_write: self.cache_write }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Cost {
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
    as_of: Option<String>,
    #[serde(default)]
    tiers: Vec<Tier>,
    #[serde(default)]
    unestimated: Vec<String>,
}

impl Cost {
    fn base(&self) -> Rates {
        Rates { input: self.input, output: self.output, cache_read: self.cache_read, cache_write: self.cache_write }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    schema: u32,
    source: String,
    observed_at: String,
    providers: Vec<Provider>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Provider {
    source_id: String,
    name: String,
    api: Option<String>,
    models: Vec<ModelFacts>,
}

pub(crate) struct Snapshot {
    observed: NaiveDate,
    providers: BTreeMap<String, BTreeMap<String, ModelFacts>>,
}

impl Snapshot {
    pub(crate) fn age_days(&self, today: NaiveDate) -> u64 {
        today.signed_duration_since(self.observed).num_days().max(0) as u64
    }

    pub(crate) fn is_stale(&self, today: NaiveDate) -> bool { self.age_days(today) > AUTOMATIC_MOVE_TRUST_DAYS }

    pub(crate) fn exact(&self, cfg: &ModelConfig) -> Option<&ModelFacts> {
        let binding = preset_for(cfg)?.catalog?;
        self.providers.get(binding.source_id)?.get(&cfg.model)
    }

    fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_BYTES, "oversized catalog");
        let file: File = serde_json::from_slice(bytes)?;
        ensure!(file.schema == 1 && file.source == "models.dev", "unknown catalog schema or source");
        let observed = date(&file.observed_at).ok_or_else(|| anyhow::anyhow!("invalid observation date"))?;
        let mut providers = BTreeMap::new();
        for p in file.providers {
            ensure!(word(&p.source_id) && word(&p.name) && !p.models.is_empty(), "invalid provider");
            ensure!(p.api.as_ref().is_none_or(|api| endpoint(api).is_some()), "invalid descriptive API");
            let mut models = BTreeMap::new();
            for m in p.models {
                m.validate(observed)?;
                ensure!(models.insert(m.id.clone(), m).is_none(), "duplicate model id");
            }
            ensure!(providers.insert(p.source_id, models).is_none(), "duplicate provider id");
        }
        Ok(Self { observed, providers })
    }

    fn validate_presets(&self) -> Result<()> {
        let mut keys = BTreeSet::new();
        let mut sources = BTreeSet::new();
        for preset in PRESETS {
            ensure!(keys.insert(preset.key), "duplicate preset key");
            let Some(binding) = preset.catalog else { continue };
            ensure!(preset.way == Way::Key && endpoint(preset.url).is_some(), "invalid catalog preset");
            sources.insert(binding.source_id);
            let rows = self.providers.get(binding.source_id)
                .ok_or_else(|| anyhow::anyhow!("missing provider {}", binding.source_id))?;
            ensure!(binding.starter_families.iter().all(|key| rows.values().any(|m| m.family.as_deref() == Some(key))), "missing starter family");
        }
        ensure!(sources.into_iter().eq(self.providers.keys().map(String::as_str)), "unbound public provider");
        Ok(())
    }

    fn retains(&self, baseline: &Self) -> bool {
        self.observed >= baseline.observed && baseline.providers.iter().all(|(id, was)| {
            self.providers.get(id).is_some_and(|now| {
                let retained = was.keys().filter(|key| now.contains_key(*key)).count();
                retained >= (was.len() * 4).div_ceil(5)
            })
        })
    }
}

pub(crate) fn shipped() -> &'static Snapshot { seed() }

fn seed() -> &'static std::sync::Arc<Snapshot> {
    static SEED: OnceLock<std::sync::Arc<Snapshot>> = OnceLock::new();
    SEED.get_or_init(|| {
        let seed = Snapshot::parse(SHIPPED).expect("shipped public model catalog");
        seed.validate_presets().expect("shipped catalog covers the presets");
        std::sync::Arc::new(seed)
    })
}

fn word(s: &str) -> bool { !s.trim().is_empty() }
fn words(xs: &[String]) -> bool {
    !xs.is_empty() && xs.iter().all(|s| word(s)) && xs.iter().collect::<BTreeSet<_>>().len() == xs.len()
}

fn date(s: &str) -> Option<NaiveDate> {
    let day = s.parse::<NaiveDate>().ok()?;
    (s.len() == 10 && !s.starts_with("0000") && day.to_string() == s).then_some(day)
}

fn release_date(s: &str) -> bool {
    date(s).is_some() || (s.len() == 7 && date(&format!("{s}-01")).is_some())
}

fn valid_rates(rates: Rates) -> bool {
    [rates.input, rates.output, rates.cache_read, rates.cache_write].into_iter()
        .flatten().all(|n| n.is_finite() && n >= 0.0)
}

impl ModelFacts {
    pub(crate) fn id(&self) -> &str { &self.id }

    pub(crate) fn text_chat(&self) -> bool {
        self.modalities.input.iter().any(|m| m == "text")
            && self.modalities.output.len() == 1 && self.modalities.output[0] == "text"
    }

    fn validate(&self, observed: NaiveDate) -> Result<()> {
        ensure!(word(&self.id) && word(&self.name), "empty model id or label");
        ensure!([&self.family, &self.status, &self.task].into_iter().flatten().all(|s| word(s)), "empty optional model field");
        ensure!(self.release_date.as_deref().is_none_or(release_date), "invalid release date");
        ensure!(words(&self.modalities.input) && words(&self.modalities.output), "invalid modalities");
        ensure!(self.context != Some(0) && self.output_limit != Some(0), "zero limits");
        if let Some(options) = &self.reasoning_options {
            ensure!(options.iter().all(|option| match option {
                ReasoningOption::Toggle => true,
                ReasoningOption::Effort { values } => words(values),
                ReasoningOption::BudgetTokens { min, max } => min.zip(*max).is_none_or(|(a, b)| a <= b),
            }), "invalid reasoning options");
        }
        // These vetoes also protect consumers if a downloaded file labels an
        // odd entry as stable. A deprecated experiment is still an experiment.
        if self.stability == Stability::Stable || self.stability == Stability::Deprecated {
            let odd = self.id.split(|c: char| !c.is_ascii_alphanumeric()).any(|part| {
                ["latest", "preview", "exp", "experimental", "alpha", "beta", "rc"].iter().any(|m| part.eq_ignore_ascii_case(m))
            });
            ensure!(!odd && match self.stability {
                Stability::Stable => matches!(self.status.as_deref(), None | Some("stable")),
                Stability::Deprecated => self.status.as_deref() == Some("deprecated"),
                _ => unreachable!(),
            }, "inconsistent stability");
        }
        if let Some(cost) = &self.cost {
            ensure!(cost.as_of.as_deref().is_none_or(|s| date(s).is_some_and(|d| d <= observed)), "invalid price date");
            ensure!(valid_rates(cost.base()) && cost.tiers.iter().all(|t| t.over > 0 && valid_rates(t.rates())), "invalid rates or threshold");
            ensure!(cost.tiers.windows(2).all(|pair| pair[0].over < pair[1].over), "unordered or duplicate context threshold");
            ensure!(cost.unestimated.is_empty() || words(&cost.unestimated), "invalid unsupported rates");
        }
        Ok(())
    }
}

fn endpoint(s: &str) -> Option<(url::Origin, String)> {
    let u = url::Url::parse(s.trim()).ok()?;
    if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() || u.query().is_some() || u.fragment().is_some() {
        return None;
    }
    Some((u.origin(), u.path().trim_end_matches('/').to_string()))
}

pub(super) fn preset_for(cfg: &ModelConfig) -> Option<&'static Preset> {
    if cfg.sign_in() != SignIn::Key || !matches!(cfg.provider_kind(), ProviderKind::Anthropic | ProviderKind::OpenAiCompatible) {
        return None;
    }
    // Anthropic's native client supplies this base for an omitted URL.
    if cfg.url.trim().is_empty() && cfg.provider_kind() == ProviderKind::Anthropic {
        return PRESETS.iter().find(|p| p.key == "anthropic");
    }
    let address = endpoint(&cfg.url)?;
    PRESETS.iter().filter(|p| p.way == Way::Key).find(|p| {
        // Switching native Anthropic and compatible adapters is not a binding.
        (p.key == "anthropic") == (cfg.provider_kind() == ProviderKind::Anthropic)
            && endpoint(p.url).as_ref() == Some(&address)
    })
}

fn accept(bytes: &[u8], baseline: &Snapshot, today: NaiveDate) -> Result<Snapshot> {
    let next = Snapshot::parse(bytes)?;
    next.validate_presets()?;
    ensure!(next.observed <= today.checked_add_days(Days::new(1)).unwrap_or(today), "future snapshot");
    ensure!(next.retains(shipped()) && next.retains(baseline), "older snapshot or abnormal row loss");
    Ok(next)
}
