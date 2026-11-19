//! Memory hosted inside another process (the Linggen engine) instead of as
//! its own daemon: open the stores once, serve the routes on the host's port,
//! and hold the store's lock for the life of whoever opened it.
//!
//! The start-up steps the old daemon took — open both tables, attach the
//! change log, start upkeep and the replica loop — without the listener, the
//! pidfile, the LAN gate, the console UI or telemetry, which are the host's
//! business. The host loads the embedding model and runs one throw-away
//! inference in the background; a call that embeds waits for it up to
//! [`crate::embed::WARM_WAIT`].

use crate::embed::{Embedder, LoadOnce};
use crate::handle::Memory;
use crate::http::state::SharedState;
use crate::memory::{Filters, MemoryStore, Recall, SortOrder};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The one lock every writer of `paired-devices.json` in this process holds
/// while it reads, changes and saves the list — the host's device store too.
pub use crate::http::connect::devices::device_list_lock;

/// Who holds a store's lock, as the holder wrote it into the lock file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Holder {
    pub pid: u32,
    /// The port a host serves memory on; `None` for a CLI that opened the
    /// store directly.
    #[serde(default)]
    pub port: Option<u16>,
}

/// An exclusive `flock` on `<data_dir>/memory/.store.lock`, released when
/// dropped (or when the process ends). One writer per store: a second opener
/// is refused, with the holder named, instead of opening a second writer.
pub struct StoreLock {
    _file: File,
}

pub fn lock_path(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join(".store.lock")
}

impl StoreLock {
    /// Take the lock now or fail. `port` is recorded for other processes to
    /// find this one (`ling mem` with the engine up).
    pub fn acquire(data_dir: &Path, port: Option<u16>) -> Result<Self> {
        let path = lock_path(data_dir);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        if !try_flock(&file)? {
            let holder = match Self::holder(data_dir) {
                Some(h) => match h.port {
                    Some(port) => format!("pid {}, serving on port {port}", h.pid),
                    None => format!("pid {}", h.pid),
                },
                None => "another process".to_string(),
            };
            anyhow::bail!(
                "the memory store under {} is already open ({holder}); one process at a time",
                data_dir.display()
            );
        }
        let me = Holder {
            pid: std::process::id(),
            port,
        };
        file.set_len(0)?;
        file.write_all(serde_json::to_string(&me)?.as_bytes())?;
        file.flush()?;
        Ok(Self { _file: file })
    }

    /// What the current holder recorded, if the file says.
    pub fn holder(data_dir: &Path) -> Option<Holder> {
        let text = std::fs::read_to_string(lock_path(data_dir)).ok()?;
        serde_json::from_str(&text).ok()
    }
}

#[cfg(unix)]
fn try_flock(file: &File) -> Result<bool> {
    use std::os::unix::io::AsRawFd;
    // SAFETY: flock on a descriptor this function borrows; no memory is shared.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    if err.kind() == std::io::ErrorKind::WouldBlock {
        return Ok(false);
    }
    Err(anyhow::Error::from(err).context("locking the memory store"))
}

#[cfg(not(unix))]
fn try_flock(_file: &File) -> Result<bool> {
    Ok(true)
}

/// One change log for both tables (`memory::changes`) — what a replica pulls.
/// A store that has never had one is seeded with every existing row, so a
/// replica's first pull is a full copy.
async fn attach_change_log(
    data_dir: &Path,
    semantic: &MemoryStore,
    episodic: &MemoryStore,
) -> Result<()> {
    use crate::memory::changes::{log_path, ChangeLog, Op};
    let log = Arc::new(ChangeLog::open(log_path(data_dir)).context("opening the change log")?);
    if log.needs_seed() {
        for store in [semantic, episodic] {
            let ids = store.ids_where(None).await?;
            log.record(store.table_name(), Op::Upsert, &ids);
        }
        tracing::info!("change log seeded at seq {}", log.head());
    }
    semantic.attach_changes(Arc::clone(&log));
    episodic.attach_changes(log);
    Ok(())
}

