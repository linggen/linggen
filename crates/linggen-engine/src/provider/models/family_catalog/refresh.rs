//! Refresh runs away from turns and never requires a credential.

use super::{accept, shipped, Snapshot, MAX_BYTES};
use chrono::{DateTime, Duration, Utc};
use std::{io::{self, Read, Write}, path::{Path, PathBuf}, sync::{Arc, LazyLock, RwLock}};

const URL: &str = "https://linggen.dev/model-catalog/v1.json";
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const MAX_ETAG: usize = 512;
/// The served catalog is regenerated weekly; asking more often finds nothing new.
pub(super) const CHECK_EVERY_DAYS: i64 = 7;

type View = Arc<Snapshot>;
static CURRENT: LazyLock<RwLock<Option<View>>> = LazyLock::new(|| RwLock::new(None));

pub(crate) fn current() -> Option<View> {
    CURRENT.read().expect("public catalog lock").clone()
}

fn publish(snapshot: View) {
    *CURRENT.write().expect("public catalog lock") = Some(snapshot);
}

/// Load the last good copy off the startup path. No outside request is
/// enabled until the static site asset is deployed and approved.
pub(crate) fn start() {
    const PUBLISHED: bool = false;
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let path = crate::paths::linggen_home().join("cache/model-catalog-v1");
        std::thread::spawn(move || {
            let mut cache = Cache::open(path, Utc::now());
            if let Some(snapshot) = cache.snapshot.clone() {
                publish(snapshot);
            }
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
                    if let Some(snapshot) = cache.refresh(Utc::now(), &client).await { publish(snapshot); }
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                }
            });
        });
    });
}

impl Transport for reqwest::Client {
    async fn get(&self, url: &str, etag: Option<&str>, limit: usize) -> anyhow::Result<Reply> {
        let mut request = reqwest::Client::get(self, url);
        if let Some(tag) = etag { request = request.header(reqwest::header::IF_NONE_MATCH, tag); }
        let mut response = request.send().await?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED { return Ok(Reply::Unchanged); }
        anyhow::ensure!(response.status() == reqwest::StatusCode::OK, "catalog not available");
        let etag = response.headers().get(reqwest::header::ETAG)
            .and_then(|tag| tag.to_str().ok()).map(str::to_string);
        anyhow::ensure!(response.content_length().is_none_or(|n| n <= limit as u64), "oversized catalog");
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(chunk.len() <= limit - bytes.len(), "oversized catalog");
            bytes.extend_from_slice(&chunk);
        }
        Ok(Reply::Body { bytes, etag })
    }
}

// The atomic disk object keeps the ETag/check time together with the exact
// public bytes, so a crash cannot pair a validator with a different body.
struct Cache {
    path: PathBuf,
    snapshot: Option<View>,
    body: Vec<u8>,
    etag: Option<String>,
    checked: Option<DateTime<Utc>>,
    retry_at: Option<DateTime<Utc>>,
    failures: u32,
}

fn read(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take((MAX_BYTES + MAX_ETAG + 81) as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn valid_etag(tag: &str) -> bool {
    let quoted = tag.strip_prefix("W/").unwrap_or(tag);
    tag.len() <= MAX_ETAG && quoted.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
        .is_some_and(|s| s.bytes().all(|b| b == 0x21 || (0x23..=0x7e).contains(&b)))
}

fn unpack(bytes: &[u8]) -> Option<(DateTime<Utc>, Option<String>, &[u8])> {
    let first = bytes.iter().position(|&b| b == b'\n')?;
    let rest = &bytes[first + 1..];
    let second = rest.iter().position(|&b| b == b'\n')?;
    let checked = std::str::from_utf8(&bytes[..first]).ok()?.parse().ok()?;
    let etag = std::str::from_utf8(&rest[..second]).ok()?;
    let etag = match etag {
        "" => None,
        tag if valid_etag(tag) => Some(tag.to_string()),
        _ => return None,
    };
    Some((checked, etag, &rest[second + 1..]))
}

impl Cache {
    fn open(path: PathBuf, now: DateTime<Utc>) -> Self {
        let mut cache = Self { path, snapshot: None, body: Vec::new(), etag: None, checked: None, retry_at: None, failures: 0 };
        if let Ok(bytes) = read(&cache.path) {
            if let Some((checked, etag, body)) = unpack(&bytes) {
                if let Ok(snapshot) = accept(body, shipped(), now.date_naive()) {
                    cache.snapshot = Some(Arc::new(snapshot));
                    cache.body = body.to_vec();
                    cache.etag = etag;
                    cache.checked = Some(checked.min(now));
                }
            }
        }
        cache
    }

    fn due(&self, now: DateTime<Utc>) -> bool {
        self.retry_at.map_or_else(
            || self.checked.is_none_or(|last| now >= last + Duration::days(CHECK_EVERY_DAYS)),
            |retry| now >= retry,
        )
    }

    fn keep(&self, now: DateTime<Utc>, tag: Option<&str>, body: &[u8]) -> io::Result<()> {
        let parent = self.path.parent().ok_or(io::ErrorKind::InvalidInput)?;
        std::fs::create_dir_all(parent)?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        writeln!(tmp, "{}\n{}", now.to_rfc3339(), tag.unwrap_or(""))?;
        tmp.write_all(body)?;
        tmp.as_file().sync_all()?;
        tmp.persist(&self.path)?;
        Ok(())
    }

    fn apply(&mut self, now: DateTime<Utc>, response: Reply) -> anyhow::Result<Option<View>> {
        let next = match response {
            Reply::Unchanged if self.etag.is_some() && self.snapshot.is_some() => {
                self.keep(now, self.etag.as_deref(), &self.body)?;
                None
            }
            Reply::Unchanged => anyhow::bail!("unsolicited 304"),
            Reply::Body { bytes, etag } => {
                let snapshot = Arc::new(accept(&bytes, self.snapshot.as_deref().unwrap_or_else(|| shipped()), now.date_naive())?);
                let etag = etag.filter(|tag| valid_etag(tag));
                self.keep(now, etag.as_deref(), &bytes)?;
                self.body = bytes;
                self.etag = etag;
                self.snapshot = Some(snapshot.clone());
                Some(snapshot)
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

    async fn refresh(&mut self, now: DateTime<Utc>, fetch: &impl Transport) -> Option<View> {
        if !self.due(now) {
            return None;
        }
        let answer = tokio::time::timeout(TIMEOUT, fetch.get(URL, self.etag.as_deref(), MAX_BYTES)).await;
        let applied = answer.ok().and_then(Result::ok).and_then(|reply| self.apply(now, reply).ok());
        match applied {
            Some(next) => next,
            None => { self.failed(now); None }
        }
    }
}

pub(super) enum Reply {
    Unchanged,
    Body { bytes: Vec<u8>, etag: Option<String> },
}

/// No credentials or configurable destination. A transport must cap the
/// streamed bytes at `limit` and report non-200/304 responses as errors.
pub(super) trait Transport {
    async fn get(&self, url: &str, etag: Option<&str>, limit: usize) -> anyhow::Result<Reply>;
}

#[cfg(test)]
mod tests;
