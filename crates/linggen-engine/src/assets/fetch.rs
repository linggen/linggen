//! The background downloader: takes the queue's next file, at most
//! `Tuning::concurrency` at a time, checks it, and puts it in the store.

use super::queue::{Failure, Want};
use super::Assets;
use crate::util::LockExt;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// A download that gets no bytes for this long has failed.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("linggen")
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .unwrap_or_default()
}

/// What happened since the last line in the log.
#[derive(Default)]
pub struct Tally {
    fetched: u32,
    bytes: u64,
    failed: u32,
    last_error: Option<String>,
}

struct FetchError {
    failure: Failure,
    why: String,
}

impl Assets {
    /// Start the downloader. It runs as long as the engine does and never
    /// blocks a request.
    pub fn spawn_worker(self: &Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move { this.run().await });
    }

    async fn run(self: Arc<Self>) {
        let slots = Arc::new(Semaphore::new(self.tuning.concurrency));
        loop {
            let Ok(slot) = slots.clone().acquire_owned().await else {
                return;
            };
            let next = self.state.lock_ok().queue.next(Instant::now());
            let Some(want) = next else {
                drop(slot);
                self.rest().await;
                continue;
            };
            let this = self.clone();
            tokio::spawn(async move {
                this.download(want).await;
                drop(slot);
                this.wake.notify_one();
            });
        }
    }

    /// Nothing can start now: say what the last burst did, then wait for
    /// the queue to change or for the next retry to come due.
    async fn rest(&self) {
        self.log_burst();
        let due = self.state.lock_ok().queue.wake_at(Instant::now());
        let changed = self.wake.notified();
        match due {
            Some(at) => {
                let at = tokio::time::Instant::from_std(at);
                tokio::select! { _ = changed => {}, _ = tokio::time::sleep_until(at) => {} }
            }
            None => changed.await,
        }
    }

    async fn download(&self, want: Want) {
        let outcome = fetch_verified(&self.client, &self.store, &want).await;
        let object = want.file.object();
        let mut state = self.state.lock_ok();
        match outcome {
            Ok(()) => {
                state.queue.succeeded(&object);
                state.tally.fetched += 1;
                state.tally.bytes += want.file.size;
            }
            Err(e) => {
                state
                    .queue
                    .failed(&object, e.failure, Instant::now(), &self.tuning);
                state.tally.failed += 1;
                state.tally.last_error = Some(e.why);
            }
        }
    }

    /// A line when the picture changes — the first failure, the recovery,
    /// a burst that fetched — never one per file or per retry.
    fn log_burst(&self) {
        let mut state = self.state.lock_ok();
        if state.queue.in_flight() > 0 {
            return;
        }
        let tally = std::mem::take(&mut state.tally);
        let was_failing = std::mem::replace(&mut state.failing, tally.last_error.is_some());
        match tally.last_error {
            Some(why) if !was_failing => tracing::warn!(
                "[assets] fetched {}, failed {} (last: {why}); retrying with growing pauses",
                tally.fetched,
                tally.failed
            ),
            Some(_) => {}
            None if tally.fetched > 0 => tracing::info!(
                "[assets] fetched {} file(s), {} bytes",
                tally.fetched,
                tally.bytes
            ),
            None => {}
        }
    }
}

/// Download `want` to a temp file, check size and hash, rename into place.
/// A file that does not match is thrown away.
async fn fetch_verified(
    client: &reqwest::Client,
    store: &super::Store,
    want: &Want,
) -> Result<(), FetchError> {
    store.ensure_dir().map_err(local)?;
    let temp = store.temp_of(&want.file);
    let result = download_to(client, want, &temp).await;
    let result = match result {
        Ok(()) => std::fs::rename(&temp, store.path_of(&want.file)).map_err(local),
        err => err,
    };
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

async fn download_to(
    client: &reqwest::Client,
    want: &Want,
    temp: &std::path::Path,
) -> Result<(), FetchError> {
    let mut response = client.get(&want.url).send().await.map_err(unreachable)?;
    let status = response.status();
    if !status.is_success() {
        return Err(FetchError {
            failure: status_failure(status),
            why: format!("{} answered {status}", want.url),
        });
    }
    let mut out = tokio::fs::File::create(temp).await.map_err(local)?;
    let mut hasher = Sha256::new();
    let mut len = 0u64;
    while let Some(chunk) = response.chunk().await.map_err(unreachable)? {
        len += chunk.len() as u64;
        if len > want.file.size {
            return Err(bad_bytes(want, "more bytes than listed"));
        }
        hasher.update(&chunk);
        out.write_all(&chunk).await.map_err(local)?;
    }
    out.flush().await.map_err(local)?;
    if len != want.file.size {
        return Err(bad_bytes(want, "fewer bytes than listed"));
    }
    if format!("{:x}", hasher.finalize()) != want.file.sha256 {
        return Err(bad_bytes(want, "not the listed hash"));
    }
    Ok(())
}

/// A service that answers "no such file" has not gone away: that file backs
/// off alone. Anything else (a 5xx, too many requests) is the service.
fn status_failure(status: reqwest::StatusCode) -> Failure {
    let refused = status.is_client_error() && status != reqwest::StatusCode::TOO_MANY_REQUESTS;
    if refused {
        Failure::Rejected
    } else {
        Failure::Unreachable
    }
}

fn bad_bytes(want: &Want, what: &str) -> FetchError {
    FetchError {
        failure: Failure::Rejected,
        why: format!("{}: {what}", want.file.path),
    }
}

fn unreachable(e: reqwest::Error) -> FetchError {
    FetchError {
        failure: Failure::Unreachable,
        why: e.without_url().to_string(),
    }
}

/// A full disk or an unwritable store is no fault of the file: pause all.
fn local(e: std::io::Error) -> FetchError {
    FetchError {
        failure: Failure::Unreachable,
        why: format!("store: {e}"),
    }
}