/// Open the stores under `data_dir` and start their background work on the
/// current runtime. `self_base` is where this memory's routes are reachable
/// over loopback, e.g. `http://127.0.0.1:9527/mem` — its port is what the
/// store reports as its own. The caller holds the [`StoreLock`].
pub async fn open(data_dir: &Path, self_base: String) -> Result<SharedState> {
    let state = open_quiet(data_dir, self_base).await?;
    let _warmup = start_embedding_warmup(&state);
    crate::ops::upkeep::spawn(Arc::clone(&state));
    crate::replica::spawn(Arc::clone(&state));
    Ok(state)
}

fn start_embedding_warmup(state: &Memory) -> Option<tokio::task::JoinHandle<()>> {
    state
        .embedder
        .start_warmup(|model| model.embed_query("memory warm-up").map(drop))
}

/// [`open`] without the background work: for a short-lived caller (a
/// `ling mem` verb run on the store directly) that only answers requests.
pub async fn open_quiet(data_dir: &Path, self_base: String) -> Result<SharedState> {
    open_with(data_dir, self_base, Embedder::new).await
}

/// [`open_quiet`] on the deterministic test embedder (`Embedder::fake`):
/// the same stores, change log and state, no model. Tests only.
#[cfg(any(test, feature = "test-support"))]
pub async fn open_quiet_fake(data_dir: &Path, self_base: String) -> Result<SharedState> {
    open_with(data_dir, self_base, Embedder::fake).await
}

/// Hold the background inference open until the test drops or sends
/// `release`, with `wait` as the bound a waiting call waits for it.
#[cfg(any(test, feature = "test-support"))]
pub async fn hold_embedding_warmup(
    state: &Memory,
    wait: std::time::Duration,
) -> (tokio::sync::oneshot::Sender<()>, tokio::task::JoinHandle<()>) {
    let (entered, running) = tokio::sync::oneshot::channel();
    let (release, held) = tokio::sync::oneshot::channel();
    let warmup = state
        .embedder
        .start_warmup_waiting(wait, move |model| {
            entered.send(()).unwrap();
            // Dropping `release` also unblocks the worker if an assertion fails.
            let _ = held.blocking_recv();
            model.embed_query("memory warm-up").map(drop)
        })
        .expect("warm-up not started yet");
    running.await.unwrap();
    (release, warmup)
}

async fn open_with(
    data_dir: &Path,
    self_base: String,
    load: fn() -> Result<Embedder>,
) -> Result<SharedState> {
    let semantic = Arc::new(
        MemoryStore::open_semantic(data_dir)
            .await
            .with_context(|| format!("opening memory store under {}", data_dir.display()))?,
    );
    let episodic = Arc::new(
        MemoryStore::open_episodic(data_dir)
            .await
            .with_context(|| format!("opening episodic store under {}", data_dir.display()))?,
    );
    attach_change_log(data_dir, &semantic, &episodic).await?;
    let recall = Arc::new(Recall::new(Arc::clone(&semantic), Arc::clone(&episodic)));
    let embedder = Arc::new(LoadOnce::new(load));
    let bound = self_addr(&self_base);
    let state = Memory::new(
        semantic,
        episodic,
        recall,
        embedder,
        data_dir.to_path_buf(),
        bound,
    );
    Ok(Arc::new(state))
}

/// Serve memory's routes on a free loopback port of this process, for as
/// long as the process runs; returns their base URL. How `ling mem` runs a
/// verb that only the HTTP handlers implement (days, stats, …) with no
/// engine up: the same handlers, on the store this process holds the lock of.
pub async fn serve_here(data_dir: &Path) -> Result<String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("binding a loopback port")?;
    let base = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let app = router(open_quiet(data_dir, base.clone()).await?);
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });
    Ok(base)
}

