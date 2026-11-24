//! What a session's model calls cost — when a price is known.
//!
//! The engine records what each call used (`usage.rs`, the session ledger);
//! a dollar figure is derived here, at read time, and only for a model with
//! a price: the model's own `price` in the config, which wins, else the
//! public catalog's price for that exact model at that exact provider
//! address (`family_catalog`; dated, never copied into a table here). No
//! price, a zero or missing base rate, or a rate the engine cannot estimate:
//! no figure — the tokens are shown and nothing is guessed. A model that
//! bills an account (the ChatGPT sign-in, Linggen Cloud, a room, a Claude
//! subscription) or runs on this machine has no per-token price: it is
//! `Billing::Account` or `Billing::Local`, and a session's spend leaves it
//! out. A call's charge is fixed when it is made and kept in the ledger; no
//! later price or catalog change reaches it.

use super::curated::{self, Bundle};
use super::family_catalog::Rates;
use super::{ModelManager, TokenUsage, Tokens};
use crate::config::{ModelConfig, ModelPrice, ProviderKind, SignIn};
use crate::state_fs::session_usage::SessionUsage;
use serde::Serialize;

/// USD per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

impl Price {
    pub fn cost(&self, t: &Tokens) -> f64 {
        (t.input as f64 * self.input
            + t.output as f64 * self.output
            + t.cache_read as f64 * self.cache_read
            + t.cache_write as f64 * self.cache_write)
            / 1_000_000.0
    }
}

impl From<&ModelPrice> for Price {
    /// A configured price leaves the cache prices out when its provider has
    /// none: they cost what plain input costs.
    fn from(p: &ModelPrice) -> Price {
        Price {
            input: p.input,
            output: p.output,
            cache_read: p.cache_read.unwrap_or(p.input),
            cache_write: p.cache_write.unwrap_or(p.input),
        }
    }
}

/// A model's price: the rates for a call, and the context tiers that replace
/// them for a long prompt, each at the threshold the provider names.
#[derive(Debug, Clone, PartialEq)]
pub struct Priced {
    base: Price,
    /// Ascending by threshold: a call whose prompt is over `.0` tokens is
    /// priced at `.1`. `None`: that tier's input or output rate is missing or
    /// zero — a call in it has no figure, and no other tier's rate stands in.
    tiers: Vec<(u32, Option<Price>)>,
    pub source: PriceSource,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "source", content = "as_of", rename_all = "kebab-case")]
pub enum PriceSource {
    /// The public catalog's price, and the day it was true: the price's own
    /// date when the catalog gives one, else the day the catalog was observed.
    Catalog(String),
    Config,
}

impl Priced {
    /// The rates a call without a long prompt is priced at, per million tokens.
    pub fn base_rates(&self) -> Price {
        self.base
    }

    /// What one call cost at this price, each request on its own; `None`
    /// when the tier its prompt falls in has no usable rate.
    pub fn cost_of_call(&self, used: &Tokens) -> Option<f64> {
        let prompt = used.prompt();
        let rates = self
            .tiers
            .iter()
            .rev()
            .find(|(over, _)| prompt > u64::from(*over))
            .map_or(Some(self.base), |(_, rates)| *rates);
        rates.map(|rates| rates.cost(used))
    }
}

/// What a call is kept as in the session ledger, worked out when the call is
/// made from the model's config as it is then: its billing class, and — when
/// a price is known — what it cost. Never re-derived afterwards.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallRecord {
    /// Absent when the model was not in the config (it claims nothing).
    pub billing: Option<Billing>,
    pub priced: bool,
    pub cost_usd: Option<f64>,
    pub source: Option<String>,
    pub as_of: Option<String>,
}

impl CallRecord {
    pub fn of(config: Option<&ModelConfig>, usage: Option<&TokenUsage>) -> CallRecord {
        Self::of_in(config, usage, &curated::current())
    }

