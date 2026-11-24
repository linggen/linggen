//! The last good list: shipped copy, disk cache, and (off) refresh.
//!
//! Same shape as the v1 refresh in `family_catalog/refresh.rs`, which this
//! replaces when the settings page moves to the curated list (slice 2); the
//! transport and the ETag rule are shared, not copied. A list and its detached
//! signature are fetched, checked and stored as one unit: a body paired with
//! the wrong signature fails the check and the last good copy stays.

use super::bundle::{accept, Bundle};
use super::trust::{OWNER_KEY_IS_PLACEHOLDER, OWNER_PUBLIC_KEY};
use super::super::family_catalog::{valid_etag, Reply, Transport, CHECK_EVERY_DAYS};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use std::{io::{self, Read, Write}, path::{Path, PathBuf}, sync::{Arc, LazyLock, OnceLock, RwLock}};

const SHIPPED: &[u8] = include_bytes!("model-catalog-v2.json");
const SHIPPED_SIGNATURE: &str = include_str!("model-catalog-v2.json.sig");
const URL: &str = "https://linggen.dev/model-catalog/v2.json";
const SIGNATURE_URL: &str = "https://linggen.dev/model-catalog/v2.json.sig";
const MAX_SIGNATURE: usize = 256;
const MAX_ETAG: usize = 512;
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Whether this engine asks the site for a newer list. Off, as the v1
/// catalog is, until the static files are deployed and approved.
const PUBLISHED: bool = false;
// Refresh must not be turned on while the list is trusted from the throwaway key.
const _: () = assert!(!(PUBLISHED && OWNER_KEY_IS_PLACEHOLDER), "replace the placeholder owner key first");

type View = Arc<Bundle>;
static CURRENT: LazyLock<RwLock<Option<View>>> = LazyLock::new(|| RwLock::new(None));

/// The copy compiled in. It passes the same check a fetched one does.
pub(crate) fn shipped() -> &'static View {
    static SEED: OnceLock<View> = OnceLock::new();
    SEED.get_or_init(|| {
        let today = Utc::now().date_naive();
        Arc::new(accept(SHIPPED, SHIPPED_SIGNATURE, &OWNER_PUBLIC_KEY, &[], today).expect("shipped curated list"))
    })
}

/// Capture once at a decision boundary. Never performs I/O.
pub(crate) fn current() -> View {
    CURRENT.read().expect("curated list lock").clone().unwrap_or_else(|| shipped().clone())
}

fn publish(bundle: View) {
    *CURRENT.write().expect("curated list lock") = Some(bundle);
}

/// Load the last good copy off the startup path; ask for a newer one only
/// when [`PUBLISHED`].
pub(crate) fn start() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let path = crate::paths::linggen_home().join("cache/model-catalog-v2");
        std::thread::spawn(move || {
            let mut cache = Cache::open(path, &OWNER_PUBLIC_KEY, shipped().clone(), Utc::now());
            if let Some(bundle) = cache.bundle.clone() { publish(bundle); }
            let list = current();
            tracing::info!(revision = list.revision(), list_revision = list.list_revision(), "curated model list loaded");
            if !PUBLISHED || cfg!(any(test, feature = "test-support"))
                || std::env::var_os("LINGGEN_TEST_SKIPPED").is_some()
            {
                return;
            }
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
            let Ok(client) = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none()).timeout(TIMEOUT).build() else { return };
            runtime.block_on(async move {
                loop {
                    if let Some(bundle) = cache.refresh(Utc::now(), &client).await { publish(bundle); }
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                }
            });
        });
    });
}

/// What one refresh brings: nothing new, or a body with its signature.
pub(super) enum Fetched {
    Unchanged,
    Body { bytes: Vec<u8>, signature: String, etag: Option<String> },
}

/// The disk object holds the check time, ETag, signature and the exact bytes
/// together, so a crash cannot pair a validator or signature with another body.
pub(super) struct Cache {
    path: PathBuf,
    key: [u8; 32],
    /// The compiled-in list: a floor no refresh may go under.
    seed: View,
    pub(super) bundle: Option<View>,
    signature: String,
    etag: Option<String>,
    checked: Option<DateTime<Utc>>,
    retry_at: Option<DateTime<Utc>>,
    failures: u32,
}