/// The loopback address in `self_base`, for the fields that report it.
fn self_addr(self_base: &str) -> SocketAddr {
    let port = self_base
        .rsplit(':')
        .next()
        .and_then(|rest| rest.split('/').next())
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    SocketAddr::from(([127, 0, 0, 1], port))
}

/// The marker that says `tag` (a host version) already backed this store up.
fn backup_marker(data_dir: &Path, tag: &str, outcome: &str) -> PathBuf {
    crate::memory::backups::backups_dir(data_dir).join(format!("takeover-{tag}.{outcome}"))
}

/// Whether `tag` has backed this store up (or found none to back up).
pub fn backup_taken(data_dir: &Path, tag: &str) -> bool {
    backup_marker(data_dir, tag, "done").exists()
}

/// Why the last backup for `tag` failed, if it did — for a status report.
pub fn backup_failure(data_dir: &Path, tag: &str) -> Option<String> {
    std::fs::read_to_string(backup_marker(data_dir, tag, "failed")).ok()
}

/// Copy the store once per host version, before that version first opens
/// it: `memory/backups/takeover-<UTC stamp>/` (expired like the other
/// migration backups), then the marker `takeover-<tag>.done`. Everything
/// under `memory/` but `backups/`, the old daemon's logs and the lock file.
/// `Ok(None)` when already done or there is no store yet. A failure leaves
/// `takeover-<tag>.failed` with the reason, and the next start tries again.
/// The caller holds the [`StoreLock`].
pub fn backup_once(data_dir: &Path, tag: &str) -> Result<Option<PathBuf>> {
    let done = backup_marker(data_dir, tag, "done");
    let failed = backup_marker(data_dir, tag, "failed");
    let mem = data_dir.join("memory");
    if done.exists() {
        return Ok(None);
    }
    std::fs::create_dir_all(crate::memory::backups::backups_dir(data_dir))?;
    if !mem.join("memory.lancedb").exists() {
        std::fs::write(&done, "no store to back up\n")?;
        return Ok(None);
    }
    let dest = crate::memory::backups::migration_backup_dir(
        data_dir,
        crate::memory::backups::TAKEOVER,
        chrono::Utc::now(),
    );
    match copy_store(&mem, &dest) {
        Ok(()) => {
            let _ = std::fs::remove_file(&failed);
            std::fs::write(&done, format!("{}\n", dest.display()))?;
            Ok(Some(dest))
        }
        Err(e) => {
            let _ = std::fs::write(&failed, format!("{e}\n"));
            Err(anyhow::Error::from(e).context(format!("backing up {}", mem.display())))
        }
    }
}