    /// [`of`](Self::of) against one captured list.
    pub(crate) fn of_in(
        config: Option<&ModelConfig>,
        usage: Option<&TokenUsage>,
        facts: &Bundle,
    ) -> CallRecord {
        let Some(config) = config else {
            return CallRecord::default();
        };
        let billing = billing_of(config);
        let priced = Some(billing)
            .filter(|b| *b == Billing::Metered)
            .and_then(|_| price_in(config, facts));
        let Some(priced) = priced else {
            return CallRecord {
                billing: Some(billing),
                ..Default::default()
            };
        };
        // A call that reported no usage had a price and cost nothing we know.
        let Some(cost) = usage
            .and_then(Tokens::of)
            .map_or(Some(0.0), |used| priced.cost_of_call(&used))
        else {
            // The call fell in a tier with no usable rate: tokens only.
            return CallRecord {
                billing: Some(billing),
                ..Default::default()
            };
        };
        // The words a stored ledger and the page already know: the catalog
        // the engine ships (and refreshes) is its "built-in" price.
        let (source, as_of) = match priced.source {
            PriceSource::Catalog(day) => ("built-in", Some(day)),
            PriceSource::Config => ("config", None),
        };
        CallRecord {
            billing: Some(billing),
            priced: true,
            cost_usd: Some(cost),
            source: Some(source.to_string()),
            as_of,
        }
    }
}

/// The model's price, if one is known: its own `price`, else the catalog in
/// force.
pub fn price_of(cfg: &ModelConfig) -> Option<Priced> {
    price_in(cfg, &curated::current())
}

/// [`price_of`] against one captured catalog snapshot.
pub(crate) fn price_in(cfg: &ModelConfig, facts: &Bundle) -> Option<Priced> {
    if let Some(configured) = cfg.price.as_ref() {
        let price = Price::from(configured);
        return Some(Priced {
            base: price,
            tiers: Vec::new(),
            source: PriceSource::Config,
        });
    }
    // Exact provider address and exact model: another host with the same
    // provider name, or a similar model name, inherits nothing.
    let cost = facts.exact(cfg)?.cost()?;
    if cost.has_unestimated_rates() {
        return None;
    }
    let base = usable(cost.base())?;
    let day = cost.as_of().unwrap_or_else(|| facts.facts_observed_on());
    Some(Priced {
        base,
        tiers: cost.tiers().map(|(over, rates)| (over, usable(rates))).collect(),
        source: PriceSource::Catalog(day.to_string()),
    })
}

/// A catalog rate set as a price. A missing or zero input or output rate is
/// no price; a missing cache rate costs what plain input costs, as it does
/// for a price the person typed.
fn usable(rates: Rates) -> Option<Price> {
    let input = rates.input.filter(|n| *n > 0.0)?;
    let output = rates.output.filter(|n| *n > 0.0)?;
    Some(Price {
        input,
        output,
        cache_read: rates.cache_read.unwrap_or(input),
        cache_write: rates.cache_write.unwrap_or(input),
    })
}

/// How a model's use is paid for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Billing {
    /// By the token, on a key: the spend is the person's.
    Metered,
    /// Against an account's allowance or subscription: no per-token price.
    Account,
    /// On this machine or its network.
    Local,
    /// A model the config no longer lists.
    Unknown,
}

impl Billing {
    /// Whether the session's spend counts this model's tokens.
    pub fn is_spend(self) -> bool {
        matches!(self, Billing::Metered | Billing::Unknown)
    }
}

pub fn billing_of(cfg: &ModelConfig) -> Billing {
    let kind = cfg.provider_kind();
    let account = matches!(
        cfg.sign_in(),
        SignIn::ChatGpt | SignIn::LinggenAccount | SignIn::Claude
    ) || kind == ProviderKind::Proxy;
    match () {
        _ if account => Billing::Account,
        // The person said what it costs: it is metered, whatever its host.
        _ if cfg.price.is_some() => Billing::Metered,
        // Anthropic's wire is a hosted service even behind a local gateway:
        // someone bills it, so its tokens stay spend.
        _ if kind != ProviderKind::Anthropic && crate::provider::is_local_backend(&cfg.url) => {
            Billing::Local
        }
        // Ollama's hosted models are a plan, not a price per token.
        _ if kind == ProviderKind::Ollama => Billing::Account,
        _ => Billing::Metered,
    }
}

/// What a model used in a session under one billing class, and what that
/// cost if it can be said — one row of the ledger's entries.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelSpend {
    pub model: String,
    pub billing: Billing,
    pub calls: u64,
    pub unreported: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// An estimate at list price, added up when each call was made; absent
    /// without a price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// "built-in" or "config", and for a built-in price the day it was true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<String>,
}

