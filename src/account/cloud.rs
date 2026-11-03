//! The account's skill cloud on linggen.dev — the saves and meters a skill
//! declares (`CloudConfig`). The engine does all the talking for a skill; a
//! skill bundle never calls out. Both take the account token, like the LLM
//! proxy — see linggensite `saves.ts` and `meters.ts`.

use crate::util::LockExt;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::{http, resolve_token, site_url};

/// A meter's window as the site last reported it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeterReading {
    pub meter: String,
    pub size: u64,
    pub used: u64,
    pub left: u64,
    pub window_hours: f64,
    /// Unix seconds when a spent window frees up; None while some is left.
    pub refill_at: Option<u64>,
}

/// How long a reading answers "what is left?" before the site is asked again.
/// Every report refreshes it, so a session in play rarely asks.
const READING_TTL: Duration = Duration::from_secs(30);

static READINGS: Mutex<Option<HashMap<String, (MeterReading, Instant)>>> = Mutex::new(None);

fn token() -> Result<String> {
    resolve_token().map(|(t, _)| t).context("signed out")
}

fn remember(r: &MeterReading) {
    let mut guard = READINGS.lock_ok();
    guard
        .get_or_insert_with(HashMap::new)
        .insert(r.meter.clone(), (r.clone(), Instant::now()));
}

fn cached(meter: &str) -> Option<(MeterReading, Instant)> {
    READINGS.lock_ok().as_ref()?.get(meter).cloned()
}

/// The last reading of `meter` this machine holds, however old.
pub fn last_reading(meter: &str) -> Option<MeterReading> {
    cached(meter).map(|(r, _)| r)
}

/// A name the site takes for a save or a meter — and the only kind the engine
/// puts into a URL or a ledger file name: `[a-z][a-z0-9-]{0,39}`, as
/// linggensite's `saves.ts` checks it.
pub fn valid_cloud_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('a'..='z'))
        && name.len() <= 40
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
}

fn named(name: &str) -> Result<&str> {
    if !valid_cloud_name(name) {
        bail!("'{name}' is not a cloud name ([a-z0-9-])");
    }
    Ok(name)
}

fn meter_url(meter: &str) -> Result<String> {
    Ok(format!("{}/api/meters/{}", site_url(), named(meter)?))
}

/// The window's state, from the cache while it is fresh.
pub async fn meter_reading(meter: &str) -> Result<MeterReading> {
    if let Some((r, at)) = cached(meter) {
        if at.elapsed() < READING_TTL {
            return Ok(r);
        }
    }
    let resp = http()
        .get(meter_url(meter)?)
        .bearer_auth(token()?)
        .send()
        .await
        .context("connect to linggen.dev")?;
    parse_reading(resp).await
}

/// Report a turn's tokens; the answer is the window after it.
pub async fn meter_report(meter: &str, tokens: u64) -> Result<MeterReading> {
    let resp = http()
        .post(meter_url(meter)?)
        .bearer_auth(token()?)
        .json(&serde_json::json!({ "tokens": tokens }))
        .send()
        .await
        .context("connect to linggen.dev")?;
    parse_reading(resp).await
}

async fn parse_reading(resp: reqwest::Response) -> Result<MeterReading> {
    if !resp.status().is_success() {
        bail!("meter: {}", resp.status());
    }
    let reading: MeterReading = resp.json().await.context("meter reading")?;
    remember(&reading);
    Ok(reading)
}

/// The account's copy of a save. Version 0 with no body: never saved. The
/// body is what the engine wrote: a file's text as a JSON string, or a bundle
/// of files (`cloud_save.rs`).
#[derive(Debug, Deserialize)]
pub struct CloudSave {
    pub version: u64,
    #[serde(default)]
    pub body: Option<serde_json::Value>,
}

pub enum PutOutcome {
    Saved(u64),
    /// Another device wrote since this one read.
    Stale,
    /// The account keeps no cloud save (not subscribed).
    Refused(Refusal),
}

/// linggen.dev's 402 `not_subscribed`: saves are for subscribers. `kept`: a
/// copy from an earlier subscription is still there, last written `updated_at`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Refusal {
    #[serde(default)]
    pub kept: bool,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// What a read of the account's save found.
pub enum Fetched {
    Save(CloudSave),
    Refused(Refusal),
}

fn refusal(status: reqwest::StatusCode, body: &serde_json::Value) -> Option<Refusal> {
    let not_subscribed = body.get("error").and_then(|e| e.as_str()) == Some("not_subscribed");
    if status != reqwest::StatusCode::PAYMENT_REQUIRED || !not_subscribed {
        return None;
    }
    Some(serde_json::from_value(body.clone()).unwrap_or_default())
}

fn save_url(name: &str) -> Result<String> {
    Ok(format!("{}/api/saves/{}", site_url(), named(name)?))
}

pub async fn save_get(name: &str) -> Result<Fetched> {
    let resp = http()
        .get(save_url(name)?)
        .bearer_auth(token()?)
        .send()
        .await
        .context("connect to linggen.dev")?;
    let status = resp.status();
    if status.is_success() {
        return Ok(Fetched::Save(resp.json().await.context("cloud save")?));
    }
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    match refusal(status, &body) {
        Some(r) => Ok(Fetched::Refused(r)),
        None => bail!("save: {status}"),
    }
}

/// Write `body` as the version after `read_version`.
pub async fn save_put(
    name: &str,
    read_version: u64,
    body: &serde_json::Value,
) -> Result<PutOutcome> {
    let resp = http()
        .put(save_url(name)?)
        .bearer_auth(token()?)
        .json(&serde_json::json!({ "version": read_version, "body": body }))
        .send()
        .await
        .context("connect to linggen.dev")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    let version = body.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
    if status.is_success() {
        return Ok(PutOutcome::Saved(version));
    }
    if status == reqwest::StatusCode::CONFLICT {
        return Ok(PutOutcome::Stale);
    }
    match refusal(status, &body) {
        Some(r) => Ok(PutOutcome::Refused(r)),
        None => bail!("save: {status}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{refusal, valid_cloud_name};
    use reqwest::StatusCode;

    #[test]
    fn only_a_402_not_subscribed_is_a_refusal() {
        let body = serde_json::json!({"error": "not_subscribed", "kept": true, "updated_at": "2026-11-03T10:00:00Z"});
        let r = refusal(StatusCode::PAYMENT_REQUIRED, &body).unwrap();
        assert!(r.kept);
        assert_eq!(r.updated_at.as_deref(), Some("2026-11-03T10:00:00Z"));
        assert!(refusal(
            StatusCode::PAYMENT_REQUIRED,
            &serde_json::json!({"error": "x"})
        )
        .is_none());
        assert!(refusal(StatusCode::FORBIDDEN, &body).is_none());
    }

    #[test]
    fn only_site_shaped_names_reach_a_url() {
        for ok in ["game", "a", "my-game-2"] {
            assert!(valid_cloud_name(ok), "{ok}");
        }
        for bad in ["", "Game", "../x", "a/b", "2game", "a_b", &"a".repeat(41)] {
            assert!(!valid_cloud_name(bad), "{bad}");
        }
    }
}
