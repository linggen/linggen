//! A monthly spending cap per pay-per-token provider — the one place that
//! decides "may this provider be called this month".
//!
//! A cap belongs to a provider (an account: its address and key), in USD per
//! UTC calendar month, kept in the runtime config (`Config::spend_caps`,
//! written only by `AgentManager::set_spend_cap`). What was spent is one small
//! file (`spend.json` in the Linggen home) holding the current month's
//! estimate per provider; a new month starts it empty. It is not summed from
//! the session ledgers: they carry no date per call.
//!
//! The figure is an ESTIMATE: each call's tokens times the model's price at
//! the time (`pricing.rs`). Use of the same key outside Linggen is not seen,
//! and a model with no known price adds nothing.
//!
//! Every backend call asks [`check`] through `ModelManager::spend_refusal`
//! (the one door every call passes; the fallback chain asks it for its
//! candidates too); the call's cost is added in `engine::spend`, the one
//! writer of the ledger. A lock test below fails when a second asker appears.
//! Only a key provider billed per token has a cap: a subscription, Linggen
//! Cloud and a local model have none here.

use super::pricing::{billing_of, Billing};
use super::providers_view::{provider_id, provider_source};
use crate::config::{ModelConfig, SignIn};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

/// At this share of the cap the person is told, once a month per provider.
const WARN_AT: f64 = 0.8;

/// What one provider has used this month.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Account {
    usd: f64,
    #[serde(default)]
    warned: bool,
}

/// The file: the month it is about and one account per provider.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Book {
    month: String,
    providers: BTreeMap<String, Account>,
}

impl Book {
    /// This month's book: a book of another month is empty.
    fn of(self, month: &str) -> Book {
        if self.month == month {
            return self;
        }
        Book { month: month.to_string(), providers: BTreeMap::new() }
    }
}

/// A provider's meter: what it used this month against its cap.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Meter {
    pub used_usd: f64,
    pub cap_usd: f64,
}

impl Meter {
    pub fn share(&self) -> f64 {
        self.used_usd / self.cap_usd
    }

    pub fn is_capped(&self) -> bool {
        self.used_usd >= self.cap_usd
    }

    fn near(&self) -> bool {
        self.share() >= WARN_AT && !self.is_capped()
    }
}

/// The caps in force and the file the spend is kept in.
struct Gate {
    caps: BTreeMap<String, f64>,
    file: PathBuf,
}

static GATE: LazyLock<Mutex<Gate>> = LazyLock::new(|| {
    Mutex::new(Gate { caps: BTreeMap::new(), file: crate::paths::linggen_home().join("spend.json") })
});

fn gate() -> std::sync::MutexGuard<'static, Gate> {
    GATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn this_month() -> String {
    chrono::Utc::now().format("%Y-%m").to_string()
}

impl Gate {
    fn book(&self, month: &str) -> Book {
        let read = std::fs::read(&self.file).ok().and_then(|b| serde_json::from_slice::<Book>(&b).ok());
        read.unwrap_or_default().of(month)
    }

    fn save(&self, book: &Book) {
        let bytes = serde_json::to_vec(book).unwrap_or_default();
        if let Err(e) = crate::state_fs::devices::write_atomic(&self.file, &bytes) {
            tracing::warn!("spend not saved: {e}");
        }
    }

    fn meter(&self, id: &str, month: &str) -> Option<Meter> {
        let cap = *self.caps.get(id)?;
        let used = self.book(month).providers.get(id).map_or(0.0, |a| a.usd);
        Some(Meter { used_usd: used, cap_usd: cap })
    }

    fn add(&self, id: &str, usd: f64, month: &str) {
        let mut book = self.book(month);
        book.providers.entry(id.to_string()).or_default().usd += usd;
        self.save(&book);
    }

    /// The meter once, when it first reaches the warning share this month.
    fn take_warning(&self, id: &str, month: &str) -> Option<Meter> {
        let meter = self.meter(id, month).filter(Meter::near)?;
        let mut book = self.book(month);
        let account = book.providers.entry(id.to_string()).or_default();
        if account.warned {
            return None;
        }
        account.warned = true;
        self.save(&book);
        Some(meter)
    }
}