/// What a session's engine calls spent: per model and class, and in all for
/// the models whose use is paid by the token. The ledger's half of what a
/// session cost (`server::session_cost`, which the chat's top bar and
/// `agent_status` both ask).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SessionSpend {
    pub models: Vec<ModelSpend>,
    /// The tokens of every model whose use is spend (not an account's, not
    /// local).
    #[serde(serialize_with = "counted")]
    pub tokens: Tokens,
    /// The part of `tokens` whose calls had a price — what `cost_usd` is of.
    #[serde(serialize_with = "counted")]
    pub priced_tokens: Tokens,
    /// The part of `tokens` of the `unpriced` models.
    #[serde(serialize_with = "counted")]
    pub unpriced_tokens: Tokens,
    /// Whether the session used any token paid for by the token — what the
    /// top bar shows a spend for.
    pub spent: bool,
    /// The estimate over the priced calls among them; absent when none had
    /// a price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Metered models with tokens but no price — their cost is not in
    /// `cost_usd`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unpriced: Vec<String>,
}

/// Tokens as the UI reads them: the four counts, and the prompt they make
/// (`Tokens::prompt`) so no reader adds them up again.
fn counted<S: serde::Serializer>(t: &Tokens, s: S) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct Counted<'a> {
        #[serde(flatten)]
        tokens: &'a Tokens,
        prompt: u64,
    }
    Counted {
        tokens: t,
        prompt: t.prompt(),
    }
    .serialize(s)
}

impl ModelManager {
    pub(crate) fn config_of(&self, id: &str) -> Option<&ModelConfig> {
        self.models.get(self.key(id).as_ref()).map(|m| &m.config)
    }
}

