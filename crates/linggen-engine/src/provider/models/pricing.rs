//! What a session's model calls cost — when a price is known.
//!
//! The engine records what each call used (`usage.rs`, the session ledger);
//! a dollar figure is derived here, at read time, and only for a model with
//! a price: the built-in table below (dated, Anthropic's own API only) or the
//! model's own `price` in the config, which wins. No price, no figure — the
//! tokens are shown and nothing is guessed. A model that bills an account
//! (the ChatGPT sign-in, Linggen Cloud, a room, a Claude subscription) or
//! runs on this machine has no per-token price: it is `Billing::Account` or
//! `Billing::Local`, and a session's spend leaves it out.

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

/// One row of the built-in table: a model of Anthropic's own API, its price,
/// and the day that price was checked against the public pricing page.
struct Listed {
    model: &'static str,
    price: Price,
    /// Haiku 5.5 only: the price of a request whose prompt is over
    /// [`super::LONG_PROMPT_TOKENS`].
    long: Option<Price>,
    as_of: &'static str,
}

const fn price(input: f64, output: f64, cache_read: f64, cache_write: f64) -> Price {
    Price {
        input,
        output,
        cache_read,
        cache_write,
    }
}

/// Anthropic's first-party API, USD per million tokens. The cache-write
/// price is the 5-minute write, 1.25x input: the engine sends plain
/// `cache_control: ephemeral`, which is 5 minutes (no 1-hour ttl is set
/// anywhere). Checked against platform.claude.com/docs/en/about-claude/pricing
/// (the rows' prices were true on 2026-11-03 and read again on 2026-11-19).
/// Haiku 5.5's long-prompt input (0.50) and output (2.50) are on that page;
/// its long-prompt cache rates (0.05 read, 0.625 write) are DERIVED from them
/// by the same ratios as everywhere else (0.1x read, 1.25x write), not listed.
const TABLE: &[Listed] = &[
    Listed {
        model: "claude-opus-5-5",
        price: price(4.0, 20.0, 0.20, 5.0),
        long: None,
        as_of: "2026-11-03",
    },
    Listed {
        model: "claude-sonnet-5-5",
        price: price(2.0, 10.0, 0.10, 2.5),
        long: None,
        as_of: "2026-11-03",
    },
    Listed {
        model: "claude-haiku-5-5",
        price: price(0.10, 0.50, 0.01, 0.125),
        long: Some(price(0.50, 2.50, 0.05, 0.625)),
        as_of: "2026-11-03",
    },
    Listed {
        model: "claude-fable-5-2",
        price: price(8.0, 40.0, 0.20, 10.0),
        long: None,
        as_of: "2026-11-03",
    },
];

/// A model's price, where it came from, and (built-in only) the day it was
/// true.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Priced {
    pub price: Price,
    /// Over-[`super::LONG_PROMPT_TOKENS`] calls; the same as `price` when a
    /// model has no long-prompt tier.
    pub long: Price,
    pub source: PriceSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "source", content = "as_of", rename_all = "kebab-case")]