/// The provider a model's spend counts under: a key provider billed per
/// token. Others have no cap here.
pub fn account_of(cfg: &ModelConfig) -> Option<String> {
    let counted = cfg.sign_in() == SignIn::Key && billing_of(cfg) == Billing::Metered;
    counted.then(|| provider_id(cfg.sign_in(), cfg.provider_name(), &cfg.url))
}

fn label_of(cfg: &ModelConfig) -> String {
    provider_source(cfg.sign_in(), cfg.provider_name(), &cfg.url).0
}

/// Why a provider may not be called now.
#[derive(Debug, Clone, PartialEq)]
pub struct Capped {
    pub provider: String,
    pub meter: Meter,
}

impl Capped {
    /// The refusal, ≤ 20 words: which provider and how to raise the cap.
    pub fn says(&self) -> String {
        format!(
            "{} reached its ${:.0} monthly cap. Raise it in Settings → Models to continue.",
            self.provider, self.meter.cap_usd
        )
    }
}

impl std::fmt::Display for Capped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.says())
    }
}

impl std::error::Error for Capped {}

/// THE decision: may this model's provider be called this month? `Ok` for a
/// provider with no cap, a subscription, Cloud or a local model.
pub fn check(cfg: &ModelConfig) -> Result<(), Capped> {
    let Some(id) = account_of(cfg) else { return Ok(()) };
    let meter = gate().meter(&id, &this_month());
    match meter.filter(Meter::is_capped) {
        Some(meter) => Err(Capped { provider: label_of(cfg), meter }),
        None => Ok(()),
    }
}

/// Add one call's estimated cost to its provider's month.
pub fn add(cfg: &ModelConfig, usd: f64) {
    if let Some(id) = account_of(cfg).filter(|_| usd > 0.0) {
        gate().add(&id, usd, &this_month());
    }
}

/// The one line for a provider that has just come near its cap, once a
/// month; `None` after that and below the share.
pub fn take_warning(cfg: &ModelConfig) -> Option<String> {
    let id = account_of(cfg)?;
    let meter = gate().take_warning(&id, &this_month())?;
    Some(format!(
        "{} has used about ${:.0} of its ${:.0} monthly cap (estimate).",
        label_of(cfg),
        meter.used_usd,
        meter.cap_usd
    ))
}

/// What a provider card shows: what was used, and the cap when there is one.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpendView {
    pub month: String,
    pub used_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap_usd: Option<f64>,
    /// The day the price table was true: the figure is an estimate by it.
    pub prices_as_of: String,
}

pub fn view(provider: &str) -> SpendView {
    let month = this_month();
    let gate = gate();
    SpendView {
        used_usd: gate.book(&month).providers.get(provider).map_or(0.0, |a| a.usd),
        cap_usd: gate.caps.get(provider).copied(),
        prices_as_of: super::curated::current().facts_observed_on().to_string(),
        month,
    }
}

/// Make these the caps in force — the config's, whenever it is read or saved
/// (`AgentManager`). Nothing else sets them.
pub fn use_caps(caps: &BTreeMap<String, f64>) {
    gate().caps = caps.clone();
}

