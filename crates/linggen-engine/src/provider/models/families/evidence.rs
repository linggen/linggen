//! Access evidence: which model ids a provider lists for this endpoint and
//! this key.
//!
//! Fetched off the turn path and outside the settings write lock, kept per
//! endpoint and key revision, and dropped when the key changes. A list proves
//! visibility, not permission to call; it is the evidence this slice has, and
//! no paid probe call is made. A provider whose list cannot be read gives no
//! evidence, and then nothing moves on its account. Nothing here logs, stores
//! or returns a key: failures are one fixed value.

use crate::provider::models::curated::Place;
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a list is believed before it is asked for again.
const FRESH_FOR: Duration = Duration::from_secs(6 * 60 * 60);
const TIMEOUT: Duration = Duration::from_secs(10);
/// A list is a few hundred ids; a page that is much bigger is not one.
const MAX_PAGES: usize = 20;
const MAX_IDS: usize = 5000;

/// Where and as whom to ask. The address is the list's own for a row's address, never typed.
pub(crate) struct Target<'a> {
    pub url: &'a str,
    pub native_anthropic: bool,
    pub key: &'a str,
}

/// The list could not be read: no key, refused, unreachable, not a list.
#[derive(Debug)]
pub(crate) struct ListFailed;

/// The seam tests replace: the real one asks the provider.
#[async_trait]
pub(crate) trait ModelLister: Send + Sync {
    async fn list(&self, target: &Target<'_>) -> Result<BTreeSet<String>, ListFailed>;
}

/// What a provider listed for one key. `revision` is derived from the key
/// and says only whether the key is the same one.
#[derive(Debug)]
pub(crate) struct Listing {
    pub revision: u64,
    pub usable: BTreeSet<String>,
}

/// A key's revision: the same key, the same number; the key is not in it.
pub(crate) fn revision_of(key: &str) -> u64 {
    let digest = Sha256::digest(format!("key-revision:{key}"));
    u64::from_be_bytes(digest[..8].try_into().expect("eight bytes"))
}

struct Held {
    revision: u64,
    listing: Option<Arc<Listing>>,
    at: Instant,
}

/// One connection's slot: its key in the list and the exact address it is asked at.
fn held_by(place: &Place<'_>) -> String {
    format!("{}@{}", place.endpoint_key, crate::credentials::endpoint_url(place.base).to_ascii_lowercase())
}

pub(crate) struct Evidence {
    lister: Arc<dyn ModelLister>,
    fresh_for: Duration,
    /// By connection (see `held_by`: the address is part of it, so a list
    /// that moves an endpoint never lends a list gathered elsewhere). A new
    /// key replaces the entry.
    held: Mutex<HashMap<String, Held>>,
    gates: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Evidence {
    pub(crate) fn new(lister: Arc<dyn ModelLister>, fresh_for: Duration) -> Self {
        Self { lister, fresh_for, held: Default::default(), gates: Default::default() }
    }

    #[cfg_attr(feature = "test-support", allow(dead_code))]
    pub(crate) fn http() -> Self { Self::new(Arc::new(HttpLister::default()), FRESH_FOR) }

    /// A test build with no fixture: no provider is reachable, every ask fails.
    #[cfg(feature = "test-support")]
    pub(crate) fn unreachable() -> Self {
        Self::new(Arc::new(Unreachable), FRESH_FOR)
    }

    /// The lists of a fixture file (system tests): `{"lists": {"<endpoint key>":
    /// ["<model id>", …]}}`, read at each ask.
    #[cfg(feature = "test-support")]
    pub(crate) fn from_fixture(path: std::path::PathBuf) -> Self {
        Self::new(Arc::new(FileLister(path)), FRESH_FOR)
    }

    /// The list already held for this endpoint and key, if it is fresh. Never
    /// asks: what a page read and a decision under the lock use.
    pub(crate) fn held(&self, place: &Place<'_>, key: &str) -> Option<Arc<Listing>> {
        self.result(place, key).flatten()
    }

    /// None means unchecked; Some(None) means an attempt failed.
    pub(crate) fn result(&self, place: &Place<'_>, key: &str) -> Option<Option<Arc<Listing>>> {
        let revision = revision_of(key);
        let held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        let entry = held.get(&held_by(place))?;
        let ttl = if entry.listing.is_some() { self.fresh_for } else { Duration::from_secs(30) };
        (entry.revision == revision && entry.at.elapsed() <= ttl).then(|| entry.listing.clone())
    }

    /// Ask the provider unless a fresh list for this key is held. Call it
    /// outside the write lock.
    pub(crate) async fn gather(&self, place: &Place<'_>, key: &str) -> Option<Arc<Listing>> {
        let gate = self.gates.lock().unwrap_or_else(|e| e.into_inner())
            .entry(held_by(place)).or_default().clone();
        let _read = gate.lock().await;
        if let Some(result) = self.result(place, key) {
            return result;
        }
        let target = Target { url: place.base, native_anthropic: place.native_anthropic(), key };
        let revision = revision_of(key);
        let listing = self.lister.list(&target).await.ok().map(|usable| Arc::new(Listing { revision, usable }));
        self.held.lock().unwrap_or_else(|e| e.into_inner())
            .insert(held_by(place), Held { revision, listing: listing.clone(), at: Instant::now() });
        listing
    }
}

pub(crate) struct HttpLister {
    client: reqwest::Client,
}

impl Default for HttpLister {
    fn default() -> Self {
        // No redirect: a key header must not follow a provider's answer elsewhere.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        Self { client }
    }
}

#[async_trait]
impl ModelLister for HttpLister {
    async fn list(&self, target: &Target<'_>) -> Result<BTreeSet<String>, ListFailed> {
        let base = target.url.trim_end_matches('/');
        let mut ids = BTreeSet::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let request = if target.native_anthropic {
                let mut url = format!("{base}/v1/models?limit=1000");
                if let Some(cursor) = &after {
                    url.push_str(&format!("&after_id={}", urlencoding::encode(cursor)));
                }
                self.client.get(url).header("x-api-key", target.key).header("anthropic-version", "2023-06-01")
            } else {
                self.client.get(format!("{base}/models")).bearer_auth(target.key)
            };
            let response = request.timeout(TIMEOUT).send().await.map_err(|_| ListFailed)?;
            if !response.status().is_success() {
                return Err(ListFailed);
            }
            let body: serde_json::Value = response.json().await.map_err(|_| ListFailed)?;
            let (page, next) = read_page(&body, target.native_anthropic).ok_or(ListFailed)?;
            ids.extend(page);
            if ids.len() > MAX_IDS {
                return Err(ListFailed);
            }
            match next {
                Some(cursor) => after = Some(cursor),
                None => return Ok(ids),
            }
        }
        Err(ListFailed)
    }
}

/// The ids on one page, and the cursor of the next page if there is one.
/// Gemini's compatible list names models `models/<id>`.
fn read_page(body: &serde_json::Value, paged: bool) -> Option<(Vec<String>, Option<String>)> {
    let items = body.get("data")?.as_array()?;
    let ids = items.iter()
        .filter_map(|item| item.get("id")?.as_str())
        .map(|id| id.strip_prefix("models/").unwrap_or(id).to_string())
        .collect();
    let more = paged && body.get("has_more").and_then(serde_json::Value::as_bool).unwrap_or(false);
    let next = more.then(|| body.get("last_id")?.as_str().map(str::to_string)).flatten();
    Some((ids, next))
}

#[cfg(feature = "test-support")]
struct Unreachable;

#[cfg(feature = "test-support")]
#[async_trait]
impl ModelLister for Unreachable {
    async fn list(&self, _: &Target<'_>) -> Result<BTreeSet<String>, ListFailed> { Err(ListFailed) }
}

/// Answers from a file, by the list's endpoint key at the asked address: a system test's provider.
#[cfg(feature = "test-support")]
struct FileLister(std::path::PathBuf);

#[cfg(feature = "test-support")]
#[async_trait]
impl ModelLister for FileLister {
    async fn list(&self, target: &Target<'_>) -> Result<BTreeSet<String>, ListFailed> {
        let text = std::fs::read_to_string(&self.0).map_err(|_| ListFailed)?;
        let file: serde_json::Value = serde_json::from_str(&text).map_err(|_| ListFailed)?;
        if let Some(ms) = file.get("delay_ms").and_then(|v| v.as_u64()) {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
        let list = crate::provider::models::curated::current();
        let place = list.place_at(target.url).ok_or(ListFailed)?;
        let ids = file.get("lists").and_then(|l| l.get(place.endpoint_key)).and_then(|l| l.as_array()).ok_or(ListFailed)?;
        Ok(ids.iter().filter_map(|id| id.as_str().map(str::to_string)).collect())
    }
}

#[cfg(test)]
mod tests;