pub enum PriceSource {
    BuiltIn(&'static str),
    Config,
}

impl Priced {
    /// What one call cost at this price: a prompt over
    /// [`super::LONG_PROMPT_TOKENS`] at the long-prompt rates, each request on
    /// its own.
    pub fn cost_of_call(&self, used: &Tokens) -> f64 {
        let rates = if used.prompt() > super::LONG_PROMPT_TOKENS as u64 {
            &self.long
        } else {
            &self.price
        };
        rates.cost(used)
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
        let Some(config) = config else {
            return CallRecord::default();
        };
        let billing = billing_of(config);
        let priced = Some(billing)
            .filter(|b| *b == Billing::Metered)
            .and_then(|_| price_of(config));
        let Some(priced) = priced else {
            return CallRecord {
                billing: Some(billing),
                ..Default::default()
            };
        };
        // A call that reported no usage had a price and cost nothing we know.
        let cost = usage
            .and_then(Tokens::of)
            .map_or(0.0, |used| priced.cost_of_call(&used));
        let (source, as_of) = match priced.source {
            PriceSource::BuiltIn(day) => ("built-in", Some(day.to_string())),
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

/// `name` is `model` itself or `model` with a date stamp (`-20261103`).
fn names(name: &str, model: &str) -> bool {
    match name.strip_prefix(model) {
        Some("") => true,
        Some(rest) => rest
            .strip_prefix('-')
            .is_some_and(|d| d.len() == 8 && d.bytes().all(|b| b.is_ascii_digit())),
        None => false,
    }
}

/// Anthropic's own API, by key: the one place the built-in table is true.
/// A gateway or a cloud reseller sets its own prices.
fn on_anthropic_api(cfg: &ModelConfig) -> bool {
    cfg.provider_kind() == ProviderKind::Anthropic
        && cfg.sign_in() != SignIn::Claude
        && (cfg.url.trim().is_empty()
            || crate::provider::anthropic::takes_top_level_cache_control(&cfg.url))
}

/// The model's price, if one is known: its own `price`, else the built-in
/// row for it.
pub fn price_of(cfg: &ModelConfig) -> Option<Priced> {
    if let Some(configured) = cfg.price.as_ref() {
        let price = Price::from(configured);
        return Some(Priced {
            price,
            long: price,
            source: PriceSource::Config,
        });
    }
    if !on_anthropic_api(cfg) {
        return None;
    }
    let row = TABLE.iter().find(|row| names(&cfg.model, row.model))?;
    Some(Priced {
        price: row.price,
        long: row.long.unwrap_or(row.price),
        source: PriceSource::BuiltIn(row.as_of),
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
            .map(|c| p.cost_of_call(&Tokens::of(c).unwrap()))
            .sum()
    }

    const API: &str = "https://api.anthropic.com";

    #[test]
    fn the_four_listed_models_cost_their_public_price() {
        // 1,000,000 fresh input, 200,000 cache read, 400,000 cache write,
        // 100,000 output on Sonnet 5.5 (2 / 10 / 0.10 / 2.50):
        // 2.00 + 0.02 + 1.00 + 1.00 = 4.02.
        let t = [call(1_600_000, 200_000, 400_000, 100_000)];
        // One call over 100K prompt tokens: Sonnet has no long tier.
        let sonnet = price_of(&model("anthropic", API, "claude-sonnet-5-5")).unwrap();
        assert!(
            (cost(&sonnet, &t) - 4.02).abs() < 1e-9,
            "{}",
            cost(&sonnet, &t)
        );
        assert_eq!(sonnet.source, PriceSource::BuiltIn("2026-11-03"));
        // Opus 5.5 (4 / 20 / 0.20 / 5): 4 + 0.04 + 2 + 2 = 8.04.
        let opus = price_of(&model("anthropic", API, "claude-opus-5-5")).unwrap();
        assert!((cost(&opus, &t) - 8.04).abs() < 1e-9);
        // Fable 5.2 (8 / 40 / 0.20 / 10): 8 + 0.04 + 4 + 4 = 16.04.
        let fable = price_of(&model("anthropic", API, "claude-fable-5-2")).unwrap();
        assert!((cost(&fable, &t) - 16.04).abs() < 1e-9);
    }

    #[test]
    fn haiku_prices_a_long_prompt_call_by_itself() {
        let haiku = price_of(&model("anthropic", API, "claude-haiku-5-5")).unwrap();
        // A short call: 50,000 in, 1,000 out → 0.005 + 0.0005.
        let short = call(50_000, 0, 0, 1_000);
        // A long call: 150,000 prompt (100,000 read) + 2,000 out at the
        // higher rates: 50,000 × 0.50 + 100,000 × 0.05 + 2,000 × 2.50 per M
        // = 0.025 + 0.005 + 0.005 = 0.035.
        let long = call(150_000, 100_000, 0, 2_000);
        let both = [short, long];
        let expected = 0.0055 + 0.035;
        assert!(
            (cost(&haiku, &both) - expected).abs() < 1e-9,
            "{}",
            cost(&haiku, &both)
        );
        // The same two calls with the long one priced short would be cheaper.
        let all = both.iter().fold(Tokens::default(), |mut a, c| {
            a.add(&Tokens::of(c).unwrap());
            a
        });
        assert!(cost(&haiku, &both) > haiku.price.cost(&all));
    }

    #[test]
    fn only_anthropics_own_api_has_built_in_prices() {
        let name = "claude-sonnet-5-5";
        assert!(
            price_of(&model("anthropic", "", name)).is_some(),
            "the default base is Anthropic's"
        );
        assert!(price_of(&model("anthropic", "https://gateway.example.com/v1", name)).is_none());
        assert!(price_of(&model("openai", "https://api.openai.com/v1", "gpt-6")).is_none());
        assert!(
            price_of(&model("anthropic", API, "claude-sonnet-3-7")).is_none(),
            "not listed"
        );
        // A dated name is the same model; a different version is not.
        assert!(price_of(&model("anthropic", API, "claude-sonnet-5-5-20261103")).is_some());
        assert!(price_of(&model("anthropic", API, "claude-sonnet-5-55")).is_none());
        // A Claude subscription is billed to the account.
        let mut oauth = model("anthropic", API, name);
        oauth.auth_mode = Some("claude_oauth".into());
        assert!(price_of(&oauth).is_none());
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
        let priced = price_of(&cfg).unwrap();
        assert_eq!(priced.source, PriceSource::Config);
        assert_eq!(priced.price, price(1.0, 3.0, 1.0, 1.5));
        // A configured price is flat: a long-prompt call costs the same.
        let t = [call(200_000, 50_000, 0, 1_000)];
        assert!((cost(&priced, &t) - (0.15 + 0.05 + 0.003)).abs() < 1e-9);
        // It prices a model the table has never heard of, too.
        let mut gemini = model(
            "gemini",
            "https://generativelanguage.googleapis.com/v1beta/openai",
            "g-9",
        );
        gemini.price = cfg.price.clone();
        assert!(price_of(&gemini).is_some());
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
        let sonnet = model("anthropic", API, "claude-sonnet-5-5");
        let rec = CallRecord::of(Some(&sonnet), Some(&used));
        assert_eq!(rec.billing, Some(Billing::Metered));
        assert!(rec.priced);
        // 1,200 x 2 + 3,000 x 2.5 + 5,000 x 0.1 + 400 x 10, per million.
        assert!((rec.cost_usd.unwrap() - 0.0144).abs() < 1e-9, "{rec:?}");
        assert_eq!(
            (rec.source.as_deref(), rec.as_of.as_deref()),
            (Some("built-in"), Some("2026-11-03"))
        );
        // A call that reported nothing had a price and cost nothing known.
        let none = CallRecord::of(Some(&sonnet), None);
        assert_eq!((none.priced, none.cost_usd), (true, Some(0.0)));
        // No price: the class, no amount.
        let gateway = model("anthropic", "https://gw.example.com", "claude-sonnet-5-5");
        let rec = CallRecord::of(Some(&gateway), Some(&used));
        assert_eq!(
            (rec.billing, rec.priced, rec.cost_usd),
            (Some(Billing::Metered), false, None)
        );
        // An account model: its class, never an amount (a price is a config error).
        let rec = CallRecord::of(Some(&model("chatgpt", "", "gpt")), Some(&used));
        assert_eq!(
            (rec.billing, rec.priced, rec.cost_usd),
            (Some(Billing::Account), false, None)
        );
        // A model not in the config claims nothing.
        assert_eq!(CallRecord::of(None, Some(&used)), CallRecord::default());
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
