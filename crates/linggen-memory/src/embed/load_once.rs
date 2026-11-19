//! Load once on the blocking pool. The engine starts a background load and
//! warm-up; short-lived local CLI calls keep the lazy load.

use anyhow::{Context, Result};
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tokio::sync::OnceCell;

struct Warmup<T> {
    started: Instant,
    result: OnceLock<Result<Arc<T>, String>>,
}

/// The model has not yet completed its first inference.
#[derive(Debug)]
pub(crate) struct WarmingUp(pub std::time::Duration);

impl std::fmt::Display for WarmingUp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Memory is warming up ({:.1}s elapsed).",
            self.0.as_secs_f64()
        )
    }
}

impl std::error::Error for WarmingUp {}

pub struct LoadOnce<T> {
    cell: OnceCell<Arc<T>>,
    load: fn() -> Result<T>,
    warmup: OnceLock<Warmup<T>>,
}

impl<T: Send + Sync + 'static> LoadOnce<T> {
    /// Nothing is loaded until the first [`Self::get`].
    pub fn new(load: fn() -> Result<T>) -> Self {
        Self {
            cell: OnceCell::new(),
            load,
            warmup: OnceLock::new(),
        }
    }

    /// Start one background load and inference before publishing the hosted
    /// store. Do not expose the model until the inference succeeds.
    pub(crate) fn start_warmup(
        self: &Arc<Self>,
        probe: impl FnOnce(&T) -> Result<()> + Send + 'static,
    ) -> Option<tokio::task::JoinHandle<()>> {
        self.warmup
            .set(Warmup {
                started: Instant::now(),
                result: OnceLock::new(),
            })
            .ok()?;
        let this = Arc::clone(self);
        Some(tokio::spawn(async move {
            let warmup = this.warmup.get().expect("warm-up started");
            let load = this.load;
            let result = tokio::task::spawn_blocking(move || {
                let model = load().context("loading the embedding model")?;
                let first = Instant::now();
                probe(&model).context("running the first embedding")?;
                Ok::<_, anyhow::Error>((model, first.elapsed()))
            })
            .await
            .context("embedding warm-up worker failed")
            .and_then(|result| result);
            let elapsed_ms = warmup.started.elapsed().as_millis() as u64;
            let ready = match result {
                Ok((model, first)) => {
                    tracing::info!(
                        target: "linggen::memory",
                        elapsed_ms,
                        first_inference_ms = first.as_millis() as u64,
                        "memory: embedder warm"
                    );
                    Ok(Arc::new(model))
                }
                Err(error) => {
                    tracing::warn!(elapsed_ms, "memory: embedding warm-up failed: {error:#}");
                    Err(format!("Memory embedding warm-up failed: {error:#}"))
                }
            };
            let _ = warmup.result.set(ready);
        }))
    }

    /// The value, loading it lazily only for callers that did not start a
    /// background warm-up.
    /// Concurrent callers share one load.
    pub async fn get(&self) -> Result<Arc<T>> {
        if let Some(warmup) = self.warmup.get() {
            return match warmup.result.get() {
                Some(Ok(model)) => Ok(Arc::clone(model)),
                Some(Err(error)) => Err(anyhow::anyhow!(error.clone())),
                None => Err(WarmingUp(warmup.started.elapsed()).into()),
            };
        }
        let load = self.load;
        self.cell
            .get_or_try_init(|| async move {
                tokio::task::spawn_blocking(load)
                    .await
                    .context("loader thread failed")?
                    .context("loading the embedding model")
                    .map(Arc::new)
            })
            .await
            .cloned()
    }

    /// Loaded already — without waiting or starting a load.
    pub fn is_ready(&self) -> bool {
        match self.warmup.get() {
            Some(warmup) => matches!(warmup.result.get(), Some(Ok(_))),
            None => self.cell.initialized(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static LOADS: AtomicUsize = AtomicUsize::new(0);
    fn counted() -> Result<usize> {
        std::thread::sleep(std::time::Duration::from_millis(50));
        Ok(LOADS.fetch_add(1, Ordering::SeqCst) + 1)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_callers_share_one_load() {
        let once = Arc::new(LoadOnce::new(counted));
        assert!(!once.is_ready());
        let mut handles = Vec::new();
        for _ in 0..8 {
            let o = Arc::clone(&once);
            handles.push(tokio::spawn(async move { *o.get().await.unwrap() }));
        }
        for h in handles {
            assert_eq!(h.await.unwrap(), 1);
        }
        assert!(once.is_ready());
        assert_eq!(LOADS.load(Ordering::SeqCst), 1);
    }

    static TRIES: AtomicUsize = AtomicUsize::new(0);
    fn fails_once() -> Result<&'static str> {
        if TRIES.fetch_add(1, Ordering::SeqCst) == 0 {
            anyhow::bail!("model not downloadable yet")
        }
        Ok("loaded")
    }

    #[tokio::test]
    async fn a_failed_load_is_retried_by_the_next_caller() {
        let once = LoadOnce::new(fails_once);
        assert!(once.get().await.is_err());
        assert!(!once.is_ready());
        assert_eq!(*once.get().await.unwrap(), "loaded");
        assert!(once.is_ready());
    }
}