fn copy_store(mem: &Path, dest: &Path) -> std::io::Result<()> {
    const SKIP: [&str; 3] = ["backups", "linggen-memory", ".store.lock"];
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(mem)? {
        let entry = entry?;
        let name = entry.file_name();
        if SKIP.iter().any(|s| name == *s) {
            continue;
        }
        let target = dest.join(&name);
        if entry.file_type()?.is_dir() {
            crate::http::migration::copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Memory's routes for a host to mount (see [`crate::http::build_hosted_router`]).
pub fn router(state: SharedState) -> axum::Router {
    crate::http::build_hosted_router(state)
}

/// Every curated row, vectors stripped, as NDJSON at `path` — what
/// `ling-mem export <file>` writes.
pub async fn export(store: &MemoryStore, path: &Path) -> Result<()> {
    let mut facts = store
        .list(&Filters::default(), SortOrder::Newest, usize::MAX, 0)
        .await?;
    let mut out = std::io::BufWriter::new(
        File::create(path).with_context(|| format!("creating {}", path.display()))?,
    );
    for f in &mut facts {
        f.vector = None;
        serde_json::to_writer(&mut out, f).context("serializing a row")?;
        out.write_all(b"\n")?;
    }
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn background_warmup_runs_once_and_stores_no_rows() {
        use crate::Who;
        use serde_json::json;

        let dir = tempfile::tempdir().unwrap();
        let state = open_quiet_fake(dir.path(), String::new()).await.unwrap();
        let warmup = start_embedding_warmup(&state).unwrap();
        assert!(start_embedding_warmup(&state).is_none());
        warmup.await.unwrap();
        assert!(state.embedder.is_ready());
        let count = state.call(&Who::Owner, "count", json!({})).await.unwrap();
        assert_eq!(count.into_result().unwrap()["count"], 0);
    }

    /// A warm-up held in its first inference until `release` is sent (or
    /// dropped), with `wait` as the bound a waiting call waits for it.
    async fn blocked_warmup(
        state: &SharedState,
        wait: std::time::Duration,
    ) -> (
        tokio::task::JoinHandle<()>,
        tokio::sync::oneshot::Sender<()>,
    ) {
        let (release, warmup) = hold_embedding_warmup(state, wait).await;
        (warmup, release)
    }

    #[tokio::test]
    async fn a_write_during_warmup_waits_for_it_and_lands() {
        use crate::Who;
        use serde_json::json;

        let dir = tempfile::tempdir().unwrap();
        let state = open_quiet_fake(dir.path(), String::new()).await.unwrap();
        let (warmup, release) = blocked_warmup(&state, crate::embed::WARM_WAIT).await;
        let writer = state.clone();
        let add = tokio::spawn(async move {
            let args = json!({"content": "a row written while warming", "tier": "semantic"});
            writer.call(&Who::Owner, "add", args).await
        });
        let batcher = state.clone();
        let batch = tokio::spawn(async move {
            let args = json!({"facts": [{"content": "an unrelated note about tea"}]});
            batcher.call(&Who::Owner, "add_batch", args).await
        });
        let searcher = state.clone();
        let search = tokio::spawn(async move {
            let args = json!({"query": "a row written while warming"});
            searcher.call(&Who::Owner, "search", args).await
        });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!add.is_finished(), "the write must wait for the warm-up");
        assert!(!search.is_finished(), "a search must wait for the warm-up");
        assert!(!batch.is_finished(), "the batch must wait for the warm-up");
        release.send(()).unwrap();
        warmup.await.unwrap();
        let added = add.await.unwrap().unwrap().into_result().unwrap();
        batch.await.unwrap().unwrap().into_result().unwrap();
        search.await.unwrap().unwrap().into_result().unwrap();
        let count = state.call(&Who::Owner, "count", json!({})).await.unwrap();
        assert_eq!(count.into_result().unwrap()["count"], 2);
        let query = json!({"query": "a row written while warming"});
        let hits = state.call(&Who::Owner, "search", query).await.unwrap();
        assert_eq!(hits.into_result().unwrap()[0]["id"], added["fact"]["id"]);
    }

    #[tokio::test]
    async fn a_recall_during_warmup_answers_at_once_through_the_door() {
        use serde_json::{json, Value};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = open_quiet_fake(dir.path(), String::new()).await.unwrap();
        let (warmup, release) = blocked_warmup(&state, crate::embed::WARM_WAIT).await;
        let call = json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "memory_search", "arguments": {
                "query": "anything about the person", "skip_if_warming": true,
            }},
        });
        let request = axum::http::Request::post("/mcp")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(call.to_string()))
            .unwrap();
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            router(state.clone()).oneshot(request),
        )
        .await
        .expect("a recall must not wait for the warm-up")
        .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        let answer: Value = serde_json::from_slice(&bytes).unwrap();
        let warming = crate::ErrorCode::WarmingUp.mcp_status();
        assert_eq!(answer["error"]["code"], warming, "{answer}");
        let message = answer["error"]["message"].as_str().unwrap();
        assert!(message.starts_with("Memory is warming up ("), "{message}");
        release.send(()).unwrap();
        warmup.await.unwrap();
    }

    #[tokio::test]
    async fn a_warmup_longer_than_the_bound_refuses_and_writes_nothing() {
        use crate::Who;
        use serde_json::json;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = open_quiet_fake(dir.path(), String::new()).await.unwrap();
        let bound = std::time::Duration::from_millis(50);
        let (warmup, release) = blocked_warmup(&state, bound).await;
        assert!(!state.embedder.is_ready());
        let request = axum::http::Request::get("/api/health")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            router(state.clone())
                .oneshot(request)
                .await
                .unwrap()
                .status(),
            200
        );
        let args = json!({"content": "a row to retry", "tier": "semantic"});
        for (verb, args) in [
            ("add", args.clone()),
            (
                "add_batch",
                json!({"facts": [{"content": "a batch to retry"}]}),
            ),
            ("search", json!({"query": "a row to retry"})),
        ] {
            let error = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                state.call(&Who::Owner, verb, args),
            )
            .await
            .expect("past the bound the call must answer while the probe is still blocked")
            .unwrap_err();
            assert_eq!(error.code.code(), "WARMING_UP");
            assert!(
                error.message.starts_with("Memory is warming up ("),
                "{error:?}"
            );
            assert!(error.message.ends_with("s elapsed)."), "{error:?}");
        }
        let count = state.call(&Who::Owner, "count", json!({})).await.unwrap();
        assert_eq!(count.into_result().unwrap()["count"], 0);
        assert_eq!(crate::replica::outbox::len(&state.data_dir), 0, "nothing queued");
        release.send(()).unwrap();
        warmup.await.unwrap();
        let count = state.call(&Who::Owner, "count", json!({})).await.unwrap();
        assert_eq!(count.into_result().unwrap()["count"], 0);
        let added = state
            .call(&Who::Owner, "add", args)
            .await
            .unwrap()
            .into_result()
            .unwrap();
        let hits = state
            .call(&Who::Owner, "search", json!({"query": "a row to retry"}))
            .await
            .unwrap()
            .into_result()
            .unwrap();
        assert_eq!(hits[0]["id"], added["fact"]["id"]);
    }

    #[test]
    fn the_store_is_backed_up_once_per_version() {
        let dir = tempfile::tempdir().unwrap();
        let mem = dir.path().join("memory");
        std::fs::create_dir_all(mem.join("memory.lancedb/semantic.lance")).unwrap();
        std::fs::write(mem.join("memory.lancedb/semantic.lance/x"), "rows").unwrap();
        std::fs::write(mem.join(".days.json"), "{}").unwrap();
        std::fs::create_dir_all(mem.join("linggen-memory")).unwrap();
        std::fs::write(mem.join("linggen-memory/serve.log"), "log").unwrap();

        let copy = backup_once(dir.path(), "9.9.9").unwrap().expect("a copy");
        assert!(copy.join("memory.lancedb/semantic.lance/x").is_file());
        assert!(copy.join(".days.json").is_file());
        assert!(!copy.join("linggen-memory").exists(), "logs are not copied");
        assert!(backup_once(dir.path(), "9.9.9").unwrap().is_none(), "once");
        assert!(
            backup_once(dir.path(), "9.9.10").unwrap().is_some(),
            "per version"
        );
    }

    #[test]
    fn a_second_opener_is_refused_until_the_first_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        let first = StoreLock::acquire(dir.path(), Some(4100)).unwrap();
        let err = StoreLock::acquire(dir.path(), None).err().expect("refused");
        assert!(err.to_string().contains("port 4100"), "{err}");
        assert_eq!(StoreLock::holder(dir.path()).unwrap().port, Some(4100));
        drop(first);
        // A process another test forks in this moment holds a copy of the
        // descriptor until it execs (milliseconds): allow for that.
        let freed = (0..100).any(|_| {
            let ok = StoreLock::acquire(dir.path(), None).is_ok();
            if !ok {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            ok
        });
        assert!(freed, "the lock is free once its holder lets go");
    }
}
