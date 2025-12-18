use anyhow::Result;

mod analytics;
mod handlers;
mod internal_indexer;
mod job_manager;
mod memory;
mod server;

/// Entry point for the Linggen backend server binary (`linggen-server`).
/// This binary no longer exposes CLI UX; it only runs the HTTP API.
#[tokio::main]
async fn main() -> Result<()> {
    // Allow overriding port via env var or a simple `--port <n>` arg.
    let mut port = std::env::var("LINGGEN_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8787);

    // If provided, the server will automatically exit when the parent PID disappears.
    // This is used by the desktop app to prevent orphaned sidecar backends.
    let mut parent_pid: Option<u32> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--port" || arg == "-p" {
            if let Some(val) = args.next() {
                if let Ok(parsed) = val.parse() {
                    port = parsed;
                }
            }
        } else if arg == "--parent-pid" {
            if let Some(val) = args.next() {
                if let Ok(parsed) = val.parse() {
                    parent_pid = Some(parsed);
                }
            }
        }
    }

    server::start_server(port, parent_pid).await
}