fn read(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let limit = super::bundle::MAX_BYTES + MAX_ETAG + MAX_SIGNATURE + 81;
    std::fs::File::open(path)?.take(limit as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// `checked \n etag \n signature \n body`.
fn unpack(bytes: &[u8]) -> Option<(DateTime<Utc>, Option<String>, &str, &[u8])> {
    let mut rest = bytes;
    let mut line = || {
        let end = rest.iter().position(|&b| b == b'\n')?;
        let (head, tail) = rest.split_at(end);
        rest = &tail[1..];
        std::str::from_utf8(head).ok()
    };
    let checked = line()?.parse().ok()?;
    let etag = match line()? {
        "" => None,
        tag if valid_etag(tag) => Some(tag.to_string()),
        _ => return None,
    };
    let signature = line()?;
    Some((checked, etag, signature, rest))
}

impl Cache {
    pub(super) fn open(path: PathBuf, key: &[u8; 32], seed: View, now: DateTime<Utc>) -> Self {
        let mut cache = Self { path, key: *key, seed, bundle: None, signature: String::new(), etag: None, checked: None, retry_at: None, failures: 0 };
        let Ok(bytes) = read(&cache.path) else { return cache };
        let Some((checked, etag, signature, body)) = unpack(&bytes) else { return cache };
        let Ok(good) = accept(body, signature, key, &[&cache.seed], now.date_naive()) else { return cache };
        cache.bundle = Some(Arc::new(good));
        cache.signature = signature.to_string();
        cache.etag = etag;
        cache.checked = Some(checked.min(now));
        cache
    }

    pub(super) fn due(&self, now: DateTime<Utc>) -> bool {
        self.retry_at.map_or_else(
            || self.checked.is_none_or(|last| now >= last + Duration::days(CHECK_EVERY_DAYS)),
            |retry| now >= retry,
        )
    }

    fn keep(&self, now: DateTime<Utc>, etag: Option<&str>, signature: &str, body: &[u8]) -> io::Result<()> {
        let parent = self.path.parent().ok_or(io::ErrorKind::InvalidInput)?;
        std::fs::create_dir_all(parent)?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        writeln!(tmp, "{}\n{}\n{}", now.to_rfc3339(), etag.unwrap_or(""), signature.trim())?;
        tmp.write_all(body)?;
        tmp.as_file().sync_all()?;
        tmp.persist(&self.path)?;
        Ok(())
    }

    /// A 304 changes neither the list nor its signature, only the check time.
    pub(super) fn apply(&mut self, now: DateTime<Utc>, fetched: Fetched) -> Result<Option<View>> {
        let next = match fetched {
            Fetched::Unchanged => {
                let good = self.bundle.clone().filter(|_| self.etag.is_some()).ok_or_else(|| anyhow::anyhow!("unsolicited 304"))?;
                self.keep(now, self.etag.as_deref(), &self.signature, good.raw())?;
                None
            }
            Fetched::Body { bytes, signature, etag } => {
                let baseline = self.bundle.clone().unwrap_or_else(|| self.seed.clone());
                let good = Arc::new(accept(&bytes, &signature, &self.key, &[&self.seed, &baseline], now.date_naive())?);
                let etag = etag.filter(|tag| valid_etag(tag));
                self.keep(now, etag.as_deref(), &signature, &bytes)?;
                self.signature = signature;
                self.etag = etag;
                self.bundle = Some(good.clone());
                Some(good)
            }
        };
        self.checked = Some(now);
        self.failures = 0;
        self.retry_at = None;
        Ok(next)
    }

    fn failed(&mut self, now: DateTime<Utc>) {
        self.failures = self.failures.saturating_add(1);
        let minutes = (15 * (1 << (self.failures - 1).min(7))).min(24 * 60);
        self.retry_at = Some(now + Duration::minutes(minutes));
    }

    /// The list, then its signature: the pair is checked together or dropped together.
    async fn fetch(&self, fetch: &impl Transport) -> Result<Fetched> {
        let bytes = match fetch.get(URL, self.etag.as_deref(), super::bundle::MAX_BYTES).await? {
            Reply::Unchanged => return Ok(Fetched::Unchanged),
            Reply::Body { bytes, etag } => (bytes, etag),
        };
        let Reply::Body { bytes: signature, .. } = fetch.get(SIGNATURE_URL, None, MAX_SIGNATURE).await? else {
            anyhow::bail!("signature not available");
        };
        Ok(Fetched::Body { bytes: bytes.0, signature: String::from_utf8(signature)?, etag: bytes.1 })
    }

    pub(super) async fn refresh(&mut self, now: DateTime<Utc>, fetch: &impl Transport) -> Option<View> {
        if !self.due(now) { return None; }
        let answer = tokio::time::timeout(TIMEOUT, self.fetch(fetch)).await;
        let applied = answer.ok().and_then(Result::ok).and_then(|fetched| self.apply(now, fetched).ok());
        match applied {
            Some(next) => next,
            None => { self.failed(now); None }
        }
    }
}
