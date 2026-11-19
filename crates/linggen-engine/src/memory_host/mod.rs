//! Memory, hosted inside this process.
//!
//! The engine opens the memory store itself at start-up (`start`) and serves
//! memory's HTTP routes on its own port (`routes`) — the only port: nothing
//! answers at the old daemon's address. No second process: what
//! used to be the `ling-mem` daemon is the `ling_mem` library, run here on a
//! tokio runtime of its own, so a slow query or a store compaction keeps to
//! memory's threads and cannot stall the engine's (WebRTC, streaming, the UI).
//!
//! - The store is held under an exclusive lock (`<store>/.store.lock`) for the
//!   life of the process; a second opener — another engine, or `ling mem` with
//!   the engine down — is refused with the holder named.
//! - A failed open does not stop the engine: memory answers 503 with the
//!   reason, everything else serves.
//! - The embedding model warms up in the background without delaying health.
//! - Before opening: an old `ling-mem` daemon still serving the store is
//!   stopped (`takeover`), and each engine version copies the store once
//!   (`memory/backups/takeover-<stamp>/`, marker `takeover-<version>.done`).
//!
//! The engine's own callers run a verb in-process (`call`: the verb answered
//! as its route answers it, for a [`ling_mem::Who`], on memory's runtime,
//! the engine's task awaiting the join). What stays HTTP: MCP clients —
//! Claude Code, Codex, the Claude runner and the engine's own agents through
//! `mcp_client` (`url` points them at [`PREFIX`] on this port) — the
//! replica protocol between machines, and `ling mem` beside a running
//! engine. Memory's own settings and connect routes, which read the
//! connection, are reached through its router (`local_call`).

mod call;
pub mod caller;
mod routes;
mod standin;
mod takeover;

pub use call::{post_verb, post_verb_blocking, Answer};
#[cfg(any(test, feature = "test-support"))]
pub use call::{TestHost, TEST_HOST};
pub use routes::{is_open_path, routes};
pub use standin::{ling_mem_path, restore_previous};

use ling_mem::hosted::StoreLock;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

/// Where memory's own routes are mounted on the engine's port —
/// `/mem/api/memory/<verb>`, `/mem/api/health`, `/mem/mcp`, … For this
/// machine; an approved machine reaches the replica protocol here (`routes`).
/// Not under `/api/`: the phone's WebRTC channel forwards `/api/*` to this
/// port as loopback, and the phone's door is the stamped `/api/memory/<verb>`.
pub const PREFIX: &str = ling_mem::replica::state::PRIMARY_PREFIX;

/// The retired address of memory's MCP server for hosts, from before
/// Linggen's one door (`/mcp`): it answers [`UPGRADE_PLUGIN`] and nothing
/// else. The engine's own agents reach memory's tools at `/mem/mcp`.
pub(crate) const MCP_PATH: &str = "/mcp/memory";

/// What an old plugin is told, wherever it knocks — `/mcp/memory`, or a
/// retired tool name at `/mcp`.
pub const UPGRADE_PLUGIN: &str =
    "This plugin uses a retired Linggen MCP endpoint; update the Linggen plugin and restart your host.";

/// Worker threads of memory's runtime. Its blocking pool (embedding, file
/// I/O) is capped separately by [`BLOCKING_THREADS`].
const WORKER_THREADS: usize = 3;
const BLOCKING_THREADS: usize = 8;

/// How long start-up waits for the store's lock: an engine being restarted
/// can still be letting go of it.
const LOCK_WAIT: Duration = Duration::from_secs(10);

static HOST: OnceLock<Host> = OnceLock::new();

struct Host {
    port: u16,
    runtime: tokio::runtime::Handle,
    served: Result<Served, String>,
}

struct Served {
    router: axum::Router,
    state: ling_mem::http::state::SharedState,
    _lock: StoreLock,
}

/// Memory's runtime, built once and kept for the life of the process
/// (a runtime dropped inside async code panics).
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(WORKER_THREADS)
            .max_blocking_threads(BLOCKING_THREADS)
            .thread_name("memory")
            .enable_all()
            .build()
            .expect("building memory's runtime")
    })
}

/// Open the store under `linggen_home()` and get ready to serve it on `port`.
/// Called once, before the listener binds. Never fails: an open that fails is
/// kept as the reason memory answers 503.
pub async fn start(port: u16) {
    let data_dir = crate::paths::linggen_home().clone();
    let served = open(&data_dir, port).await;
    match &served {
        Ok(_) => {
            tracing::info!(
                "memory: store open under {} — serving at {PREFIX} and {MCP_PATH}",
                data_dir.display()
            );
            standin::install(port);
        }
        Err(why) => tracing::warn!("memory: not serving — {why}"),
    }
    let _ = HOST.set(Host {
        port,
        runtime: runtime().handle().clone(),
        served,
    });
}

