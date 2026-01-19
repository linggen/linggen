use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;

mod cli;
mod manifest;

use cli::{
    find_server_binary, handle_check, handle_doctor, handle_index, handle_install, handle_start,
    handle_status, handle_update, is_in_path, ApiClient,
};

#[derive(Parser)]
#[command(name = "linggen")]
#[command(about = "Linggen - AI-powered code intelligence CLI", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// API URL for CLI commands
    #[arg(
        long,
        env = "LINGGEN_API_URL",
        default_value = "http://127.0.0.1:8787",
        global = true
    )]
    api_url: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the Linggen server if needed and show status
    Start,

    /// Index a local directory
    Index {
        /// Path to the directory to index (defaults to current directory)
        path: Option<PathBuf>,

        /// Indexing mode: auto, full, or incremental
        #[arg(long, default_value = "auto")]
        mode: String,

        /// Override the default source name
        #[arg(long)]
        name: Option<String>,

        /// Include patterns (glob patterns)
        #[arg(long = "include")]
        include_patterns: Vec<String>,

        /// Exclude patterns (glob patterns)
        #[arg(long = "exclude")]
        exclude_patterns: Vec<String>,

        /// Wait for the indexing job to complete (default: true, use --no-wait to disable)
        #[arg(long, default_value = "true")]
        wait: bool,
    },

    /// Show system status and recent jobs
    Status {
        /// Number of recent jobs to show
        #[arg(long, default_value = "10")]
        limit: usize,
    },

    /// Start or ensure the Linggen backend server is running
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "8787")]
        port: u16,
    },

    /// Install Linggen components for this platform
    Install {
        /// Specific version to install (e.g., "0.2.1"). If not provided, installs latest.
        version: Option<String>,
    },

    /// Update Linggen components for this platform
    Update,

    /// Check for updates (CLI + runtime/app)
    #[command(alias = "version")]
    Check,

    /// Diagnose installation, backend connectivity, and update configuration
    Doctor,
}

