//! A memory server for tests: the hosted memory on a temp store, served on a
//! free loopback port — what the engine hosts, without the engine.

use std::net::SocketAddr;

/// Held by a test that connects servers on the process's one registry
/// (`registry()`): `connect_all` replaces its servers wholesale, so two such
/// tests at once would wipe each other's. Connect nothing at the end.
pub(crate) async fn global_registry() -> tokio::sync::MutexGuard<'static, ()> {
    static HELD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    HELD.lock().await
}

/// The MCP URL of a fresh memory on a temp store. Keep the dir alive.
pub(crate) async fn mcp_url() -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temp store");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let port = listener.local_addr().expect("its port").port();
    let state = ling_mem::hosted::open_quiet_fake(dir.path(), format!("http://127.0.0.1:{port}"))
        .await
        .expect("open the temp store");
    let app = ling_mem::hosted::router(state);
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });
    (format!("http://127.0.0.1:{port}/mcp"), dir)
}