/// What the ledger's models used and cost. Only sums what the ledger holds:
/// the class and the amount of each call were fixed when it was made, so no
/// config, price or table is read here.
pub fn spend_of(ledger: &SessionUsage) -> SessionSpend {
    let mut out = SessionSpend::default();
    for e in &ledger.entries {
        let billing = e.billing.unwrap_or(Billing::Unknown);
        if billing.is_spend() {
            out.tokens.add(&e.tally.tokens);
            if let Some(c) = e.cost_usd {
                *out.cost_usd.get_or_insert(0.0) += c;
            }
            if e.priced {
                out.priced_tokens.add(&e.tally.tokens);
            }
            let unpriced =
                billing == Billing::Metered && !e.priced && !e.tally.tokens.is_zero();
            if unpriced {
                out.unpriced_tokens.add(&e.tally.tokens);
                if !out.unpriced.contains(&e.model) {
                    out.unpriced.push(e.model.clone());
                }
            }
        }
        out.models.push(ModelSpend {
            model: e.model.clone(),
            billing,
            calls: e.tally.calls,
            unreported: e.tally.unreported,
            input: e.tally.tokens.input,
            output: e.tally.tokens.output,
            cache_read: e.tally.tokens.cache_read,
            cache_write: e.tally.tokens.cache_write,
            cost_usd: e.cost_usd,
            source: e.source.clone(),
            as_of: e.as_of.clone(),
        });
    }
    out.spent = !out.tokens.is_zero();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::models::ModelTally;
    use crate::state_fs::session_usage::LedgerEntry;

    fn model(provider: &str, url: &str, name: &str) -> ModelConfig {
        serde_json::from_value(serde_json::json!({
            "id": "m", "provider": provider, "url": url, "model": name,
        }))
        .unwrap()
    }

    fn price(input: f64, output: f64, cache_read: f64, cache_write: f64) -> Price {
        Price { input, output, cache_read, cache_write }
    }

    fn call(prompt: usize, read: usize, write: usize, out: usize) -> TokenUsage {
        TokenUsage {
            prompt_tokens: Some(prompt),
            completion_tokens: Some(out),
            total_tokens: None,
            cached_tokens: Some(read),
            cache_write_tokens: Some(write),
        }
    }

    /// What the calls cost at `p`, each priced on its own.
    fn cost(p: &Priced, calls: &[TokenUsage]) -> f64 {
        calls
            .iter()
            .map(|c| p.cost_of_call(&Tokens::of(c).unwrap()).unwrap())
            .sum()
    }

    const API: &str = "https://api.anthropic.com";

    /// One catalog entry: text in and out, with `cost`.
    fn facts_entry(id: &str, cost: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "id": id, "name": id, "stability": "stable",
            "modalities": {"input": ["text"], "output": ["text"]}, "cost": cost,
        })
    }

    /// Anthropic's address, observed on 2026-11-23, with these entries.
    fn anthropic(models: Vec<serde_json::Value>) -> Bundle {
        Bundle::for_tests("2026-11-23", API, models.into())
    }

    /// The four models the retired hand table listed, at its rates.
    fn four() -> Bundle {
        let flat = |i: f64, o: f64, r: f64, w: f64| {
            serde_json::json!({"input": i, "output": o, "cache_read": r, "cache_write": w})
        };
        anthropic(vec![
            facts_entry("claude-opus-5-5", flat(4.0, 20.0, 0.20, 5.0)),
            facts_entry("claude-sonnet-5-5", flat(2.0, 10.0, 0.10, 2.5)),
            facts_entry("claude-fable-5-2", flat(8.0, 40.0, 0.20, 10.0)),
            facts_entry(
                "claude-haiku-5-5",
                serde_json::json!({"input": 0.10, "output": 0.50, "cache_read": 0.01,
                    "cache_write": 0.125, "tiers": [{"over": 100000, "input": 0.50,
                    "output": 2.50, "cache_read": 0.05, "cache_write": 0.625}]}),
            ),
        ])
    }

    fn priced(facts: &Bundle, url: &str, name: &str) -> Option<Priced> {
        price_in(&model("anthropic", url, name), facts)
    }

    #[test]
    fn the_four_models_the_hand_table_listed_cost_the_same_from_the_catalog() {
        let facts = four();
        // 1,000,000 fresh input, 200,000 cache read, 400,000 cache write,
        // 100,000 output on Sonnet 5.5 (2 / 10 / 0.10 / 2.50):
        // 2.00 + 0.02 + 1.00 + 1.00 = 4.02.
        let t = [call(1_600_000, 200_000, 400_000, 100_000)];
        let sonnet = priced(&facts, API, "claude-sonnet-5-5").unwrap();
        assert!((cost(&sonnet, &t) - 4.02).abs() < 1e-9, "{}", cost(&sonnet, &t));
        // The catalog gave no price date: the day it was observed stands.
        assert_eq!(sonnet.source, PriceSource::Catalog("2026-11-23".into()));
        // Opus 5.5 (4 / 20 / 0.20 / 5): 4 + 0.04 + 2 + 2 = 8.04.
        let opus = priced(&facts, API, "claude-opus-5-5").unwrap();
        assert!((cost(&opus, &t) - 8.04).abs() < 1e-9);
        // Fable 5.2 (8 / 40 / 0.20 / 10): 8 + 0.04 + 4 + 4 = 16.04.
        let fable = priced(&facts, API, "claude-fable-5-2").unwrap();
        assert!((cost(&fable, &t) - 16.04).abs() < 1e-9);
    }

    /// The hand table could go because the copy the engine ships prices the
    /// same four models at the same address.
    #[test]
    fn the_shipped_catalog_prices_the_four_models_the_hand_table_listed() {
        let shipped = curated::current();
        for name in ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5", "claude-fable-5-2"] {
            assert!(priced(&shipped, API, name).is_some(), "{name}");
            assert!(priced(&shipped, "", name).is_some(), "{name} at the default base");
        }
    }

    #[test]
    fn haiku_prices_a_long_prompt_call_by_itself() {
        let haiku = priced(&four(), API, "claude-haiku-5-5").unwrap();
        // A short call: 50,000 in, 1,000 out → 0.005 + 0.0005.
        let short = call(50_000, 0, 0, 1_000);
        // A long call: 150,000 prompt (100,000 read) + 2,000 out at the
        // higher rates: 50,000 × 0.50 + 100,000 × 0.05 + 2,000 × 2.50 per M
        // = 0.025 + 0.005 + 0.005 = 0.035.
        let long = call(150_000, 100_000, 0, 2_000);
        let both = [short, long];
        let expected = 0.0055 + 0.035;
        assert!((cost(&haiku, &both) - expected).abs() < 1e-9, "{}", cost(&haiku, &both));
        // The same two calls with the long one priced short would be cheaper.
        let all = both.iter().fold(Tokens::default(), |mut a, c| {
            a.add(&Tokens::of(c).unwrap());
            a
        });
        assert!(cost(&haiku, &both) > haiku.base.cost(&all));
    }

    /// A tier starts where the provider says, not at one threshold for all.
    #[test]
    fn each_context_tier_starts_at_its_own_threshold() {
        let facts = anthropic(vec![facts_entry(
            "tiered",
            serde_json::json!({"input": 1.0, "output": 1.0, "tiers": [
                {"over": 50000, "input": 2.0, "output": 2.0},
                {"over": 200000, "input": 4.0, "output": 4.0}]}),
        )]);
        let p = priced(&facts, API, "tiered").unwrap();
        let per_token = |prompt: usize| {
            let t = Tokens::of(&call(prompt, 0, 0, 0)).unwrap();
            p.cost_of_call(&t).unwrap() * 1_000_000.0 / prompt as f64
        };
        // "Over" is strictly over: the threshold itself is the lower tier.
        for (prompt, rate) in [(50_000, 1.0), (50_001, 2.0), (200_000, 2.0), (200_001, 4.0)] {
            assert!((per_token(prompt) - rate).abs() < 1e-9, "{prompt}");
        }
    }

    #[test]
    fn a_call_in_a_tier_with_no_usable_rate_has_no_figure() {
        let facts = anthropic(vec![facts_entry(
            "gap",
            serde_json::json!({"input": 1.0, "output": 2.0,
                "tiers": [{"over": 100000, "input": 3.0}]}),
        )]);
        let p = priced(&facts, API, "gap").unwrap();
        let of = |prompt| p.cost_of_call(&Tokens::of(&call(prompt, 0, 0, 10)).unwrap());
        assert!(of(100_000).is_some());
        assert_eq!(of(100_001), None, "no borrowed base rate, no guessed tier");
        // The ledger holds tokens and no amount for such a call.
        let cfg = model("anthropic", API, "gap");
        let rec = CallRecord::of_in(Some(&cfg), Some(&call(150_000, 0, 0, 10)), &facts);
        assert_eq!((rec.billing, rec.priced, rec.cost_usd), (Some(Billing::Metered), false, None));
        let rec = CallRecord::of_in(Some(&cfg), Some(&call(1_000, 0, 0, 10)), &facts);
        assert!(rec.priced && rec.cost_usd.is_some());
    }

    /// A dated estimate versus tokens only.
    #[test]
    fn a_price_is_dated_or_there_is_none() {
        let facts = anthropic(vec![
            facts_entry("own-date", serde_json::json!({"input": 1.0, "output": 2.0, "as_of": "2026-11-01"})),
            facts_entry("observed", serde_json::json!({"input": 1.0, "output": 2.0})),
            // Cache rates the file omits cost what plain input costs.
            facts_entry("no-cache", serde_json::json!({"input": 3.0, "output": 6.0})),
            facts_entry("zero-input", serde_json::json!({"input": 0, "output": 2.0})),
            facts_entry("no-output", serde_json::json!({"input": 1.0})),
            facts_entry("audio", serde_json::json!({"input": 1.0, "output": 2.0, "unestimated": ["input_audio"]})),
            serde_json::json!({"id": "no-price", "name": "n", "stability": "stable",
                "modalities": {"input": ["text"], "output": ["text"]}}),
        ]);
        let source = |name: &str| priced(&facts, API, name).map(|p| p.source);
        assert_eq!(source("own-date"), Some(PriceSource::Catalog("2026-11-01".into())));
        assert_eq!(source("observed"), Some(PriceSource::Catalog("2026-11-23".into())));
        let p = priced(&facts, API, "no-cache").unwrap();
        assert_eq!(p.base, price(3.0, 6.0, 3.0, 3.0));
        for none in ["zero-input", "no-output", "audio", "no-price", "unknown-entry"] {
            assert_eq!(source(none), None, "{none}: tokens only");
        }
        // The record of a call to a dated price carries the day.
        let rec = CallRecord::of_in(Some(&model("anthropic", API, "own-date")), Some(&call(1_000, 0, 0, 10)), &facts);
        assert_eq!((rec.source.as_deref(), rec.as_of.as_deref()), (Some("built-in"), Some("2026-11-01")));
    }

    #[test]
    fn only_an_exact_address_and_model_have_a_catalog_price() {
        let facts = four();
        let name = "claude-sonnet-5-5";
        assert!(priced(&facts, "", name).is_some(), "the default base is Anthropic's");
        assert!(priced(&facts, "https://api.anthropic.com/", name).is_some());
        // The provider name at another address inherits nothing.
        assert!(priced(&facts, "https://gateway.example.com/v1", name).is_none());
        assert!(price_in(&model("openai", "https://api.openai.com/v1", "gpt-6"), &facts).is_none());
        // Not listed; a similar name is not the model.
        assert!(priced(&facts, API, "claude-sonnet-3-7").is_none());
        assert!(priced(&facts, API, "claude-sonnet-5-5-20261103").is_none());
        assert!(priced(&facts, API, "claude-sonnet-5-55").is_none());
        // A Claude subscription is billed to the account.
        let mut oauth = model("anthropic", API, name);
        oauth.auth_mode = Some("claude_oauth".into());
        assert!(price_in(&oauth, &facts).is_none());
        assert_eq!(billing_of(&oauth), Billing::Account);
    }

    #[test]
    fn a_configured_price_wins_and_defaults_its_cache_prices_to_input() {
        let mut cfg = model("anthropic", API, "claude-sonnet-5-5");
        cfg.price = Some(ModelPrice {
            input: 1.0,
            output: 3.0,
            cache_read: None,
            cache_write: Some(1.5),
        });
        let priced = price_in(&cfg, &four()).unwrap();
        assert_eq!(priced.source, PriceSource::Config);
        assert_eq!(priced.base, price(1.0, 3.0, 1.0, 1.5));
        // A configured price is flat: a long-prompt call costs the same.
        let t = [call(200_000, 50_000, 0, 1_000)];
        assert!((cost(&priced, &t) - (0.15 + 0.05 + 0.003)).abs() < 1e-9);
        // It prices a model the catalog has never heard of, too.
        let mut gemini = model(
            "gemini",
            "https://generativelanguage.googleapis.com/v1beta/openai",
            "g-9",
        );
        gemini.price = cfg.price.clone();
        assert!(price_in(&gemini, &four()).is_some());
    }

    #[test]
    fn a_model_is_billed_by_the_token_by_account_or_locally() {
        assert_eq!(billing_of(&model("anthropic", API, "x")), Billing::Metered);
        assert_eq!(
            billing_of(&model("gemini", "https://example.com/v1", "x")),
            Billing::Metered
        );
        assert_eq!(billing_of(&model("chatgpt", "", "x")), Billing::Account);
        assert_eq!(billing_of(&model("proxy", "", "x")), Billing::Account);
        let mut cloud = model("openai", "https://linggen.dev/v1", "x");
        cloud.auth_mode = Some("linggen_account".into());
        assert_eq!(billing_of(&cloud), Billing::Account);
        assert_eq!(
            billing_of(&model("ollama", "http://127.0.0.1:11434", "x")),
            Billing::Local
        );
        assert_eq!(
            billing_of(&model("openai", "http://localhost:1234/v1", "x")),
            Billing::Local
        );
        assert_eq!(
            billing_of(&model("ollama", "https://ollama.com", "x")),
            Billing::Account
        );
    }

    /// Finding 6: the person said what it costs — whatever the host.
    #[test]
    fn a_model_with_a_configured_price_is_metered_whatever_its_host() {
        let mut gateway = model("openai", "http://192.168.1.20:8000/v1", "x");
        assert_eq!(billing_of(&gateway), Billing::Local);
        gateway.price = Some(ModelPrice {
            input: 1.0,
            output: 2.0,
            cache_read: None,
            cache_write: None,
        });
        assert_eq!(billing_of(&gateway), Billing::Metered);
        let mut local = model("ollama", "http://127.0.0.1:11434", "x");
        local.price = gateway.price.clone();
        assert_eq!(billing_of(&local), Billing::Metered);
        // An allowance model stays one: a price on it is a config error.
        let mut plan = model("chatgpt", "", "x");
        plan.price = gateway.price.clone();
        assert_eq!(billing_of(&plan), Billing::Account);
    }

    /// The record of a call: class and cost fixed from the config as it is
    /// when the call is made.
    #[test]
    fn a_call_is_recorded_with_its_class_and_its_cost() {
        let used = call(9_200, 5_000, 3_000, 400); // 1,200 in, 3,000 written, 5,000 read, 400 out
        let facts = four();
        let sonnet = model("anthropic", API, "claude-sonnet-5-5");
        let rec = CallRecord::of_in(Some(&sonnet), Some(&used), &facts);
        assert_eq!(rec.billing, Some(Billing::Metered));
        assert!(rec.priced);
        // 1,200 x 2 + 3,000 x 2.5 + 5,000 x 0.1 + 400 x 10, per million.
        assert!((rec.cost_usd.unwrap() - 0.0144).abs() < 1e-9, "{rec:?}");
        assert_eq!(
            (rec.source.as_deref(), rec.as_of.as_deref()),
            (Some("built-in"), Some("2026-11-23"))
        );
        // A call that reported nothing had a price and cost nothing known.
        let none = CallRecord::of_in(Some(&sonnet), None, &facts);
        assert_eq!((none.priced, none.cost_usd), (true, Some(0.0)));
        // No price: the class, no amount.
        let gateway = model("anthropic", "https://gw.example.com", "claude-sonnet-5-5");
        let rec = CallRecord::of_in(Some(&gateway), Some(&used), &facts);
        assert_eq!(
            (rec.billing, rec.priced, rec.cost_usd),
            (Some(Billing::Metered), false, None)
        );
        // An account model: its class, never an amount (a price is a config error).
        let rec = CallRecord::of_in(Some(&model("chatgpt", "", "gpt")), Some(&used), &facts);
        assert_eq!(
            (rec.billing, rec.priced, rec.cost_usd),
            (Some(Billing::Account), false, None)
        );
        // A model not in the config claims nothing.
        assert_eq!(CallRecord::of_in(None, Some(&used), &facts), CallRecord::default());
    }

    fn entry(
        model: &str,
        billing: Option<Billing>,
        priced: bool,
        input: u64,
        cost: Option<f64>,
    ) -> LedgerEntry {
        LedgerEntry {
            model: model.into(),
            billing,
            priced,
            tally: ModelTally {
                calls: 1,
                unreported: 0,
                tokens: Tokens {
                    input,
                    output: 10,
                    cache_read: 0,
                    cache_write: 0,
                },
            },
            cost_usd: cost,
            ..Default::default()
        }
    }

    /// The total only sums what the ledger holds.
    #[test]
    fn a_session_spend_sums_the_stored_amounts_and_names_the_unpriced() {
        let metered = Some(Billing::Metered);
        let ledger = SessionUsage::of_entries(vec![
            entry("sonnet", metered, true, 1_000_000, Some(3.0)),
            entry("gem", metered, false, 500, None),
            entry("gpt", Some(Billing::Account), false, 9_000, None),
            entry("gone", metered, false, 7, None),
            // The same model before it had a price: tokens, no amount.
            entry("sonnet", metered, false, 40, None),
            // From before classes were recorded: tokens, no claim.
            entry("legacy", None, false, 70, None),
        ]);
        let spend = spend_of(&ledger);
        assert!((spend.cost_usd.unwrap() - 3.0).abs() < 1e-9);
        assert_eq!(
            spend.unpriced,
            ["gem", "gone", "sonnet"].map(String::from).to_vec()
        );
        // The account's tokens are no spend; the legacy entry's are shown.
        assert_eq!(spend.tokens.input, 1_000_000 + 500 + 7 + 40 + 70);
        let by = |m: &str, priced: bool| {
            spend
                .models
                .iter()
                .find(|s| s.model == m && s.cost_usd.is_some() == priced)
                .unwrap()
        };
        assert_eq!(by("gpt", false).billing, Billing::Account);
        assert_eq!(by("legacy", false).billing, Billing::Unknown);
        assert_eq!(by("sonnet", true).cost_usd, Some(3.0));
        // The priced and unpriced parts are told here, once — the top bar's
        // hover only words them.
        assert_eq!(spend.priced_tokens.input, 1_000_000);
        assert_eq!(spend.unpriced_tokens.input, 500 + 7 + 40);
        assert!(spend.spent);
        let wire = serde_json::to_value(&spend).unwrap();
        assert_eq!(wire["tokens"]["prompt"], 1_000_000 + 500 + 7 + 40 + 70);
        assert_eq!(wire["unpriced_tokens"]["prompt"], 547);
        assert_eq!(wire["spent"], true);
        // Only an account's tokens: nothing spent by the token.
        let account = spend_of(&SessionUsage::of_entries(vec![entry(
            "gpt",
            Some(Billing::Account),
            false,
            9_000,
            None,
        )]));
        assert!(!account.spent);
    }
}
