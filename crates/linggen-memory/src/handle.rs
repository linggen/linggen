//! The memory handle: everything an open store owns — both tables, recall,
//! the embedder, the data directory, replica state, the connect broker,
//! presence and the activity clock. Opened once (`crate::hosted`) and shared
//! as an `Arc`; the HTTP handlers reach it through `axum::State`.
//!
//! Named `handle` beside `memory/` because `memory/` is the store itself
//! (rows, tables, recall); this is the one open instance of it.

use crate::embed::{Embedder, LoadOnce, Patience, WarmingUp};
use crate::error::{ErrorCode, MemError};
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
    /// Warmed in the background by the engine. Only verbs that embed
    /// access it, through [`Memory::embedder`] (which waits for the warm-up).
    pub embedder: Arc<LoadOnce<Embedder>>,
    /// Data directory root (typically `~/.linggen/`). User-tunable knobs
    /// (`.config.json` — episodic TTL etc.) live under
    /// `<data_dir>/memory/.config.json`.
    pub data_dir: PathBuf,
    /// The loopback TCP port this memory's routes answer on, for the fields
    /// that report it.
    pub port: u16,
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

fn warmup_error(warming: &WarmingUp) -> MemError {
    MemError {
        code: ErrorCode::WarmingUp,
        message: warming.to_string(),
    }
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
            connect: Default::default(),
            presence: Default::default(),

            started: Instant::now(),
            last_request_ms: AtomicU64::new(0),
        }
    }

    /// The hosted warm-up's refusal for a call that waits as `patience`
    /// says ([`Patience`]): `None` once warm (or with no warm-up under way —
    /// a quiet CLI store is not loaded here).
    pub(crate) async fn warming(&self, patience: Patience) -> Option<MemError> {
        self.embedder.warming(patience).await.as_ref().map(warmup_error)
    }

    /// The embedder, waiting for the engine's warm-up up to its bound;
    /// past it, refused with how long memory has been warming.
    pub async fn embedder(&self) -> Result<Arc<Embedder>, MemError> {
        self.embedder_with(Patience::WaitForWarmup).await
    }

    /// [`Memory::embedder`], saying whether to wait for the warm-up.
    pub async fn embedder_with(&self, patience: Patience) -> Result<Arc<Embedder>, MemError> {
        self.embedder.get_with(patience).await.map_err(|e| {
            if let Some(warming) = e.downcast_ref::<WarmingUp>() {
                return warmup_error(warming);
            }
            MemError::internal(e)
        })
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
