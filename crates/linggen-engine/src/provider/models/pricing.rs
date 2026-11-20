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

use super::{signs_in_with_chatgpt, ModelManager, ModelTally, Tokens};
use crate::config::{ModelConfig, ModelPrice};
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
    /// What `tally` cost at this price.
    pub fn cost(&self, tally: &ModelTally) -> f64 {
        let short = tally.tokens.minus(&tally.long);
        self.price.cost(&short) + self.long.cost(&tally.long)
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
    cfg.provider == "anthropic"
        && cfg.auth_mode.as_deref() != Some("claude_oauth")
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
    let mode = cfg.auth_mode.as_deref();
    let account = signs_in_with_chatgpt(cfg)
        || matches!(mode, Some("linggen_account" | "claude_oauth"))
        || cfg.provider == "proxy";
    match () {
        _ if account => Billing::Account,
        // The person said what it costs: it is metered, whatever its host.
        _ if cfg.price.is_some() => Billing::Metered,
        // Anthropic's wire is a hosted service even behind a local gateway:
        // someone bills it, so its tokens stay spend.
        _ if cfg.provider != "anthropic" && crate::provider::is_local_backend(&cfg.url) => {
            Billing::Local
        }
        // Ollama's hosted models are a plan, not a price per token.
        _ if cfg.provider == "ollama" => Billing::Account,
        _ => Billing::Metered,
    }
}

/// What one model used in a session, and what that cost if it can be said.
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
    /// An estimate at list price; absent without a price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", flatten)]
    pub price_source: Option<PriceSource>,
}

/// What a session spent: per model, and in all for the models whose use is
/// paid by the token. The one answer the chat's top bar and `agent_status`
/// both give.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SessionSpend {
    pub models: Vec<ModelSpend>,
    /// The tokens of every model whose use is spend (not an account's, not
    /// local).
    pub tokens: Tokens,
    /// The estimate over the priced models among them; absent when none has
    /// a price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Spend models with tokens but no price — their cost is not in
    /// `cost_usd`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unpriced: Vec<String>,
}

impl SessionSpend {
    /// Whether the session spent by the token at all.
    pub fn has_spend(&self) -> bool {
        self.models.iter().any(|m| m.billing.is_spend())
    }
}

impl ModelManager {
    pub(crate) fn config_of(&self, id: &str) -> Option<&ModelConfig> {
        self.models.get(self.key(id).as_ref()).map(|m| &m.config)
    }

