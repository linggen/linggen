//! Primary or replica — where this machine's memory is written.
//!
//! Every machine runs its own ling-mem, and every client talks to the local
//! one on loopback. That daemon is either a **primary** (the default: its
//! store is the store) or a **replica** of another machine's primary. A
//! replica serves reads from its own copy, pulls the primary's changes
//! (`/api/sync/changes`), and forwards writes to it — queueing them in an
//! outbox while the primary is offline. Clients never learn where the primary
//! is.
//!
//! The choice and everything a replica needs live in
//! `<data_dir>/memory/.replica.json` (owner-only): the upstream address, the
//! device token the primary issued on Allow, the sync cursor, and the last
//! outcome. No client ever sees the token.
//!
//! - `state.rs` — that file.
//! - `link.rs`  — becoming a replica: the connect request to the primary.
//! - `pull.rs`  — pulling and applying the primary's changes.
//! - `outbox.rs`, `forward.rs` — writes on a replica.

pub mod forward;
pub mod link;
pub mod outbox;
pub mod pull;
pub mod state;

pub use state::{ReplicaFile, Role};

use std::sync::Arc;
use std::time::Duration;

/// How often a replica pulls when nothing asks sooner.
pub const PULL_EVERY: Duration = Duration::from_secs(30);
/// Requests to the primary give up after this — a dead primary must not stall
/// a client's write for long before it is queued instead.
pub const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(5);
/// A pull page carries vectors (~10 MB per 500 rows), so it gets longer to
/// arrive; a dead primary is still caught by the connect timeout.
pub const PULL_TIMEOUT: Duration = Duration::from_secs(60);

/// The header a paired device presents — the primary's gate reads it.
pub const DEVICE_HEADER: &str = "x-linggen-device";

/// The live replica state the daemon shares: the file's contents behind a
/// lock, and a nudge that wakes the pull loop early (after a write).
pub struct Replica {
    file: std::sync::Mutex<ReplicaFile>,
    data_dir: std::path::PathBuf,
    pub(crate) wake: tokio::sync::Notify,
    /// Serializes pulls and outbox replays, so two never interleave.
    pub(crate) sync_lock: tokio::sync::Mutex<()>,
}

impl Replica {
    pub fn load(data_dir: &std::path::Path) -> Self {
        Self {
            file: std::sync::Mutex::new(ReplicaFile::load(data_dir)),
            data_dir: data_dir.to_path_buf(),
            wake: tokio::sync::Notify::new(),
            sync_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn snapshot(&self) -> ReplicaFile {
        self.file.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Change the file and persist it.
    pub fn update(&self, f: impl FnOnce(&mut ReplicaFile)) {
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut file);
        if let Err(e) = file.save(&self.data_dir) {
            tracing::warn!("replica: could not save state: {e}");
        }
    }

    pub fn role(&self) -> Role {
        self.snapshot().role()
    }

    /// The upstream base URL and token, when this is a working replica.
    pub fn upstream(&self) -> Option<(String, String)> {
        let f = self.snapshot();
        Some((f.upstream_url()?, f.token.clone()?))
    }

    pub fn nudge(&self) {
        self.wake.notify_one();
    }
}

/// A client for the primary, with the token attached by the caller.
pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(UPSTREAM_TIMEOUT)
        .build()
        .unwrap_or_default()
}

/// A client for pulling pages: quick to notice a dead primary, patient with a
/// big page on a slow link.
pub fn pull_http() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(UPSTREAM_TIMEOUT)
        .timeout(PULL_TIMEOUT)
        .build()
        .unwrap_or_default()
}

/// Run the pull loop for the life of the daemon: every `PULL_EVERY`, or at
/// once when nudged. Does nothing on a primary.
pub fn spawn(state: crate::http::state::SharedState) {
    tokio::spawn(async move {
        loop {
            let _ = tokio::time::timeout(PULL_EVERY, state.replica.wake.notified()).await;
            if state.replica.upstream().is_some() {
                sync_once(&state).await;
            }
        }
    });
}

/// Replay what queued while offline, then pull.
pub async fn sync_once(state: &crate::http::state::SharedState) {
    let _guard = state.replica.sync_lock.lock().await;
    if let Err(e) = outbox::replay(state).await {
        tracing::info!("replica: outbox not replayed yet: {e:#}");
    }
    pull::pull_all(state).await;
}

/// For tests and handlers that only need the shared handle.
pub type SharedReplica = Arc<Replica>;
