//! What a session's model calls used — tokens, by billing class.
//!
//! The engine records what each call used (`usage.rs`, the session ledger).
//! No price is looked up and none is configured: a key provider's spend is
//! the person's to see in that provider's own console, and the session shows
//! tokens. A model that bills an account (the ChatGPT sign-in, Linggen
//! Cloud, a room, a Claude subscription) or runs on this machine is
//! `Billing::Account` or `Billing::Local`, and a session's spend leaves it
//! out. A ledger written while prices were kept still holds its amounts;
//! `spend_of` sums them as stored and nothing re-prices them.

use super::{ModelManager, TokenUsage};
use crate::config::{ModelConfig, ProviderKind, SignIn};
use crate::state_fs::session_usage::SessionUsage;
use super::Tokens;
use serde::Serialize;

/// What a call is kept as in the session ledger, worked out when the call is
/// made from the model's config as it is then: its billing class. The price
/// fields stay for the ledger's shape; a new call never fills them.
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
    pub fn of(config: Option<&ModelConfig>, _usage: Option<&TokenUsage>) -> CallRecord {
        CallRecord { billing: config.map(billing_of), ..Default::default() }
    }
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

    /// A key provider's call is kept with its class and no amount.
    #[test]
    fn a_new_call_is_recorded_with_its_class_and_never_a_price() {
        let used = TokenUsage { prompt_tokens: Some(1_000), completion_tokens: Some(10), total_tokens: None, cached_tokens: None, cache_write_tokens: None };
        let rec = CallRecord::of(Some(&model("anthropic", "https://api.anthropic.com", "claude-sonnet-5-5")), Some(&used));
        assert_eq!(rec, CallRecord { billing: Some(Billing::Metered), ..Default::default() });
        assert_eq!(CallRecord::of(None, Some(&used)), CallRecord::default());
    }

    #[test]
    fn a_model_is_billed_by_the_token_by_account_or_locally() {
        assert_eq!(billing_of(&model("anthropic", "https://api.anthropic.com", "x")), Billing::Metered);
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
