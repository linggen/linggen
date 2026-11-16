//! The memory handle: everything an open store owns — both tables, recall,
//! the embedder, the data directory, replica state, the connect broker,
//! presence and the activity clock. Opened once (`crate::hosted`) and shared
//! as an `Arc`; the HTTP handlers reach it through `axum::State`.
//!
//! Named `handle` beside `memory/` because `memory/` is the store itself
//! (rows, tables, recall); this is the one open instance of it.

use crate::embed::{Embedder, LoadOnce};
use crate::error::MemError;
use crate::memory::{MemoryStore, Recall};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Memory {
    /// Curated `semantic` store — the default-table write/CRUD/browse path.
    pub store: Arc<MemoryStore>,
    /// Staging `episodic` store — the encoder + dream worklist target.
    /// Selected over `store` when an `add`/`delete` request sets
    /// `episodic: true`.
    pub episodic: Arc<MemoryStore>,
    /// Dual-table read path (`semantic` + `episodic`) for recall/search.
    /// Shares both handles above.
    pub recall: Arc<Recall>,
    /// Loaded in the background at startup ([`LoadOnce`]); a handler that
    /// embeds awaits it through [`Memory::embedder`], one that only reads
    /// rows never waits on it.
    pub embedder: Arc<LoadOnce<Embedder>>,
    /// Data directory root (typically `~/.linggen/`). User-tunable knobs
    /// (`.config.json` — episodic TTL etc.) live under
    /// `<data_dir>/memory/.config.json`.
    pub data_dir: PathBuf,
    /// Bound TCP port of this daemon's HTTP server. The MCP handler uses
    /// it to loopback POST against `/api/memory/<verb>` so MCP `tools/call`
    /// reuses the same handlers (and their dispatch fixes) instead of
    /// duplicating logic.
    pub port: u16,
    /// Where this memory's routes are reachable over loopback — the MCP
    /// handler's self-POST base. `http://127.0.0.1:<port>` for the daemon;
    /// the host's prefix when another process hosts memory (`crate::hosted`).
    pub self_base: String,
    /// Machines on the LAN asking to connect — see `http::connect`.
    pub connect: crate::http::connect::Broker,
    /// Paired devices seen by the gate, in memory — see `http::presence`.
    pub presence: crate::http::presence::Presence,
    /// Primary or replica, and a replica's sync state — see `crate::replica`.
    pub replica: crate::replica::Replica,

    /// When the store was opened, and when it last served a request —
    /// milliseconds since `started`. Background storage maintenance waits
    /// for a quiet window before rewriting tables, because compaction
    /// holds a table's write lock and would otherwise stall a live recall.
    ///
    /// An `Instant` baseline plus a relative counter, rather than wall
    /// clock: a clock step (NTP, sleep/wake, timezone) must not make the
    /// store look idle for hours.
    started: Instant,
    last_request_ms: AtomicU64,
}

impl Memory {
    /// Build the handle. `started` is stamped here so "idle since" is
    /// measured from opening, not from the first request.
    pub fn new(
        store: Arc<MemoryStore>,
        episodic: Arc<MemoryStore>,
        recall: Arc<Recall>,
        embedder: Arc<LoadOnce<Embedder>>,
        data_dir: PathBuf,
        bound: SocketAddr,
    ) -> Self {
        let replica = crate::replica::Replica::load(&data_dir);
        Self {
            replica,
            store,
            episodic,
            recall,
            embedder,
            data_dir,
            port: bound.port(),
            self_base: format!("http://127.0.0.1:{}", bound.port()),
            connect: Default::default(),
            presence: Default::default(),

            started: Instant::now(),
            last_request_ms: AtomicU64::new(0),
        }
    }

    /// The embedder, waiting for its load if it is still under way.
    pub async fn embedder(&self) -> Result<Arc<Embedder>, MemError> {
        self.embedder
            .get()
            .await
            .map_err(|e| MemError::internal(e.context("loading the embedding model")))
    }

    /// Record that a request just arrived. Called by `Memory::call` and by
    /// the router's activity layer on every route; `Relaxed` is right
    /// because the only reader is a background timer that tolerates being a
    /// moment stale.
    pub fn mark_request(&self) {
        self.last_request_ms
            .store(self.started.elapsed().as_millis() as u64, Ordering::Relaxed);
    }

    /// How long since the last request (or since startup, if none).
    pub fn idle_for(&self) -> Duration {
        let last = self.last_request_ms.load(Ordering::Relaxed);
        self.started
            .elapsed()
            .saturating_sub(Duration::from_millis(last))
    }
}

/// A handle over a scratch store, for handler tests.
#[cfg(test)]
pub(crate) async fn scratch_state(dir: &std::path::Path, bound: SocketAddr) -> Arc<Memory> {
    let semantic = Arc::new(MemoryStore::open_semantic(dir).await.unwrap());
    let episodic = Arc::new(MemoryStore::open_episodic(dir).await.unwrap());
    let recall = Arc::new(Recall::new(Arc::clone(&semantic), Arc::clone(&episodic)));
    fn no_model() -> anyhow::Result<Embedder> {
        anyhow::bail!("no model in unit tests")
    }
    Arc::new(Memory::new(
        semantic,
        episodic,
        recall,
        Arc::new(LoadOnce::new(no_model)),
        dir.to_path_buf(),
        bound,
    ))
}
