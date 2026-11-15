//! A value built once, off the request path, by whoever needs it first.
//!
//! The daemon's embedder is the case it exists for: loading Qwen3 weights
//! (~1.2 GB) takes one to several seconds on a cold disk, and while
//! `serve` loaded it before serving, every request that arrived in that
//! window waited for it — including `session_start`, which reads rows and
//! never embeds. Hosts cap that call (the engine at 2 s), so the first
//! session after the daemon started went without core memory.
//!
//! Now `serve` starts the load in the background and serves at once; a
//! request that needs the value awaits the same load, one that doesn't
//! never touches it. A failed load is not kept: the next caller tries again.

use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::sync::OnceCell;

pub struct LoadOnce<T> {
    cell: OnceCell<Arc<T>>,
    load: fn() -> Result<T>,
}

impl<T: Send + Sync + 'static> LoadOnce<T> {
    /// Nothing is loaded until the first [`Self::get`].
    pub fn new(load: fn() -> Result<T>) -> Self {
        Self {
            cell: OnceCell::new(),
            load,
        }
    }

    /// The value, loading it on the blocking pool if no one has yet.
    /// Concurrent callers share one load.
    pub async fn get(&self) -> Result<Arc<T>> {
        let load = self.load;
        self.cell
            .get_or_try_init(|| async move {
                tokio::task::spawn_blocking(load)
                    .await
                    .context("loader thread failed")?
                    .map(Arc::new)
            })
            .await
            .cloned()
    }

    /// Loaded already — without waiting or starting a load.
    pub fn is_ready(&self) -> bool {
        self.cell.initialized()
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
