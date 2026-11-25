//! The last good list: shipped copy, disk cache, and (off) refresh.
//!
//! The transport and the ETag rule are in `transport`. No
//! signature is read or fetched (the owner's decision, 2026-11-24: the list
//! travels over HTTPS). A list that fails the check, or is older than the last
//! good one, is refused as a unit and the last good copy stays.

use super::bundle::{accept, Bundle};
use super::transport::{valid_etag, Reply, Transport, CHECK_EVERY_DAYS, MAX_ETAG};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use std::{io::{self, Read, Write}, path::{Path, PathBuf}, sync::{Arc, LazyLock, OnceLock, RwLock}};

const SHIPPED: &[u8] = include_bytes!("model-catalog-v2.json");
const URL: &str = "https://linggen.dev/model-catalog/v2.json";
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Whether this engine asks the site for a newer list. Off until the owner
/// says so (the static file is not deployed yet).
const PUBLISHED: bool = false;
// The endpoint rule: the list is fetched over HTTPS only.
const _: () = assert!(URL.as_bytes().len() > 8 && matches!(URL.as_bytes(), [b'h', b't', b't', b'p', b's', b':', b'/', b'/', ..]), "the list is fetched over HTTPS");

type View = Arc<Bundle>;
static CURRENT: LazyLock<RwLock<Option<View>>> = LazyLock::new(|| RwLock::new(None));

/// The copy compiled in. It passes the same check a fetched one does.
pub(crate) fn shipped() -> &'static View {
    static SEED: OnceLock<View> = OnceLock::new();
    SEED.get_or_init(|| {
        let today = Utc::now().date_naive();
        Arc::new(accept(SHIPPED, &[], today).expect("shipped curated list"))
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
            let mut cache = Cache::open(path, shipped().clone(), Utc::now());
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

/// What one refresh brings: nothing new, or a body.
pub(super) enum Fetched {
    Unchanged,
    Body { bytes: Vec<u8>, etag: Option<String> },
}

/// The disk object holds the check time, ETag and the exact bytes together,
/// so a crash cannot pair a validator with another body.
pub(super) struct Cache {
    path: PathBuf,
    /// The compiled-in list: a floor no refresh may go under.
    seed: View,
    pub(super) bundle: Option<View>,
    etag: Option<String>,
    checked: Option<DateTime<Utc>>,
    retry_at: Option<DateTime<Utc>>,
    failures: u32,
}

fn read(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let limit = super::bundle::MAX_BYTES + MAX_ETAG + 64;
    std::fs::File::open(path)?.take(limit as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// `checked \n etag \n body`.
fn unpack(bytes: &[u8]) -> Option<(DateTime<Utc>, Option<String>, &[u8])> {
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
    Some((checked, etag, rest))
}

impl Cache {
    pub(super) fn open(path: PathBuf, seed: View, now: DateTime<Utc>) -> Self {
        let mut cache = Self { path, seed, bundle: None, etag: None, checked: None, retry_at: None, failures: 0 };
        let Ok(bytes) = read(&cache.path) else { return cache };
        let Some((checked, etag, body)) = unpack(&bytes) else { return cache };
        let Ok(good) = accept(body, &[&cache.seed], now.date_naive()) else { return cache };
        cache.bundle = Some(Arc::new(good));
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

    fn keep(&self, now: DateTime<Utc>, etag: Option<&str>, body: &[u8]) -> io::Result<()> {
        let parent = self.path.parent().ok_or(io::ErrorKind::InvalidInput)?;
        std::fs::create_dir_all(parent)?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        writeln!(tmp, "{}\n{}", now.to_rfc3339(), etag.unwrap_or(""))?;
        tmp.write_all(body)?;
        tmp.as_file().sync_all()?;
        tmp.persist(&self.path)?;
        Ok(())
    }

    /// A 304 changes the check time only.
    pub(super) fn apply(&mut self, now: DateTime<Utc>, fetched: Fetched) -> Result<Option<View>> {
        let next = match fetched {
            Fetched::Unchanged => {
                let good = self.bundle.clone().filter(|_| self.etag.is_some()).ok_or_else(|| anyhow::anyhow!("unsolicited 304"))?;
                self.keep(now, self.etag.as_deref(), good.raw())?;
                None
            }
            Fetched::Body { bytes, etag } => {
                let baseline = self.bundle.clone().unwrap_or_else(|| self.seed.clone());
                let good = Arc::new(accept(&bytes, &[&self.seed, &baseline], now.date_naive())?);
                let etag = etag.filter(|tag| valid_etag(tag));
                self.keep(now, etag.as_deref(), &bytes)?;
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

    async fn fetch(&self, fetch: &impl Transport) -> Result<Fetched> {
        match fetch.get(URL, self.etag.as_deref(), super::bundle::MAX_BYTES).await? {
            Reply::Unchanged => Ok(Fetched::Unchanged),
            Reply::Body { bytes, etag } => Ok(Fetched::Body { bytes, etag }),
        }
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
