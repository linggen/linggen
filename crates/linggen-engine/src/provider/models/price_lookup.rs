//! The price of a custom model (自定义, typed by name), asked of linggen.dev.
//!
//! `GET /api/model-price?provider=<address>&model=<id>` answers from the
//! public catalog the site keeps; the engine never asks models.dev and the
//! person never types a price. The answer is kept on the row
//! (`ModelConfig::price_lookup`) and `pricing::price_in` reads it — there is
//! no second price table. A model the site has no price for shows "no price,
//! not counted toward the cap" and is asked again once a day, off the turn's
//! path (`server/background.rs`). Tests never reach the site: they feed
//! [`parse_answer`] a body.

use crate::config::{ModelConfig, ModelPrice, PriceLookup, SignIn};
use serde::Deserialize;

const URL: &str = "https://linggen.dev/api/model-price";
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    found: bool,
    cost: Option<Cost>,
    release_date: Option<String>,
    observed_at: Option<String>,
}

#[derive(Deserialize)]
struct Cost {
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
}

/// What the site's body says, kept as of `today`. A missing or non-positive
/// input or output rate is no price (the same rule as the catalog's).
pub(crate) fn parse_answer(body: &[u8], today: &str) -> Option<PriceLookup> {
    let answer: Answer = serde_json::from_slice(body).ok()?;
    let usable = |n: Option<f64>| n.filter(|n| n.is_finite() && *n > 0.0);
    let price = answer.found.then_some(answer.cost).flatten().and_then(|c| {
        Some(ModelPrice {
            input: usable(c.input)?,
            output: usable(c.output)?,
            cache_read: usable(c.cache_read),
            cache_write: usable(c.cache_write),
        })
    });
    let as_of = price.as_ref().map(|_| answer.release_date.or(answer.observed_at).unwrap_or_else(|| today.to_string()));
    Some(PriceLookup { checked_on: today.to_string(), price, as_of })
}

/// Ask the site about one model at one address. `Err`: the site could not be
/// reached or answered badly — nothing is recorded, so it is asked again soon.
pub(crate) async fn ask(client: &reqwest::Client, base: &str, address: &str, model: &str, today: &str) -> anyhow::Result<PriceLookup> {
    let response = client.get(base).query(&[("provider", address), ("model", model)]).send().await?;
    anyhow::ensure!(response.status().is_success(), "price lookup failed");
    let body = response.bytes().await?;
    parse_answer(&body, today).ok_or_else(|| anyhow::anyhow!("price lookup unreadable"))
}

/// [`ask`] against the site.
pub async fn ask_site(address: &str, model: &str, today: &str) -> anyhow::Result<PriceLookup> {
    let client = reqwest::Client::builder().timeout(TIMEOUT).build()?;
    ask(&client, URL, address, model, today).await
}

/// A model the person added by name on a key: no family keeps it, no price
/// of theirs is on it.
fn is_custom(row: &ModelConfig) -> bool {
    !row.is_builtin && row.family_binding.is_none() && row.price.is_none() && row.sign_in() == SignIn::Key
}

/// Whether the row should be asked about now: never asked, or no price found
/// on an earlier day. A found price is kept; a bad row (no address) is not asked.
pub fn is_due(row: &ModelConfig, today: &str) -> bool {
    if !is_custom(row) || row.url.is_empty() {
        return false;
    }
    match &row.price_lookup {
        None => true,
        Some(lookup) => lookup.price.is_none() && lookup.checked_on != today,
    }
}

/// Rows to ask about, as (row id, address, model).
pub fn due_rows(models: &[ModelConfig], today: &str) -> Vec<(String, String, String)> {
    models.iter().filter(|r| is_due(r, today)).map(|r| (r.id.clone(), r.url.clone(), r.model.clone())).collect()
}

/// Keep an answer on the row, if it is still there and still the same model.
pub fn keep(models: &mut [ModelConfig], id: &str, model: &str, lookup: PriceLookup) -> bool {
    let Some(row) = models.iter_mut().find(|r| r.id == id && r.model == model && is_custom(r)) else { return false };
    row.price_lookup = Some(lookup);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> ModelConfig {
        serde_json::from_value(serde_json::json!({"id": "own-1", "provider": "anthropic", "url": "https://api.anthropic.com", "model": "claude-old-1"})).unwrap()
    }

    #[test]
    fn a_found_price_is_kept_with_its_day() {
        let body = br#"{"found":true,"cost":{"input":3,"output":15,"cache_read":0.3},"release_date":"2025-05-01","observed_at":"2026-11-20"}"#;
        let got = parse_answer(body, "2026-11-26").unwrap();
        let price = got.price.unwrap();
        assert_eq!((price.input, price.output, price.cache_read, price.cache_write), (3.0, 15.0, Some(0.3), None));
        assert_eq!((got.as_of.as_deref(), got.checked_on.as_str()), (Some("2025-05-01"), "2026-11-26"));
    }

    #[test]
    fn not_found_or_a_zero_rate_is_no_price() {
        for body in [&br#"{"found":false,"observed_at":"2026-11-20"}"#[..], br#"{"found":true,"cost":{"input":0,"output":5}}"#, br#"{"found":true,"cost":{"output":5}}"#] {
            let got = parse_answer(body, "2026-11-26").unwrap();
            assert!(got.price.is_none() && got.as_of.is_none());
        }
        assert!(parse_answer(b"<html>", "2026-11-26").is_none());
    }

    #[test]
    fn a_new_custom_row_is_due_and_no_price_is_asked_again_the_next_day_only() {
        let mut r = row();
        assert!(is_due(&r, "2026-11-26"));
        r.price_lookup = Some(PriceLookup { checked_on: "2026-11-26".into(), price: None, as_of: None });
        assert!(!is_due(&r, "2026-11-26") && is_due(&r, "2026-11-27"));
        r.price_lookup = parse_answer(br#"{"found":true,"cost":{"input":1,"output":2}}"#, "2026-11-26");
        assert!(!is_due(&r, "2026-12-30"), "a found price is kept");
    }

    #[test]
    fn a_typed_price_or_a_family_row_is_never_asked() {
        let mut r = row();
        r.price = Some(ModelPrice { input: 1.0, output: 2.0, cache_read: None, cache_write: None });
        assert!(!is_due(&r, "2026-11-26"));
    }

    #[test]
    fn the_found_price_counts_through_the_one_price_decision() {
        let mut r = row();
        let mut models = vec![r.clone()];
        let lookup = parse_answer(br#"{"found":true,"cost":{"input":1,"output":2},"observed_at":"2026-11-20"}"#, "2026-11-26").unwrap();
        assert!(keep(&mut models, "own-1", "claude-old-1", lookup));
        r = models.remove(0);
        let priced = crate::provider::models::price_of(&r).unwrap();
        assert_eq!(priced.base_rates().output, 2.0);
        assert!(!keep(&mut [r.clone()], "own-1", "another-model", PriceLookup { checked_on: String::new(), price: None, as_of: None }));
    }
}
