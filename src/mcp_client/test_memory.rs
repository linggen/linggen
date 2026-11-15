//! A memory server for tests: the hosted memory on a temp store, served on a
//! free loopback port — what the engine hosts, without the engine.

use std::net::SocketAddr;

/// The MCP URL of a fresh memory on a temp store. Keep the dir alive.
pub(crate) async fn mcp_url() -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temp store");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let port = listener.local_addr().expect("its port").port();
    let state = ling_mem::hosted::open(dir.path(), format!("http://127.0.0.1:{port}"))
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