/// Ensure the Linggen backend is running at the given API URL.
/// If it is not reachable, this will start it in the background
/// and wait until it responds (or time out with an error).
async fn ensure_backend_running(api_url: &str) -> Result<()> {
    let api_client = ApiClient::new(api_url.to_string());

    // Fast path: backend already running
    if api_client.get_status().await.is_ok() {
        println!("✅ Connected to existing backend at {}", api_url);
        return Ok(());
    }

    // Try to derive the port from the URL, fall back to default 8787
    let port = extract_port_from_url(api_url).unwrap_or(8787);

    // 1. Start backend first (headless) - this is fast
    start_backend_subprocess(port)?;

    // 2. On macOS, launch the desktop app (Tauri) in parallel.
    // Since backend is already starting, the app will connect to it.
    #[cfg(target_os = "macos")]
    {
        let _ = try_launch_desktop_app();
    }

    // 3. Poll until backend becomes available, with a timeout.
    // Use smaller intervals for a faster "feeling".
    let max_attempts = 40; // 40 * 250ms = 10 seconds
    for i in 0..max_attempts {
        if api_client.get_status().await.is_ok() {
            println!("✅ Backend is ready at {}", api_url);
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
        if i % 4 == 3 {
            // Every second, print a heartbeat if not ready
            println!("... waiting for backend to initialize ...");
        }
    }

    Err(anyhow::Error::msg(format!(
        "Timed out waiting for Linggen backend to start at {}",
        api_url
    )))
}

#[cfg(target_os = "macos")]
fn try_launch_desktop_app() -> Result<()> {
    use std::process::{Command, Stdio};

    eprintln!("🚀 Launching Linggen desktop app (Linggen.app)...");

    // Prefer app name lookup first (works if installed normally).
    let ok = Command::new("open")
        .args(["-a", "Linggen"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if ok {
        return Ok(());
    }

    // Fallback to explicit path.
    let ok = Command::new("open")
        .arg("/Applications/Linggen.app")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if ok {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "Failed to launch Linggen.app (is it installed in /Applications?)"
    ))
}

/// Start the Linggen backend as a separate background process.
/// This assumes a `linggen-server` binary is available on PATH or in standard locations.
fn start_backend_subprocess(port: u16) -> Result<()> {
    use std::fs::OpenOptions;
    use std::process::{Command, Stdio};

    // Get log directory
    let log_dir = dirs::data_local_dir()
        .ok_or_else(|| anyhow::anyhow!("Failed to determine local data directory"))?
        .join("Linggen");
    std::fs::create_dir_all(&log_dir)?;

    let log_file = log_dir.join("server.log");
    let log_handle = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)?;

    // Determine the server binary to run
    let server_bin = find_server_binary();

    // Spawn the server and do not wait for it.
    let spawn_result = Command::new(&server_bin)
        .arg("--port")
        .arg(port.to_string())
        .stdin(Stdio::null())
        .stdout(log_handle.try_clone()?)
        .stderr(log_handle)
        .spawn();

    match spawn_result {
        Ok(_) => {
            println!(
                "🚀 Starting Linggen backend in background on port {} (logs: {})",
                port,
                log_file.display()
            );
            Ok(())
        }
        Err(e) => {
            // If we failed to spawn, provide a more helpful message
            Err(anyhow::anyhow!(
                "Failed to start Linggen backend server (tried '{}'): {}\n\n\
                 Please ensure linggen-server is installed and in your PATH,\n\
                 or that Linggen.app is installed in /Applications.",
                server_bin,
                e
            ))
        }
    }
}

/// Best-effort extraction of a port number from an API URL string.
/// Examples:
/// - "http://127.0.0.1:8787" -> Some(8787)
/// - "http://localhost" -> None
fn extract_port_from_url(api_url: &str) -> Option<u16> {
    // Strip scheme if present
    let without_scheme = if let Some(pos) = api_url.find("://") {
        &api_url[pos + 3..]
    } else {
        api_url
    };

    // For hosts like "127.0.0.1:8787" or "[::1]:8787"
    if let Some((_, port_str)) = without_scheme.rsplit_once(':') {
        port_str.parse().ok()
    } else {
        None
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli_args = Cli::parse();

    match cli_args.command {
        // Default: just ensure backend running and print status
        None | Some(Commands::Start) => {
            ensure_backend_running(&cli_args.api_url).await?;
            let api_client = ApiClient::new(cli_args.api_url);
            handle_start(&api_client).await?;
        }

        Some(Commands::Serve { port }) => {
            let check_url = format!("http://127.0.0.1:{}", port);
            ensure_backend_running(&check_url).await?;
            println!(
                "✅ Linggen backend is running in background on {}",
                check_url
            );
            println!("   Use your system tools to stop the linggen-server process if needed.");
        }

        Some(Commands::Index {
            path,
            mode,
            name,
            include_patterns,
            exclude_patterns,
            wait,
        }) => {
            ensure_backend_running(&cli_args.api_url).await?;
            let api_client = ApiClient::new(cli_args.api_url);
            handle_index(
                &api_client,
                path,
                mode,
                name,
                include_patterns,
                exclude_patterns,
                wait,
            )
            .await?;
        }

        Some(Commands::Status { limit }) => {
            ensure_backend_running(&cli_args.api_url).await?;
            let api_client = ApiClient::new(cli_args.api_url);
            handle_status(&api_client, limit).await?;
        }

        Some(Commands::Install { version }) => {
            handle_install(version.as_deref()).await?;
        }

        Some(Commands::Update) => {
            handle_update().await?;
        }

        Some(Commands::Check) => {
            handle_check().await?;
        }

        Some(Commands::Doctor) => {
            handle_doctor(&cli_args.api_url).await?;
        }
    }

    Ok(())
}
