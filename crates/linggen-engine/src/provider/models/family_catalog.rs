//! What the list says about a model: the facts types and the one reading of
//! them, and the one family selector (`selection`). The facts themselves ride
//! in the curated list (`curated`), joined by exact address and wire id.

use anyhow::{ensure, Result};
use chrono::NaiveDate;
use serde::Deserialize;
use std::collections::BTreeSet;

mod selection;
pub(crate) use selection::{family_current, Access, AccessEvidence, Connection, FamilyCurrent, Target};

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
    pub(crate) fn base(&self) -> Rates {
        Rates { input: self.input, output: self.output, cache_read: self.cache_read, cache_write: self.cache_write }
    }

    /// Context tiers, lowest threshold first: a call whose prompt is over
    /// the threshold is priced at that tier's rates, each on its own.
    pub(crate) fn tiers(&self) -> impl Iterator<Item = (u32, Rates)> + '_ {
        self.tiers.iter().map(|t| (t.over, t.rates()))
    }

    /// The date a supplied price was true, when the file says so.
    pub(crate) fn as_of(&self) -> Option<NaiveDate> { self.as_of.as_deref().and_then(date) }

    /// Whether the file names a rate (audio, reasoning, an odd tier) this
    /// engine cannot estimate: a dollar figure would then be too low.
    pub(crate) fn has_unestimated_rates(&self) -> bool { !self.unestimated.is_empty() }
}

pub(super) fn word(s: &str) -> bool { !s.trim().is_empty() }
fn words(xs: &[String]) -> bool {
    !xs.is_empty() && xs.iter().all(|s| word(s)) && xs.iter().collect::<BTreeSet<_>>().len() == xs.len()
}

pub(super) fn date(s: &str) -> Option<NaiveDate> {
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
    pub(crate) fn name(&self) -> &str { &self.name }

    pub(crate) fn effort_values(&self) -> impl Iterator<Item = &str> {
        self.reasoning_options.iter().flatten().filter_map(|option| match option {
            ReasoningOption::Effort { values } => Some(values),
            _ => None,
        }).flatten().map(String::as_str)
    }
    pub(crate) fn family(&self) -> Option<&str> { self.family.as_deref() }
    pub(crate) fn release_date(&self) -> Option<&str> { self.release_date.as_deref() }
    pub(crate) fn context(&self) -> Option<u32> { self.context }
    pub(crate) fn tool_call(&self) -> Option<bool> { self.tool_call }

    /// Whether a plain text conversation runs on this entry: text in, text
    /// only out (image or audio in is fine), and no other declared task. An
    /// embedding entry also says "text out", so the task decides first. This
    /// is the one such fact: `kinds` and the family selector both ask it.
    pub(crate) fn text_chat(&self) -> bool {
        matches!(self.task.as_deref(), None | Some("chat"))
            && self.modalities.input.iter().any(|m| m == "text")
            && self.modalities.output.len() == 1 && self.modalities.output[0] == "text"
    }

    pub(crate) fn cost(&self) -> Option<&Cost> { self.cost.as_ref() }

    /// Whether a person may pick this exact version by hand: a dated, stable,
    /// plain chat model. (Moving a row by itself asks more: `family_current`.)
    pub(crate) fn chooseable(&self) -> bool {
        self.stability == Stability::Stable && self.text_chat() && self.release_date.as_deref().and_then(date).is_some()
    }

    /// Also the curated list's check of the facts it carries: one rule for both readers.
    pub(super) fn validate(&self, observed: NaiveDate) -> Result<()> {
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

pub(super) fn endpoint(s: &str) -> Option<(url::Origin, String)> {
    let u = url::Url::parse(s.trim()).ok()?;
    if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() || u.query().is_some() || u.fragment().is_some() {
        return None;
    }
    Some((u.origin(), u.path().trim_end_matches('/').to_string()))
}