async fn open(data_dir: &Path, port: u16) -> Result<Served, String> {
    takeover::stop_old_daemon(data_dir).await?;
    let lock = acquire_lock(data_dir.to_path_buf(), port).await?;
    back_up_once(data_dir);
    let dir = data_dir.to_path_buf();
    let self_base = url_for(port);
    let state = runtime()
        .spawn(async move { ling_mem::hosted::open(&dir, self_base).await })
        .await
        .map_err(|e| format!("opening the memory store: {e}"))?
        .map_err(|e| format!("opening the memory store: {e:#}"))?;
    Ok(Served {
        router: ling_mem::hosted::router(state.clone()),
        state,
        _lock: lock,
    })
}

/// This version's one copy of the store before it first opens it
/// (`ling_mem::hosted::backup_once`). A failure does not stop start-up; it
/// is logged as an error and `ling status` reports it until a start succeeds.
fn back_up_once(data_dir: &Path) {
    match ling_mem::hosted::backup_once(data_dir, VERSION) {
        Ok(Some(copy)) => tracing::info!("memory: store backed up to {}", copy.display()),
        Ok(None) => {}
        Err(e) => tracing::error!("memory: BACKUP BEFORE FIRST OPEN FAILED — {e:#}"),
    }
}

/// The engine's version: one store backup per version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The store's lock, waiting up to [`LOCK_WAIT`] for a holder to let go.
async fn acquire_lock(data_dir: PathBuf, port: u16) -> Result<StoreLock, String> {
    let started = std::time::Instant::now();
    loop {
        match StoreLock::acquire(&data_dir, Some(port)) {
            Ok(lock) => return Ok(lock),
            Err(e) if started.elapsed() >= LOCK_WAIT => return Err(format!("{e:#}")),
            Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
}

fn url_for(port: u16) -> String {
    format!("http://127.0.0.1:{port}{PREFIX}")
}

/// One request to the hosted memory's own routes as this machine (no HTTP
/// hop): the engine's settings and Allow / Deny call memory this way.
/// Answers memory's envelope, or a 503 envelope when memory is not served.
pub async fn local_call(
    method: axum::http::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (axum::http::StatusCode, serde_json::Value) {
    use axum::http::StatusCode;
    use tower::ServiceExt;
    let unavailable = |why: &str| {
        let body = serde_json::json!({ "ok": false, "error": why, "code": "MEMORY_UNAVAILABLE" });
        (StatusCode::SERVICE_UNAVAILABLE, body)
    };
    let Some(host) = HOST.get() else {
        return unavailable("memory is not hosted by this process");
    };
    let served = match &host.served {
        Ok(served) => served,
        Err(why) => return unavailable(why),
    };
    let bytes = body.map(|b| b.to_string()).unwrap_or_default();
    let mut req = match axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("host", format!("127.0.0.1:{}", host.port))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(bytes))
    {
        Ok(req) => req,
        Err(e) => return unavailable(&format!("bad memory path {path}: {e}")),
    };
    let me = std::net::SocketAddr::from(([127, 0, 0, 1], 0));
    req.extensions_mut().insert(axum::extract::ConnectInfo(me));
    let router = served.router.clone();
    let answer = host.runtime.spawn(async move {
        let response = match router.oneshot(req).await {
            Ok(response) => response,
            Err(never) => match never {},
        };
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 16 << 20)
            .await
            .map_err(|e| e.to_string())?;
        Ok::<_, String>((status, serde_json::from_slice(&bytes).unwrap_or_default()))
    });
    match answer.await {
        Ok(Ok(answer)) => answer,
        Ok(Err(e)) => unavailable(&format!("memory answered unreadably: {e}")),
        Err(e) => unavailable(&format!("memory request failed: {e}")),
    }
}

/// Note that approved machine `id` was just seen at `ip` asking for `path`
/// — its presence, as memory's device list shows it. Nothing when this
/// process hosts no open store.
pub fn note_machine(id: &str, ip: std::net::IpAddr, path: &str) {
    let Some(Ok(served)) = HOST.get().map(|h| h.served.as_ref()) else {
        return;
    };
    served
        .state
        .presence
        .record(id, ip, path, std::time::Instant::now());
}

/// Where an MCP client of this engine reaches memory's MCP server (`/mem/mcp`
/// under this base): the store this process hosts, on its own port. Empty
/// when this process hosts none (a CLI command, a unit test) — a call to it
/// then fails at once and never reaches another process. The engine's own
/// verb calls do not use it: they call memory in-process ([`post_verb`]).
pub fn url() -> String {
    HOST.get().map(|h| url_for(h.port)).unwrap_or_default()
}

/// The address of `verb`'s route on this engine — for the log line of a
/// call that is answered as that route answers it (no request is made).
pub fn route_of(verb: &str) -> String {
    format!("{}/api/memory/{verb}", url())
}

/// Write every curated row (no vectors) to `path` as NDJSON — the store
/// snapshot a memory mission takes before it runs. Runs on memory's runtime.
pub(crate) async fn export_to(path: &Path) -> anyhow::Result<()> {
    let host = HOST
        .get()
        .ok_or_else(|| anyhow::anyhow!("memory is not hosted by this process"))?;
    let served = host
        .served
        .as_ref()
        .map_err(|why| anyhow::anyhow!("{why}"))?;
    let store = served.state.store.clone();
    let path = path.to_path_buf();
    host.runtime
        .spawn(async move { ling_mem::hosted::export(&store, &path).await })
        .await?
}