/// A cap a person may set: a positive, finite amount of dollars.
pub fn valid_cap(usd: f64) -> bool {
    usd.is_finite() && usd > 0.0 && usd <= 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate_in(dir: &std::path::Path, id: &str, cap: f64) -> Gate {
        Gate { caps: BTreeMap::from([(id.to_string(), cap)]), file: dir.join("spend.json") }
    }

    #[test]
    fn the_meter_adds_calls_and_reports_the_share() {
        let dir = tempfile::tempdir().unwrap();
        let g = gate_in(dir.path(), "p", 200.0);
        g.add("p", 50.0, "2026-11");
        g.add("p", 10.5, "2026-11");
        let m = g.meter("p", "2026-11").unwrap();
        assert_eq!(m, Meter { used_usd: 60.5, cap_usd: 200.0 });
        assert!((m.share() - 0.3025).abs() < 1e-9);
        assert!(!m.is_capped());
        assert!(g.meter("other", "2026-11").is_none(), "no cap, no meter");
    }

    #[test]
    fn the_warning_is_given_once_a_month_at_eighty_percent() {
        let dir = tempfile::tempdir().unwrap();
        let g = gate_in(dir.path(), "p", 100.0);
        g.add("p", 79.0, "2026-11");
        assert!(g.take_warning("p", "2026-11").is_none(), "below 80%");
        g.add("p", 1.0, "2026-11");
        assert_eq!(g.take_warning("p", "2026-11").unwrap().used_usd, 80.0);
        g.add("p", 5.0, "2026-11");
        assert!(g.take_warning("p", "2026-11").is_none(), "only once");
        g.add("p", 85.0, "2026-12");
        assert!(g.take_warning("p", "2026-12").is_some(), "a new month warns again");
    }

    #[test]
    fn at_the_cap_the_provider_is_stopped_and_a_raised_cap_lets_it_go_on() {
        let dir = tempfile::tempdir().unwrap();
        let mut g = gate_in(dir.path(), "p", 100.0);
        g.add("p", 100.0, "2026-11");
        assert!(g.meter("p", "2026-11").unwrap().is_capped());
        assert!(g.take_warning("p", "2026-11").is_none(), "the refusal speaks at the cap");
        g.caps.insert("p".into(), 150.0);
        assert!(!g.meter("p", "2026-11").unwrap().is_capped());
    }

    #[test]
    fn a_new_month_starts_at_zero() {
        let dir = tempfile::tempdir().unwrap();
        let g = gate_in(dir.path(), "p", 100.0);
        g.add("p", 100.0, "2026-11");
        assert!(g.meter("p", "2026-11").unwrap().is_capped());
        assert_eq!(g.meter("p", "2026-12").unwrap().used_usd, 0.0);
        g.add("p", 1.0, "2026-12");
        assert_eq!(g.meter("p", "2026-12").unwrap().used_usd, 1.0);
        assert_eq!(g.meter("p", "2026-11").unwrap().used_usd, 0.0, "the old month is not kept");
    }

    #[test]
    fn only_a_key_provider_billed_per_token_has_a_cap() {
        let row = |v: serde_json::Value| -> ModelConfig { serde_json::from_value(v).unwrap() };
        let key = row(serde_json::json!({"id":"a","provider":"anthropic","url":"https://api.anthropic.com","model":"m","api_key":"k"}));
        let chatgpt = row(serde_json::json!({"id":"c","provider":"chatgpt","url":"","model":"m","auth_mode":"chatgpt_oauth"}));
        let local = row(serde_json::json!({"id":"l","provider":"openai","url":"http://127.0.0.1:1234/v1","model":"m"}));
        assert!(account_of(&key).is_some());
        assert!(account_of(&chatgpt).is_none());
        assert!(account_of(&local).is_none());
    }

    /// The lock: `check` is asked in one function of `ModelManager` that every
    /// backend call and the fallback chain pass through; the ledger writer
    /// alone adds spend. A second asker or writer fails here.
    #[test]
    fn one_place_asks_the_gate_and_one_adds_to_it() {
        fn files(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    files(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push((p.display().to_string(), std::fs::read_to_string(&p).unwrap()));
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut all = Vec::new();
        files(&root, &mut all);
        let hits = |needle: &str| -> Vec<(String, usize)> {
            all.iter()
                .filter(|(f, t)| !f.ends_with("spend_cap.rs") && !f.ends_with("spend_tests.rs") && t.contains(needle))
                .map(|(f, t)| (f.rsplit("src/").next().unwrap().to_string(), t.matches(needle).count()))
                .collect()
        };
        assert_eq!(hits("spend_cap::check("), vec![("provider/models/mod.rs".to_string(), 1)]);
        assert_eq!(hits("spend_cap::add("), vec![("engine/spend.rs".to_string(), 1)]);
        let models = &all.iter().find(|(f, _)| f.ends_with("provider/models/mod.rs")).unwrap().1;
        let backend_calls = models.matches(".text_stream(").count() + models.matches(".tool_stream(").count();
        assert_eq!(models.matches("self.spend_refusal(&instance.config)?").count(), backend_calls, "every backend call asks first");
    }
}
