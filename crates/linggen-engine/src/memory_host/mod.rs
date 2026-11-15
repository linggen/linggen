//! Memory, hosted inside this process.
//!
//! The engine opens the memory store itself at start-up (`start`) and serves
//! memory's HTTP routes on its own port (`routes`). No second process: what
//! used to be the `ling-mem` daemon is the `ling_mem` library, run here on a
//! tokio runtime of its own, so a slow query or a store compaction keeps to
//! memory's threads and cannot stall the engine's (WebRTC, streaming, the UI).
//!
//! - The store is held under an exclusive lock (`<store>/.store.lock`) for the
//!   life of the process; a second opener — another engine, or `ling mem` with
//!   the engine down — is refused with the holder named.
//! - A failed open does not stop the engine: memory answers 503 with the
//!   reason, everything else serves.
//! - The embedding model loads on the first call that embeds, not at start.
//! - Before opening: an old `ling-mem` daemon still serving the store is
//!   stopped (`takeover`), and each engine version copies the store once
//!   (`memory/backups/takeover-<stamp>/`, marker `takeover-<version>.done`).
//!
//! The engine's own callers still reach memory over loopback HTTP and MCP
//! (`effective_url` points them at [`PREFIX`] on this port); calling the
//! library directly is a later step.

pub mod caller;
mod legacy_port;
mod routes;
mod standin;
mod takeover;

pub use routes::{is_open_path, routes};

use ling_mem::hosted::StoreLock;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

/// Where memory's own routes are mounted on the engine's port —
/// `/mem/api/memory/<verb>`, `/mem/api/health`, `/mem/mcp`, … Loopback only.
/// Not under `/api/`: the phone's WebRTC channel forwards `/api/*` to this
/// port as loopback, and the phone's door is the stamped `/api/memory/<verb>`.
pub const PREFIX: &str = ling_mem::replica::state::PRIMARY_PREFIX;

/// Memory's MCP server for hosts on this machine (Claude Code, Codex, …),
/// beside the engine's own `/mcp`. The same server is at `/mem/mcp`.
pub(crate) const MCP_PATH: &str = "/mcp/memory";

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
        Ok(s) => {
            tracing::info!(
                "memory: store open under {} — serving at {PREFIX} and {MCP_PATH}",
                data_dir.display()
            );
            keep_old_callers_working(&s.router);
        }
        Err(why) => tracing::warn!("memory: not serving — {why}"),
    }
    let _ = HOST.set(Host {
        port,
        runtime: runtime().handle().clone(),
        served,
    });
}

/// What callers of the old daemon still use: its address on this machine
/// (`legacy_port`) and its binary (`standin`).
fn keep_old_callers_working(router: &axum::Router) {
    let port = legacy_port::port();
    if let Some(port) = port {
        legacy_port::serve(runtime(), router.clone(), port);
    }
    standin::install(port.unwrap_or(legacy_port::DEFAULT_PORT));
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

/// The base URL of the memory this process hosts, once `start` has run.
pub(crate) fn hosted_url() -> Option<String> {
    HOST.get().map(|h| url_for(h.port))
}

/// The memory address the engine's own callers use. With memory hosted here,
/// any address on this machine — the old daemon's `127.0.0.1:9528` included —
/// means this process; another machine's address is kept as configured.
pub fn effective_url(configured: &str) -> String {
    match hosted_url() {
        Some(own) if is_loopback_url(configured) => own,
        _ => configured.trim_end_matches('/').to_string(),
    }
}

fn is_loopback_url(raw: &str) -> bool {
    let Ok(url) = url::Url::parse(raw.trim()) else {
        return false;
    };
    match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
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

#[cfg(test)]
mod tests {
    use super::is_loopback_url;

    #[test]
    fn addresses_on_this_machine_are_the_hosted_memory() {
        assert!(is_loopback_url("http://127.0.0.1:9528"));
        assert!(is_loopback_url("http://localhost:4100/"));
        assert!(is_loopback_url("http://[::1]:9528"));
        assert!(!is_loopback_url("http://192.168.1.5:9528"));
        assert!(!is_loopback_url("http://mem.lan:9528"));
        assert!(!is_loopback_url("not a url"));
    }
}