    /// What the ledger's models used and cost, by their configs now.
    pub fn spend_of(&self, ledger: &SessionUsage) -> SessionSpend {
        let mut out = SessionSpend::default();
        for (id, tally) in &ledger.models {
            // The class the calls had when they were made; a price is the
            // config's now, and only a metered model has one.
            let billing = tally.billing.unwrap_or(Billing::Unknown);
            let priced = self
                .config_of(id)
                .and_then(price_of)
                .filter(|_| billing == Billing::Metered);
            let cost = priced.map(|p| p.cost(tally));
            if billing.is_spend() {
                out.tokens.add(&tally.tokens);
                match cost {
                    Some(c) => *out.cost_usd.get_or_insert(0.0) += c,
                    None if billing == Billing::Metered && !tally.tokens.is_zero() => {
                        out.unpriced.push(id.clone())
                    }
                    None => {}
                }
            }
            out.models.push(ModelSpend {
                model: id.clone(),
                billing,
                calls: tally.calls,
                unreported: tally.unreported,
                input: tally.tokens.input,
                output: tally.tokens.output,
                cache_read: tally.tokens.cache_read,
                cache_write: tally.tokens.cache_write,
                cost_usd: cost,
                price_source: priced.map(|p| p.source),
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::models::TokenUsage;

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

    fn tally(calls: &[TokenUsage]) -> ModelTally {
        let mut t = ModelTally::default();
        for c in calls {
            t.add(Some(c));
        }
        t
    }

    const API: &str = "https://api.anthropic.com";

    #[test]
    fn the_four_listed_models_cost_their_public_price() {
        // 1,000,000 fresh input, 200,000 cache read, 400,000 cache write,
        // 100,000 output on Sonnet 5.5 (2 / 10 / 0.10 / 2.50):
        // 2.00 + 0.02 + 1.00 + 1.00 = 4.02.
        let t = tally(&[call(1_600_000, 200_000, 400_000, 100_000)]);
        // One call over 100K prompt tokens: Sonnet has no long tier.
        let sonnet = price_of(&model("anthropic", API, "claude-sonnet-5-5")).unwrap();
        assert!((sonnet.cost(&t) - 4.02).abs() < 1e-9, "{}", sonnet.cost(&t));
        assert_eq!(sonnet.source, PriceSource::BuiltIn("2026-11-03"));
        // Opus 5.5 (4 / 20 / 0.20 / 5): 4 + 0.04 + 2 + 2 = 8.04.
        let opus = price_of(&model("anthropic", API, "claude-opus-5-5")).unwrap();
        assert!((opus.cost(&t) - 8.04).abs() < 1e-9);
        // Fable 5.2 (8 / 40 / 0.20 / 10): 8 + 0.04 + 4 + 4 = 16.04.
        let fable = price_of(&model("anthropic", API, "claude-fable-5-2")).unwrap();
        assert!((fable.cost(&t) - 16.04).abs() < 1e-9);
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
        let both = tally(&[short.clone(), long]);
        let expected = 0.0055 + 0.035;
        assert!(
            (haiku.cost(&both) - expected).abs() < 1e-9,
            "{}",
            haiku.cost(&both)
        );
        // The same two calls with the long one priced short would be cheaper.
        assert!(haiku.cost(&both) > haiku.price.cost(&both.tokens));
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
        let t = tally(&[call(200_000, 50_000, 0, 1_000)]);
        assert!((priced.cost(&t) - (0.15 + 0.05 + 0.003)).abs() < 1e-9);
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

    #[test]
    fn a_session_spend_counts_priced_models_and_names_the_unpriced() {
        let mut priced = model("anthropic", API, "claude-sonnet-5-5");
        priced.id = "sonnet".into();
        let mut unpriced = model("gemini", "https://example.com/v1", "g");
        unpriced.id = "gem".into();
        let mut account = model("chatgpt", "", "gpt");
        account.id = "gpt".into();
        let models = ModelManager::new_with_credentials(
            vec![priced, unpriced, account],
            &crate::credentials::Credentials::default(),
        );
        let with = |billing: Option<Billing>, t: ModelTally| ModelTally { billing, ..t };
        let mut ledger = SessionUsage::default();
        let metered = Some(Billing::Metered);
        ledger.models.insert(
            "sonnet".into(),
            with(metered, tally(&[call(1_000_000, 0, 0, 100_000)])),
        );
        ledger
            .models
            .insert("gem".into(), with(metered, tally(&[call(500, 0, 0, 20)])));
        ledger.models.insert(
            "gpt".into(),
            with(Some(Billing::Account), tally(&[call(9_000, 0, 0, 90)])),
        );
        ledger
            .models
            .insert("gone".into(), with(metered, tally(&[call(7, 0, 0, 1)])));
        let spend = models.spend_of(&ledger);
        // 1M in × 2 + 0.1M out × 10 = 3.00; the ChatGPT account has no price.
        assert!((spend.cost_usd.unwrap() - 3.0).abs() < 1e-9);
        assert_eq!(spend.unpriced, vec!["gem".to_string(), "gone".to_string()]);
        assert_eq!(spend.tokens.input, 1_000_000 + 500 + 7);
        let by = |m: &str| spend.models.iter().find(|s| s.model == m).unwrap();
        assert_eq!(by("gpt").billing, Billing::Account);
        assert_eq!(by("gpt").cost_usd, None);
        assert_eq!(by("gem").cost_usd, None);
        assert_eq!(by("gone").billing, Billing::Metered);
    }

    /// Finding 5: the class a call had when it was made is what it keeps —
    /// an allowance model that left the config later is still no spend, and
    /// a call from before classes were recorded makes no claim.
    #[test]
    fn a_calls_billing_class_does_not_change_with_the_config() {
        let models = ModelManager::new_with_credentials(
            Vec::new(),
            &crate::credentials::Credentials::default(),
        );
        let mut ledger = SessionUsage::default();
        ledger.models.insert(
            "old-chatgpt".into(),
            ModelTally {
                billing: Some(Billing::Account),
                ..tally(&[call(9_000, 0, 0, 90)])
            },
        );
        ledger
            .models
            .insert("legacy".into(), tally(&[call(70, 0, 0, 1)]));
        let spend = models.spend_of(&ledger);
        assert_eq!(
            spend.tokens.input, 70,
            "an allowance model's tokens are no spend, gone or not"
        );
        assert!(
            spend.unpriced.is_empty(),
            "no claim about a call of unknown class: {:?}",
            spend.unpriced
        );
        assert_eq!(spend.cost_usd, None);
        let legacy = spend.models.iter().find(|m| m.model == "legacy").unwrap();
        assert_eq!(
            (legacy.billing, legacy.input, legacy.cost_usd),
            (Billing::Unknown, 70, None)
        );
    }

    /// A ledger written before classes were recorded still reads.
    #[test]
    fn an_older_ledger_entry_without_a_class_reads() {
        let old = r#"{"calls":1,"unreported":0,"tokens":{"input":5,"output":1,"cache_read":0,"cache_write":0}}"#;
        let tally: ModelTally = serde_json::from_str(old).unwrap();
        assert_eq!(tally.billing, None);
    }
}
